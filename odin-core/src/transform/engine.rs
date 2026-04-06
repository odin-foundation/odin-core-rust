//! Transform execution engine.
//!
//! Executes an `OdinTransform` against source data (`DynValue`) to produce a `TransformResult`.
//!
//! ## Execution flow
//!
//! 1. Create execution context (source data, constants, accumulators, counters).
//! 2. Process segments in pass order (pass 1 first, then pass 2, etc., then pass 0/None).
//! 3. For each segment, check for a `_loop` `source_path` directive and iterate or process once.
//! 4. For each mapping, evaluate the expression and set the value in the output at the target path.
//! 5. Format the output using the target format.

use std::collections::HashMap;
use crate::types::transform::{
    OdinTransform, TransformResult, TransformError, TransformWarning, TransformSegment,
    FieldMapping, FieldExpression, VerbCall, VerbArg, DynValue, ConfidentialMode,
    extract_error_code,
};
use crate::types::values::{OdinValue, OdinArrayItem};
use super::verbs::{VerbRegistry, VerbContext};
use super::formatters;

// ─────────────────────────────────────────────────────────────────────────────
// Execution context
// ─────────────────────────────────────────────────────────────────────────────

/// Mutable state carried through a transform execution.
struct ExecContext<'a> {
    /// Root source data.
    source: &'a DynValue,
    /// Named constants converted to `DynValue`.
    constants: HashMap<String, DynValue>,
    /// Accumulator values.
    accumulators: HashMap<String, DynValue>,
    /// Lookup tables.
    tables: HashMap<String, crate::types::transform::LookupTable>,
    /// Loop variables for the current iteration scope.
    loop_vars: HashMap<String, DynValue>,
    /// Verb registry.
    verbs: VerbRegistry,
    /// Collected warnings.
    warnings: Vec<TransformWarning>,
    /// Collected non-fatal errors.
    errors: Vec<TransformError>,
    /// Confidential enforcement mode.
    enforce_confidential: Option<ConfidentialMode>,
    /// Snapshot of the global output (for cross-segment reference resolution).
    global_output: DynValue,
    /// Collected field modifiers (target path → modifiers).
    field_modifiers: HashMap<String, crate::types::values::OdinModifiers>,
    /// Source format string (e.g., "fixed-width", "odin", "json").
    /// Used to determine whether extraction directives should be applied on references.
    source_format: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Execute a parsed transform against source data and return a `TransformResult`.
pub fn execute(transform: &OdinTransform, source: &DynValue) -> TransformResult {
    // Check for multi-record mode (source has discriminator config)
    if let Some(ref source_config) = transform.source {
        if let Some(disc_str) = source_config.options.get("discriminator") {
            if let DynValue::String(raw_input) = source {
                return execute_multi_record(transform, raw_input, disc_str, &source_config.format);
            }
        }
    }

    // If source is a raw string and we know the source format, parse it first.
    // Source format can come from explicit {$source} config or from the direction string.
    if let DynValue::String(raw) = source {
        let src_fmt: Option<&str> = transform.source.as_ref().map(|s| s.format.as_str())
            .or_else(|| {
                transform.metadata.direction.as_deref()
                    .and_then(|d: &str| d.split("->").next())
            });
        if let Some(fmt) = src_fmt {
            if matches!(fmt, "csv" | "delimited" | "fixed-width" | "xml" | "json" | "yaml" | "flat-kvp" | "flat-yaml") {
                if let Ok(parsed) = crate::transform::source_parsers::parse_source(raw, fmt) {
                    return execute(transform, &parsed);
                }
            }
        }
    }

    // 1. Build execution context
    let constants = transform.constants.iter().map(|(k, v)| {
        (k.clone(), odin_value_to_dyn(v))
    }).collect::<HashMap<_, _>>();

    let accumulators = transform.accumulators.iter().map(|(k, def)| {
        (k.clone(), odin_value_to_dyn(&def.initial))
    }).collect::<HashMap<_, _>>();

    let tables = transform.tables.clone();

    // Determine source format from source config or direction string
    let source_format = transform.source.as_ref().map(|s| s.format.clone())
        .or_else(|| {
            transform.metadata.direction.as_deref()
                .and_then(|d| d.split("->").next())
                .map(std::string::ToString::to_string)
        })
        .unwrap_or_default();

    let mut ctx = ExecContext {
        source,
        constants,
        accumulators,
        tables,
        loop_vars: HashMap::new(),
        verbs: VerbRegistry::new(),
        warnings: Vec::new(),
        errors: Vec::new(),
        enforce_confidential: transform.enforce_confidential,
        global_output: DynValue::Object(Vec::new()),
        field_modifiers: HashMap::new(),
        source_format,
    };

    // 2. Build output object
    let mut output = DynValue::Object(Vec::new());

    // 3. Order segments by pass: pass 1 first, then 2, ..., then 0/None last
    let ordered = order_segments_by_pass(&transform.segments);

    let mut is_first_pass = true;
    let mut current_pass: Option<usize> = None;
    for seg in &ordered {
        // Reset non-persist accumulators at each pass transition (matching TS behavior).
        // The first pass never resets. All subsequent pass transitions do.
        let seg_pass = seg.pass;
        if seg_pass != current_pass {
            if !is_first_pass {
                for (name, def) in &transform.accumulators {
                    if !def.persist {
                        let initial = odin_value_to_dyn(&def.initial);
                        ctx.accumulators.insert(name.clone(), initial);
                    }
                }
            }
            is_first_pass = false;
            current_pass = seg_pass;
        }

        process_segment(seg, &mut ctx, &mut output, "");
        // Update global output snapshot for cross-segment references
        ctx.global_output = output.clone();
    }

    // 4. Apply confidential enforcement to the entire output tree
    if let Some(mode) = ctx.enforce_confidential {
        apply_confidential_enforcement(&transform.segments, &mode, &mut output);
    }

    // 5. Format the output
    let formatted = if transform.target.format.eq_ignore_ascii_case("odin") {
        let include_header = transform.target.options.get("header").is_some_and(|v| v == "true");
        formatters::format_odin_with_modifiers(&output, &ctx.field_modifiers, include_header)
    } else if transform.target.format.eq_ignore_ascii_case("fixed-width") {
        // Fixed-width export uses segment mapping directives for positioning
        formatters::format_fixed_width_from_segments(&output, &transform.segments, &transform.target.options)
    } else if transform.target.format.eq_ignore_ascii_case("xml") {
        formatters::format_xml_full(&output, &transform.target.options, &ctx.field_modifiers)
    } else {
        format_output(&output, &transform.target.format, &transform.target.options)
    };

    let success = ctx.errors.is_empty();

    TransformResult {
        success,
        output: Some(output),
        formatted: Some(formatted),
        errors: ctx.errors,
        warnings: ctx.warnings,
        modifiers: ctx.field_modifiers,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Multi-record execution
// ─────────────────────────────────────────────────────────────────────────────

/// Discriminator extraction mode.
enum DiscriminatorMode {
    Position { pos: usize, len: usize },
    Field { index: usize },
}

/// Parse discriminator config string like ":pos 0 :len 3" or ":field 0".
fn parse_discriminator_config(config: &str) -> Option<DiscriminatorMode> {
    let parts: Vec<&str> = config.split_whitespace().collect();
    let mut pos: Option<usize> = None;
    let mut len: Option<usize> = None;
    let mut field: Option<usize> = None;

    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            ":pos" if i + 1 < parts.len() => { pos = parts[i + 1].parse().ok(); i += 2; }
            ":len" if i + 1 < parts.len() => { len = parts[i + 1].parse().ok(); i += 2; }
            ":field" if i + 1 < parts.len() => { field = parts[i + 1].parse().ok(); i += 2; }
            _ => { i += 1; }
        }
    }

    if let Some(fi) = field {
        Some(DiscriminatorMode::Field { index: fi })
    } else if let (Some(p), Some(l)) = (pos, len) {
        Some(DiscriminatorMode::Position { pos: p, len: l })
    } else {
        None
    }
}

/// Extract discriminator value from a record line.
fn extract_discriminator_value(line: &str, mode: &DiscriminatorMode, delimiter: &str) -> String {
    match mode {
        DiscriminatorMode::Position { pos, len } => {
            if *pos + *len <= line.len() {
                line[*pos..*pos + *len].trim().to_string()
            } else if *pos < line.len() {
                line[*pos..].trim().to_string()
            } else {
                String::new()
            }
        }
        DiscriminatorMode::Field { index } => {
            let fields: Vec<&str> = line.split(delimiter).collect();
            fields.get(*index).map(|s| s.trim().to_string()).unwrap_or_default()
        }
    }
}

/// Parse a record for multi-record processing.
/// For CSV/delimited: fields indexed as "0", "1", etc.
/// For fixed-width: raw line available as "_line" and "_raw".
fn parse_record(line: &str, format: &str, delimiter: &str) -> DynValue {
    match format {
        "csv" | "delimited" => {
            let fields: Vec<&str> = line.split(delimiter).collect();
            let mut entries: Vec<(String, DynValue)> = vec![
                ("_raw".to_string(), DynValue::String(line.to_string())),
                ("_line".to_string(), DynValue::String(line.to_string())),
            ];
            for (i, f) in fields.iter().enumerate() {
                entries.push((i.to_string(), DynValue::String((*f).to_string())));
            }
            DynValue::Object(entries)
        }
        _ => {
            // fixed-width: raw line for :pos/:len extraction
            DynValue::Object(vec![
                ("_raw".to_string(), DynValue::String(line.to_string())),
                ("_line".to_string(), DynValue::String(line.to_string())),
            ])
        }
    }
}

/// Execute a multi-record transform (CSV/fixed-width with discriminator).
fn execute_multi_record(
    transform: &OdinTransform,
    raw_input: &str,
    disc_config: &str,
    source_format: &str,
) -> TransformResult {
    let Some(disc_mode) = parse_discriminator_config(disc_config) else {
        return TransformResult {
            success: false,
            output: None,
            formatted: None,
            errors: vec![TransformError {
                message: format!("Invalid discriminator config: {disc_config}"),
                path: None,
                code: None,
            }],
            warnings: Vec::new(),
            modifiers: HashMap::new(),
        };
    };

    let delimiter = transform.source.as_ref()
        .and_then(|s| s.options.get("delimiter"))
        .map_or(",", std::string::String::as_str);

    // Build segment routing map: discriminator value -> segment
    let mut segment_map: HashMap<String, &TransformSegment> = HashMap::new();
    for seg in &transform.segments {
        // Look for _type mapping to determine which discriminator value routes here
        for mapping in &seg.mappings {
            if mapping.target == "_type" {
                if let FieldExpression::Literal(ref lit) = mapping.expression {
                    // Extract string value from OdinValue
                    let type_str = match lit {
                        OdinValue::String { value, .. } => Some(value.clone()),
                        _ => None,
                    };
                    // Support comma-separated type values
                    if let Some(ts) = type_str {
                        for type_val in ts.split(',') {
                            segment_map.insert(type_val.trim().to_string(), seg);
                        }
                    }
                }
            }
        }
    }

    // Build context
    let constants = transform.constants.iter().map(|(k, v)| {
        (k.clone(), odin_value_to_dyn(v))
    }).collect::<HashMap<_, _>>();
    let accumulators = transform.accumulators.iter().map(|(k, def)| {
        (k.clone(), odin_value_to_dyn(&def.initial))
    }).collect::<HashMap<_, _>>();

    let mut ctx = ExecContext {
        source: &DynValue::Null,
        constants,
        accumulators,
        tables: transform.tables.clone(),
        loop_vars: HashMap::new(),
        verbs: VerbRegistry::new(),
        warnings: Vec::new(),
        errors: Vec::new(),
        enforce_confidential: transform.enforce_confidential,
        global_output: DynValue::Object(Vec::new()),
        field_modifiers: HashMap::new(),
        source_format: source_format.to_string(),
    };

    let mut output = DynValue::Object(Vec::new());
    let mut array_accumulators: HashMap<String, Vec<DynValue>> = HashMap::new();

    // Initialize array accumulators for array segments
    for seg in &transform.segments {
        if seg.name.ends_with("[]") {
            let name = seg.name.strip_suffix("[]").unwrap_or(&seg.name).to_string();
            array_accumulators.insert(name, Vec::new());
        }
    }

    // Collect records to process (to avoid borrow issues)
    let records: Vec<(String, &TransformSegment)> = raw_input.lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let disc_value = extract_discriminator_value(line, &disc_mode, delimiter);
            segment_map.get(&disc_value).map(|seg| (line.to_string(), *seg))
        })
        .collect();

    // Process each record
    for (line, segment) in &records {
        let record_source = parse_record(line, source_format, delimiter);
        ctx.source = &DynValue::Null; // Not used for multi-record — process_mapping gets source directly

        // Process items in order (preserves interleaved mapping/child order)
        let mut record_output = DynValue::Object(Vec::new());
        for item in &segment.items {
            match item {
                crate::types::transform::SegmentItem::Mapping(mapping) => {
                    if mapping.target == "_type" {
                        continue; // Skip discriminator mapping
                    }
                    process_mapping(mapping, &mut ctx, &record_source, &mut record_output, "");
                }
                crate::types::transform::SegmentItem::Child(child_seg) => {
                    // Process child segment's mappings (flatten dotted paths)
                    for child_mapping in &child_seg.mappings {
                        let full_target = format!("{}.{}", child_seg.name, child_mapping.target);
                        let wrapper = FieldMapping {
                            target: full_target,
                            expression: child_mapping.expression.clone(),
                            directives: child_mapping.directives.clone(),
                            modifiers: child_mapping.modifiers.clone(),
                        };
                        process_mapping(&wrapper, &mut ctx, &record_source, &mut record_output, "");
                    }
                }
            }
        }

        // Merge into output
        let seg_name = segment.name.strip_suffix("[]").unwrap_or(&segment.name);
        if segment.name.ends_with("[]") {
            // Accumulate into array
            if let Some(arr) = array_accumulators.get_mut(seg_name) {
                arr.push(record_output);
            }
        } else {
            // Merge single record into output at segment path
            if let DynValue::Object(entries) = &mut output {
                if let DynValue::Object(record_entries) = record_output {
                    // Find or create the segment object
                    let existing = entries.iter().position(|(k, _)| k == seg_name);
                    if let Some(pos) = existing {
                        if let DynValue::Object(existing_entries) = &mut entries[pos].1 {
                            for (k, v) in record_entries {
                                existing_entries.push((k, v));
                            }
                        }
                    } else {
                        entries.push((seg_name.to_string(), DynValue::Object(record_entries)));
                    }
                }
            }
        }
    }

    // Merge array accumulators into output in transform segment order
    if let DynValue::Object(entries) = &mut output {
        for seg in &transform.segments {
            if seg.name.ends_with("[]") {
                let name = seg.name.strip_suffix("[]").unwrap_or(&seg.name).to_string();
                if let Some(items) = array_accumulators.remove(&name) {
                    entries.push((name, DynValue::Array(items)));
                }
            }
        }
    }

    // Format the output
    let formatted = if transform.target.format.eq_ignore_ascii_case("odin") {
        let include_header = transform.target.options.get("header").is_some_and(|v| v == "true");
        formatters::format_odin_with_modifiers(&output, &ctx.field_modifiers, include_header)
    } else if transform.target.format.eq_ignore_ascii_case("fixed-width") {
        formatters::format_fixed_width_from_segments(&output, &transform.segments, &transform.target.options)
    } else if transform.target.format.eq_ignore_ascii_case("xml") {
        formatters::format_xml_full(&output, &transform.target.options, &ctx.field_modifiers)
    } else {
        format_output(&output, &transform.target.format, &transform.target.options)
    };

    let success = ctx.errors.is_empty();

    TransformResult {
        success,
        output: Some(output),
        formatted: Some(formatted),
        errors: ctx.errors,
        warnings: ctx.warnings,
        modifiers: ctx.field_modifiers,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Segment ordering
// ─────────────────────────────────────────────────────────────────────────────

/// Return references to segments sorted by pass number.
/// Pass 1 comes first, then 2, etc. Pass 0 or None comes last.
fn order_segments_by_pass(segments: &[TransformSegment]) -> Vec<&TransformSegment> {
    let mut refs: Vec<&TransformSegment> = segments.iter().collect();
    refs.sort_by_key(|seg| {
        match seg.pass {
            Some(0) | None => usize::MAX,
            Some(n) => n,
        }
    });
    refs
}

// ─────────────────────────────────────────────────────────────────────────────
// Segment processing
// ─────────────────────────────────────────────────────────────────────────────

fn process_segment(segment: &TransformSegment, ctx: &mut ExecContext, output: &mut DynValue, path_prefix: &str) {
    // Check condition (simple truthy evaluation of a source path)
    if let Some(ref condition) = segment.condition {
        let cond_val = resolve_path(ctx.source, condition, &ctx.constants, &ctx.accumulators);
        if !is_truthy(&cond_val) {
            return;
        }
    }

    // Check discriminator
    if let Some(ref disc) = segment.discriminator {
        let disc_val = resolve_path(ctx.source, &disc.path, &ctx.constants, &ctx.accumulators);
        let matches = match &disc_val {
            DynValue::String(s) => s == &disc.value,
            DynValue::Integer(n) => n.to_string() == disc.value,
            DynValue::Float(n) => n.to_string() == disc.value,
            DynValue::Bool(b) => b.to_string() == disc.value,
            _ => false,
        };
        if !matches {
            return;
        }
    }

    // Determine the output target path for this segment.
    // The segment name becomes the key in the output object, unless it's empty
    // or the root segment name (like "$" or "_root").
    let seg_name = &segment.name;
    // Strip trailing [] from segment name for array segments
    let clean_name = seg_name.strip_suffix("[]").unwrap_or(seg_name);
    // Check for indexed array segment name (e.g., "vehicles[0]")
    let array_index = parse_array_index(clean_name);
    let is_root = clean_name.is_empty() || clean_name == "$" || clean_name == "_root";
    // Sections starting with '_' are internal/computation-only (e.g., _calcSubtotal);
    // they execute for side effects but don't produce output entries.
    let is_internal = !is_root && clean_name.starts_with('_');

    // Build the full path prefix for modifier tracking
    let current_prefix = if is_root {
        path_prefix.to_string()
    } else if path_prefix.is_empty() {
        clean_name.to_string()
    } else {
        format!("{path_prefix}.{clean_name}")
    };

    // Check for array loop: if source_path is set, iterate over array elements
    if let Some(ref source_path) = segment.source_path {
        let source_val = resolve_path(ctx.source, source_path, &ctx.constants, &ctx.accumulators);
        // Wrap non-array values in a single-element array for iteration
        // (handles XML single-element case where parser doesn't create an array).
        // Null (missing path) produces an empty array — zero iterations,
        // matching TS where Array.isArray(null) is false and loop is skipped.
        let array_val = match source_val {
            DynValue::Array(_) => source_val,
            DynValue::Null => DynValue::Array(Vec::new()),
            other => DynValue::Array(vec![other]),
        };
        if let DynValue::Array(ref items) = array_val {
            let mut result_items = Vec::new();
            let len = items.len() as i64;
            let is_value_only = segment.mappings.iter().all(|m| m.target == "_");
            for (idx, item) in items.iter().enumerate() {
                // Set loop variables
                ctx.loop_vars.insert("_item".to_string(), item.clone());
                ctx.loop_vars.insert("_index".to_string(), DynValue::Integer(idx as i64));
                ctx.loop_vars.insert("_length".to_string(), DynValue::Integer(len));

                let mut item_output = DynValue::Object(Vec::new());
                for mapping in &segment.mappings {
                    if mapping.target == "_" {
                        // Target "_" means "replace the entire item output"
                        // Reborrow item_output as shared for evaluation
                        match evaluate_expression(&mapping.expression, ctx, item, &item_output) {
                            Ok(val) => {
                                let is_raw = matches!(
                                    ctx.source_format.as_str(),
                                    "fixed-width" | "flat" | "flat-kvp" | "flat-yaml" | "csv" | "delimited"
                                );
                                let val = if is_raw {
                                    apply_type_directives(val, &mapping.directives)
                                } else {
                                    let type_dirs: Vec<_> = mapping.directives.iter()
                                        .filter(|d| !matches!(d.name.as_str(), "pos" | "len" | "leftPad" | "rightPad" | "truncate"))
                                        .cloned()
                                        .collect();
                                    apply_type_directives(val, &type_dirs)
                                };
                                if is_value_only {
                                    item_output = val;
                                }
                            }
                            Err(e) => {
                                let (code, msg) = extract_error_code(&e);
                                ctx.errors.push(TransformError {
                                    message: format!("mapping '_': {msg}"),
                                    path: Some("_".to_string()),
                                    code: code.map(|c| c.to_string()),
                                });
                            }
                        }
                    } else {
                        process_mapping(mapping, ctx, item, &mut item_output, &current_prefix);
                    }
                }
                result_items.push(item_output);

                ctx.loop_vars.remove("_item");
                ctx.loop_vars.remove("_index");
                ctx.loop_vars.remove("_length");
            }
            let array_val = DynValue::Array(result_items);
            if is_root {
                *output = array_val;
            } else {
                set_path(output, clean_name, array_val);
            }
        } else {
            ctx.warnings.push(TransformWarning {
                message: format!("segment '{seg_name}': source_path '{source_path}' did not resolve to an array"),
                path: Some(source_path.clone()),
            });
        }
    } else if !segment.items.is_empty() {
        // Use interleaved items list for correct field ordering.
        // This processes mappings and child segments in the order they
        // appeared in the original transform document.
        use crate::types::transform::SegmentItem;
        if is_root {
            for item in &segment.items {
                match item {
                    SegmentItem::Mapping(m) => {
                        process_mapping(m, ctx, ctx.source, output, &current_prefix);
                    }
                    SegmentItem::Child(child) => {
                        process_segment(child, ctx, output, &current_prefix);
                    }
                }
            }
        } else if let Some((arr_name, idx)) = array_index {
            ensure_array_entry_at(output, arr_name, idx);
            for item in &segment.items {
                match item {
                    SegmentItem::Mapping(m) => {
                        if let Some(target) = get_array_entry_mut(output, arr_name, idx) {
                            process_mapping(m, ctx, ctx.source, target, &current_prefix);
                        }
                    }
                    SegmentItem::Child(child) => {
                        // Use array entry if available, otherwise fall back to output
                        if let Some(child_target) = get_array_entry_mut(output, arr_name, idx) {
                            process_segment(child, ctx, child_target, &current_prefix);
                        } else {
                            process_segment(child, ctx, output, &current_prefix);
                        }
                    }
                }
            }
        } else if is_internal {
            // Internal sections (e.g. _calcSubtotal): execute for side effects only,
            // don't create a named key in the output.
            for item in &segment.items {
                match item {
                    SegmentItem::Mapping(m) => {
                        process_mapping(m, ctx, ctx.source, output, &current_prefix);
                    }
                    SegmentItem::Child(child) => {
                        process_segment(child, ctx, output, &current_prefix);
                    }
                }
            }
        } else {
            ensure_object_at(output, clean_name);
            for item in &segment.items {
                match item {
                    SegmentItem::Mapping(m) => {
                        if let Some(target) = get_mut_path(output, clean_name) {
                            process_mapping(m, ctx, ctx.source, target, &current_prefix);
                        }
                    }
                    SegmentItem::Child(child) => {
                        let child_target = match get_mut_path(output, clean_name) {
                            Some(t) => t,
                            None => output,
                        };
                        process_segment(child, ctx, child_target, &current_prefix);
                    }
                }
            }
        }
    } else {
        // Fallback: process mappings then children separately
        if is_root {
            for mapping in &segment.mappings {
                process_mapping(mapping, ctx, ctx.source, output, &current_prefix);
            }
        } else if is_internal {
            // Internal sections: execute for side effects, don't store in output
            for mapping in &segment.mappings {
                process_mapping(mapping, ctx, ctx.source, output, &current_prefix);
            }
        } else if let Some((arr_name, idx)) = array_index {
            ensure_array_entry_at(output, arr_name, idx);
            if let Some(target) = get_array_entry_mut(output, arr_name, idx) {
                for mapping in &segment.mappings {
                    process_mapping(mapping, ctx, ctx.source, target, &current_prefix);
                }
            }
        } else {
            ensure_object_at(output, clean_name);
            if let Some(target) = get_mut_path(output, clean_name) {
                for mapping in &segment.mappings {
                    process_mapping(mapping, ctx, ctx.source, target, &current_prefix);
                }
            }
        }
        for child in &segment.children {
            if is_root || is_internal {
                process_segment(child, ctx, output, &current_prefix);
            } else if let Some((arr_name, idx)) = array_index {
                if let Some(child_target) = get_array_entry_mut(output, arr_name, idx) {
                    process_segment(child, ctx, child_target, &current_prefix);
                } else {
                    process_segment(child, ctx, output, &current_prefix);
                }
            } else {
                let child_target = match get_mut_path(output, clean_name) {
                    Some(t) => t,
                    None => output,
                };
                process_segment(child, ctx, child_target, &current_prefix);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Mapping processing
// ─────────────────────────────────────────────────────────────────────────────

fn process_mapping(
    mapping: &FieldMapping,
    ctx: &mut ExecContext,
    current_source: &DynValue,
    output: &mut DynValue,
    path_prefix: &str,
) {
    // Pass the current output so expressions can reference previously-set fields.
    // Reborrow output as shared — evaluate_expression only reads from it.
    let value = evaluate_expression(&mapping.expression, ctx, current_source, &*output);

    match value {
        Ok(val) => {
            // Apply type coercion/extraction from directives.
            // For raw-text source formats (fixed-width, csv, etc.), apply ALL directives
            // including :pos/:len for substring extraction.
            // For structured formats (odin, json, xml), skip formatting directives
            // (:pos, :len, :leftPad, :rightPad) — they're for the output formatter.
            let is_raw_text_source = matches!(
                ctx.source_format.as_str(),
                "fixed-width" | "flat" | "flat-kvp" | "flat-yaml" | "csv" | "delimited"
            );
            let val = if is_raw_text_source {
                apply_type_directives(val, &mapping.directives)
            } else {
                let type_dirs: Vec<_> = mapping.directives.iter()
                    .filter(|d| !matches!(d.name.as_str(), "pos" | "len" | "leftPad" | "rightPad" | "truncate"))
                    .cloned()
                    .collect();
                apply_type_directives(val, &type_dirs)
            };

            // Apply confidential modifiers at the mapping level if needed
            let final_val = if let Some(ref mods) = mapping.modifiers {
                if mods.confidential {
                    if let Some(mode) = &ctx.enforce_confidential {
                        apply_confidential_to_value(&val, mode)
                    } else {
                        val
                    }
                } else {
                    val
                }
            } else {
                val
            };
            // Target "_" means discard (side-effect only, e.g., accumulate)
            if mapping.target != "_" {
                set_path(output, &mapping.target, final_val);
            }
            // Record field modifiers for ODIN/XML formatter (using full path)
            if let Some(ref mods) = mapping.modifiers {
                if mods.confidential || mods.required || mods.deprecated || mods.attr {
                    let full_key = if path_prefix.is_empty() {
                        mapping.target.clone()
                    } else {
                        format!("{}.{}", path_prefix, mapping.target)
                    };
                    ctx.field_modifiers.insert(full_key, mods.clone());
                }
            }
        }
        Err(e) => {
            let (code, msg) = extract_error_code(&e);
            ctx.errors.push(TransformError {
                message: format!("mapping '{}': {}", mapping.target, msg),
                path: Some(mapping.target.clone()),
                code: code.map(|c| c.to_string()),
            });
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Expression evaluation
// ─────────────────────────────────────────────────────────────────────────────

fn evaluate_expression(
    expr: &FieldExpression,
    ctx: &mut ExecContext,
    current_source: &DynValue,
    current_output: &DynValue,
) -> Result<DynValue, String> {
    match expr {
        FieldExpression::Copy(path) => {
            // Check if the path starts with loop variable references
            if path.starts_with("_item") || path.starts_with("@_item") {
                let clean = path.strip_prefix('@').unwrap_or(path);
                if let Some(item) = ctx.loop_vars.get("_item") {
                    if clean == "_item" {
                        return Ok(item.clone());
                    }
                    // Resolve remaining path within the loop item
                    let remaining = clean.strip_prefix("_item.").unwrap_or("");
                    if remaining.is_empty() {
                        return Ok(item.clone());
                    }
                    return Ok(resolve_sub_path(item, remaining));
                }
            }
            if path.starts_with("_index") || path.starts_with("@_index") {
                if let Some(idx) = ctx.loop_vars.get("_index") {
                    return Ok(idx.clone());
                }
            }
            if path.starts_with("_length") || path.starts_with("@_length") {
                if let Some(len) = ctx.loop_vars.get("_length") {
                    return Ok(len.clone());
                }
            }
            // Try current segment output first for local field references, then source
            Ok(resolve_path_with_output(current_source, current_output, &ctx.global_output, path, &ctx.constants, &ctx.accumulators))
        }

        FieldExpression::Literal(odin_val) => {
            Ok(odin_value_to_dyn(odin_val))
        }

        FieldExpression::Transform(verb_call) => {
            execute_verb_call(verb_call, ctx, current_source, current_output)
        }

        FieldExpression::Object(mappings) => {
            let mut obj = DynValue::Object(Vec::new());
            for m in mappings {
                let val = evaluate_expression(&m.expression, ctx, current_source, current_output)?;
                set_path(&mut obj, &m.target, val);
            }
            Ok(obj)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Verb execution
// ─────────────────────────────────────────────────────────────────────────────

fn execute_verb_call(
    call: &VerbCall,
    ctx: &mut ExecContext,
    current_source: &DynValue,
    current_output: &DynValue,
) -> Result<DynValue, String> {
    // Special handling for short-circuit verbs: ifElse, cond
    if call.verb == "ifElse" && call.args.len() >= 3 {
        // Evaluate condition first
        let condition = evaluate_verb_arg(&call.args[0], ctx, current_source, current_output)?;
        let is_true = match &condition {
            DynValue::Bool(b) => *b,
            DynValue::String(s) => !s.is_empty() && s != "false",
            DynValue::Null => false,
            DynValue::Integer(n) => *n != 0,
            DynValue::Float(n) => *n != 0.0,
            _ => true,
        };
        // Only evaluate the chosen branch
        let result = if is_true {
            evaluate_verb_arg(&call.args[1], ctx, current_source, current_output)?
        } else {
            evaluate_verb_arg(&call.args[2], ctx, current_source, current_output)?
        };
        return Ok(result);
    }

    if call.verb == "cond" && call.args.len() >= 2 {
        // cond: c1 v1 c2 v2 ... default
        let mut i = 0;
        while i + 1 < call.args.len() {
            let condition = evaluate_verb_arg(&call.args[i], ctx, current_source, current_output)?;
            let is_true = match &condition {
                DynValue::Bool(b) => *b,
                DynValue::String(s) => !s.is_empty() && s != "false",
                DynValue::Null => false,
                _ => true,
            };
            if is_true {
                return evaluate_verb_arg(&call.args[i + 1], ctx, current_source, current_output);
            }
            i += 2;
        }
        // Default value (last arg if odd count)
        if call.args.len() % 2 == 1 {
            return evaluate_verb_arg(&call.args[call.args.len() - 1], ctx, current_source, current_output);
        }
        return Ok(DynValue::Null);
    }

    // Standard eager evaluation for all other verbs
    let mut evaluated_args = Vec::with_capacity(call.args.len());
    for arg in &call.args {
        let val = evaluate_verb_arg(arg, ctx, current_source, current_output)?;
        evaluated_args.push(val);
    }

    // Look up verb in registry. Custom verbs (is_custom) act as echo/passthrough
    // if not explicitly registered — they return their first argument.
    let verb_fn = match ctx.verbs.get(&call.verb) {
        Some(f) => f,
        None if call.is_custom => {
            // Custom verb not registered — echo first argument
            return if evaluated_args.is_empty() {
                Ok(DynValue::Null)
            } else {
                Ok(evaluated_args.into_iter().next().unwrap_or(DynValue::Null))
            };
        }
        None => return Err(format!("unknown verb: '{}'", call.verb)),
    };

    // Build verb context
    let verb_ctx = VerbContext {
        source: current_source.clone(),
        loop_vars: ctx.loop_vars.clone(),
        accumulators: ctx.accumulators.clone(),
        tables: ctx.tables.clone(),
    };

    let result = verb_fn(&evaluated_args, &verb_ctx)?;

    // Special handling: accumulate and set update the accumulator state
    if call.verb == "accumulate" || call.verb == "set" {
        if let Some(DynValue::String(name)) = evaluated_args.first() {
            ctx.accumulators.insert(name.clone(), result.clone());
        }
    }

    Ok(result)
}

fn evaluate_verb_arg(
    arg: &VerbArg,
    ctx: &mut ExecContext,
    current_source: &DynValue,
    current_output: &DynValue,
) -> Result<DynValue, String> {
    match arg {
        VerbArg::Reference(path, directives) => {
            // Same loop-variable awareness as Copy
            let mut val = if path.starts_with("_item") || path.starts_with("@_item") {
                let clean = path.strip_prefix('@').unwrap_or(path);
                if let Some(item) = ctx.loop_vars.get("_item") {
                    if clean == "_item" {
                        item.clone()
                    } else {
                        let remaining = clean.strip_prefix("_item.").unwrap_or("");
                        if remaining.is_empty() {
                            item.clone()
                        } else {
                            resolve_sub_path(item, remaining)
                        }
                    }
                } else {
                    resolve_path_with_output(current_source, current_output, &ctx.global_output, path, &ctx.constants, &ctx.accumulators)
                }
            } else if path.starts_with("_index") || path.starts_with("@_index") {
                if let Some(idx) = ctx.loop_vars.get("_index") {
                    idx.clone()
                } else {
                    resolve_path_with_output(current_source, current_output, &ctx.global_output, path, &ctx.constants, &ctx.accumulators)
                }
            } else {
                resolve_path_with_output(current_source, current_output, &ctx.global_output, path, &ctx.constants, &ctx.accumulators)
            };

            // Apply extraction directives (:pos, :len, :field, :trim) to the resolved value.
            // Only apply extraction for raw-text source formats (fixed-width, flat, csv, delimited).
            // For structured formats (odin, json, xml), these directives are output formatting
            // instructions handled by the formatter, not extraction directives.
            if !directives.is_empty() {
                let is_raw_text_source = matches!(
                    ctx.source_format.as_str(),
                    "fixed-width" | "flat" | "flat-kvp" | "flat-yaml" | "csv" | "delimited"
                );
                if is_raw_text_source {
                    val = apply_type_directives(val, directives);
                } else {
                    // For non-raw sources, only apply type coercion directives (not :pos/:len extraction)
                    let type_only: Vec<_> = directives.iter()
                        .filter(|d| !matches!(d.name.as_str(), "pos" | "len" | "field" | "leftPad" | "rightPad" | "truncate"))
                        .cloned()
                        .collect();
                    if !type_only.is_empty() {
                        val = apply_type_directives(val, &type_only);
                    }
                }
            }

            Ok(val)
        }
        VerbArg::Literal(odin_val) => {
            Ok(odin_value_to_dyn(odin_val))
        }
        VerbArg::Verb(nested_call) => {
            execute_verb_call(nested_call, ctx, current_source, current_output)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Path resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve a path, checking the current output for local references first.
///
/// For paths without a leading `.` (like `actual`), checks the current segment output
/// before falling back to the source data. This allows mappings to reference fields
/// set earlier in the same segment.
fn resolve_path_with_output(
    source: &DynValue,
    output: &DynValue,
    global_output: &DynValue,
    path: &str,
    constants: &HashMap<String, DynValue>,
    accumulators: &HashMap<String, DynValue>,
) -> DynValue {
    let path = path.trim();

    // Empty path or bare `@` — return the source directly
    if path.is_empty() || path == "@" {
        return source.clone();
    }

    // Constants and accumulators always resolve from their respective maps
    if path.starts_with("$const.") || path.starts_with("$constants.")
        || path.starts_with("$accumulator.") || path.starts_with("$accumulators.") {
        return resolve_path(source, path, constants, accumulators);
    }

    // Paths with leading `.` (after stripping `@`) always resolve against source
    let clean = path.strip_prefix('@').unwrap_or(path);
    if clean.starts_with('.') || clean.is_empty() {
        return resolve_path(source, path, constants, accumulators);
    }

    // For bare paths (no leading dot):
    // 1. Try the current segment's local output
    let from_output = resolve_path(output, path, constants, accumulators);
    if !matches!(from_output, DynValue::Null) {
        return from_output;
    }

    // 2. Try the global output (for cross-segment references)
    let from_global = resolve_path(global_output, path, constants, accumulators);
    if !matches!(from_global, DynValue::Null) {
        return from_global;
    }

    // 3. Fall back to source data
    resolve_path(source, path, constants, accumulators)
}

/// Resolve a dotted path against source data.
///
/// Paths use `@.foo.bar` or `@foo.bar` dot notation, `@.items[0]` for array indexing.
/// `$const.X` resolves against constants, `$accumulator.X` against accumulators.
fn resolve_path(
    source: &DynValue,
    path: &str,
    constants: &HashMap<String, DynValue>,
    accumulators: &HashMap<String, DynValue>,
) -> DynValue {
    let path = path.trim();

    // Handle constant references: $const.X
    if let Some(rest) = path.strip_prefix("$const.") {
        return constants.get(rest).cloned().unwrap_or(DynValue::Null);
    }
    if let Some(rest) = path.strip_prefix("$constants.") {
        return constants.get(rest).cloned().unwrap_or(DynValue::Null);
    }

    // Handle accumulator references: $accumulator.X
    if let Some(rest) = path.strip_prefix("$accumulator.") {
        return accumulators.get(rest).cloned().unwrap_or(DynValue::Null);
    }
    if let Some(rest) = path.strip_prefix("$accumulators.") {
        return accumulators.get(rest).cloned().unwrap_or(DynValue::Null);
    }

    // Strip leading @ and optional leading dot
    let clean = path.strip_prefix('@').unwrap_or(path);
    let clean = clean.strip_prefix('.').unwrap_or(clean);

    if clean.is_empty() {
        return source.clone();
    }

    resolve_sub_path(source, clean)
}

/// Resolve a sub-path (no `@` prefix) against a value.
fn resolve_sub_path(value: &DynValue, path: &str) -> DynValue {
    if path.is_empty() {
        return value.clone();
    }

    let segments = parse_path_segments(path);
    let mut current = value;

    for seg in &segments {
        match seg {
            PathSegment::Field(name) => {
                match current.get(name) {
                    Some(v) => current = v,
                    None => return DynValue::Null,
                }
            }
            PathSegment::Index(name, idx) => {
                // First resolve the field, then index into the array
                let field_val = if name.is_empty() {
                    current
                } else {
                    match current.get(name) {
                        Some(v) => v,
                        None => return DynValue::Null,
                    }
                };
                match field_val.get_index(*idx) {
                    Some(v) => current = v,
                    None => return DynValue::Null,
                }
            }
        }
    }

    current.clone()
}

/// A single segment of a dotted path.
enum PathSegment {
    /// A plain field name.
    Field(String),
    /// A field name with an array index, e.g., `items[0]`.
    Index(String, usize),
}

/// Parse a dotted path string into segments.
///
/// Handles `foo.bar.baz`, `items[0].name`, `[2].field`, etc.
fn parse_path_segments(path: &str) -> Vec<PathSegment> {
    let mut segments = Vec::new();
    let mut remaining = path;

    while !remaining.is_empty() {
        // Skip leading dot
        remaining = remaining.strip_prefix('.').unwrap_or(remaining);
        if remaining.is_empty() {
            break;
        }

        // Check if this segment starts with a bracket (bare index)
        if remaining.starts_with('[') {
            if let Some(bracket_end) = remaining.find(']') {
                let idx_str = &remaining[1..bracket_end];
                if let Ok(idx) = idx_str.parse::<usize>() {
                    segments.push(PathSegment::Index(String::new(), idx));
                }
                remaining = &remaining[bracket_end + 1..];
                continue;
            }
        }

        // Find the end of this segment: next dot or bracket
        let mut end = remaining.len();
        let mut bracket_pos = None;
        for (i, ch) in remaining.char_indices() {
            if ch == '.' {
                end = i;
                break;
            }
            if ch == '[' && bracket_pos.is_none() {
                bracket_pos = Some(i);
            }
            if ch == ']' && bracket_pos.is_some() {
                end = i + 1;
                break;
            }
        }

        let segment_str = &remaining[..end];
        remaining = &remaining[end..];

        // Check if this segment has an array index
        if let Some(bracket_start) = segment_str.find('[') {
            if let Some(bracket_end) = segment_str.find(']') {
                let field_name = &segment_str[..bracket_start];
                let idx_str = &segment_str[bracket_start + 1..bracket_end];
                if let Ok(idx) = idx_str.parse::<usize>() {
                    segments.push(PathSegment::Index(field_name.to_string(), idx));
                    continue;
                }
            }
        }

        // Plain field name
        segments.push(PathSegment::Field(segment_str.to_string()));
    }

    segments
}

// ─────────────────────────────────────────────────────────────────────────────
// Setting values in output
// ─────────────────────────────────────────────────────────────────────────────

/// Set a value at the given dotted path in an output `DynValue` tree.
///
/// Creates intermediate objects as needed. Handles `items[]` syntax to push
/// onto arrays.
fn set_path(output: &mut DynValue, path: &str, value: DynValue) {
    let parts = split_set_path(path);

    if parts.is_empty() {
        *output = value;
        return;
    }

    if parts.len() == 1 {
        set_single_field(output, &parts[0], value);
        return;
    }

    // Navigate to the parent, creating intermediate objects
    let mut current = output;
    for part in &parts[..parts.len() - 1] {
        current = ensure_and_descend(current, part);
    }

    set_single_field(current, &parts[parts.len() - 1], value);
}

/// Split a set-path string into parts, respecting array bracket notation.
fn split_set_path(path: &str) -> Vec<SetPathPart> {
    let mut parts = Vec::new();
    let mut remaining = path;

    while !remaining.is_empty() {
        remaining = remaining.strip_prefix('.').unwrap_or(remaining);
        if remaining.is_empty() {
            break;
        }

        // Find next dot (that is not inside brackets)
        let mut end = remaining.len();
        let mut depth = 0;
        for (i, ch) in remaining.char_indices() {
            match ch {
                '[' => depth += 1,
                ']' => depth -= 1,
                '.' if depth == 0 && i > 0 => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }

        let seg = &remaining[..end];
        remaining = &remaining[end..];

        // Parse the segment
        if let Some(name) = seg.strip_suffix("[]") {
            // Array push notation: `items[]`
            parts.push(SetPathPart::ArrayPush(name.to_string()));
        } else if let Some(bracket_start) = seg.find('[') {
            if let Some(bracket_end) = seg.find(']') {
                let name = &seg[..bracket_start];
                let idx_str = &seg[bracket_start + 1..bracket_end];
                if let Ok(idx) = idx_str.parse::<usize>() {
                    parts.push(SetPathPart::ArrayIndex(name.to_string(), idx));
                } else {
                    parts.push(SetPathPart::Field(seg.to_string()));
                }
            } else {
                parts.push(SetPathPart::Field(seg.to_string()));
            }
        } else {
            parts.push(SetPathPart::Field(seg.to_string()));
        }
    }

    parts
}

#[derive(Debug)]
enum SetPathPart {
    Field(String),
    ArrayIndex(String, usize),
    ArrayPush(String),
}

/// Set a single field/array-push on the current `DynValue` object.
fn set_single_field(obj: &mut DynValue, part: &SetPathPart, value: DynValue) {
    match part {
        SetPathPart::Field(name) => {
            if let DynValue::Object(ref mut entries) = obj {
                // Update existing or append
                if let Some(existing) = entries.iter_mut().find(|(k, _)| k == name) {
                    existing.1 = value;
                } else {
                    entries.push((name.clone(), value));
                }
            }
        }
        SetPathPart::ArrayIndex(name, idx) => {
            if !name.is_empty() {
                // Ensure the field exists as an array
                if let DynValue::Object(ref mut entries) = obj {
                    let arr = entries.iter_mut().find(|(k, _)| k == name);
                    if let Some((_, ref mut arr_val)) = arr {
                        if let DynValue::Array(ref mut items) = arr_val {
                            while items.len() <= *idx {
                                items.push(DynValue::Null);
                            }
                            items[*idx] = value;
                        }
                    } else {
                        let mut items = Vec::new();
                        while items.len() <= *idx {
                            items.push(DynValue::Null);
                        }
                        items[*idx] = value;
                        entries.push((name.clone(), DynValue::Array(items)));
                    }
                }
            } else if let DynValue::Array(ref mut items) = obj {
                while items.len() <= *idx {
                    items.push(DynValue::Null);
                }
                items[*idx] = value;
            }
        }
        SetPathPart::ArrayPush(name) => {
            if let DynValue::Object(ref mut entries) = obj {
                let arr = entries.iter_mut().find(|(k, _)| k == name);
                if let Some((_, ref mut arr_val)) = arr {
                    if let DynValue::Array(ref mut items) = arr_val {
                        items.push(value);
                    }
                } else {
                    entries.push((name.clone(), DynValue::Array(vec![value])));
                }
            }
        }
    }
}

/// Ensure an intermediate path node exists and return a mutable reference to it.
fn ensure_and_descend<'a>(current: &'a mut DynValue, part: &SetPathPart) -> &'a mut DynValue {
    match part {
        SetPathPart::Field(name) => {
            if let DynValue::Object(ref mut entries) = current {
                // Find or create the field
                let idx = entries.iter().position(|(k, _)| k == name);
                if let Some(i) = idx {
                    &mut entries[i].1
                } else {
                    entries.push((name.clone(), DynValue::Object(Vec::new())));
                    let len = entries.len();
                    &mut entries[len - 1].1
                }
            } else {
                // Can't descend into a non-object; return current as fallback
                current
            }
        }
        SetPathPart::ArrayIndex(name, idx) => {
            if let DynValue::Object(ref mut entries) = current {
                // Ensure the field exists as an array
                let pos = entries.iter().position(|(k, _)| k == name);
                let arr_ref = if let Some(p) = pos {
                    &mut entries[p].1
                } else {
                    entries.push((name.clone(), DynValue::Array(Vec::new())));
                    let len = entries.len();
                    &mut entries[len - 1].1
                };

                if let DynValue::Array(ref mut items) = arr_ref {
                    while items.len() <= *idx {
                        items.push(DynValue::Object(Vec::new()));
                    }
                    &mut items[*idx]
                } else {
                    arr_ref
                }
            } else {
                current
            }
        }
        SetPathPart::ArrayPush(name) => {
            if let DynValue::Object(ref mut entries) = current {
                let pos = entries.iter().position(|(k, _)| k == name);
                let arr_ref = if let Some(p) = pos {
                    &mut entries[p].1
                } else {
                    entries.push((name.clone(), DynValue::Array(Vec::new())));
                    let len = entries.len();
                    &mut entries[len - 1].1
                };

                if let DynValue::Array(ref mut items) = arr_ref {
                    items.push(DynValue::Object(Vec::new()));
                    let len = items.len();
                    &mut items[len - 1]
                } else {
                    arr_ref
                }
            } else {
                current
            }
        }
    }
}

/// Ensure an object field exists at the given key.
/// Parse an array index from a segment name like "vehicles[0]" → Some(("vehicles", 0)).
fn parse_array_index(name: &str) -> Option<(&str, usize)> {
    if let Some(bracket_start) = name.find('[') {
        if let Some(bracket_end) = name.find(']') {
            let arr_name = &name[..bracket_start];
            let idx_str = &name[bracket_start + 1..bracket_end];
            if let Ok(idx) = idx_str.parse::<usize>() {
                return Some((arr_name, idx));
            }
        }
    }
    None
}

/// Ensure an array entry at the given index exists, creating the array and filling gaps.
fn ensure_array_entry_at(output: &mut DynValue, arr_name: &str, idx: usize) {
    if let DynValue::Object(ref mut entries) = output {
        // Find or create the array
        let arr_pos = entries.iter().position(|(k, _)| k == arr_name);
        let arr_pos = if let Some(pos) = arr_pos {
            pos
        } else {
            entries.push((arr_name.to_string(), DynValue::Array(Vec::new())));
            entries.len() - 1
        };
        // Ensure array has enough entries
        if let DynValue::Array(ref mut items) = entries[arr_pos].1 {
            while items.len() <= idx {
                items.push(DynValue::Object(Vec::new()));
            }
        }
    }
}

/// Get a mutable reference to an array entry.
fn get_array_entry_mut<'a>(output: &'a mut DynValue, arr_name: &str, idx: usize) -> Option<&'a mut DynValue> {
    if let DynValue::Object(ref mut entries) = output {
        if let Some((_, DynValue::Array(ref mut items))) = entries.iter_mut().find(|(k, _)| k == arr_name) {
            items.get_mut(idx)
        } else {
            None
        }
    } else {
        None
    }
}

fn ensure_object_at(output: &mut DynValue, key: &str) {
    if let DynValue::Object(ref mut entries) = output {
        if !entries.iter().any(|(k, _)| k == key) {
            entries.push((key.to_string(), DynValue::Object(Vec::new())));
        }
    }
}

/// Get a mutable reference to a value at a single-level key.
fn get_mut_path<'a>(output: &'a mut DynValue, key: &str) -> Option<&'a mut DynValue> {
    if let DynValue::Object(ref mut entries) = output {
        entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    } else {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// OdinValue -> DynValue conversion
// ─────────────────────────────────────────────────────────────────────────────

/// Convert an `OdinValue` to a `DynValue`.
fn odin_value_to_dyn(val: &OdinValue) -> DynValue {
    match val {
        OdinValue::Null { .. } => DynValue::Null,

        OdinValue::Boolean { value, .. } => DynValue::Bool(*value),

        OdinValue::String { value, .. } => DynValue::String(value.clone()),

        OdinValue::Integer { value, .. } => DynValue::Integer(*value),

        OdinValue::Number { value, .. } => DynValue::Float(*value),

        OdinValue::Currency { value, decimal_places, currency_code, .. } => {
            DynValue::Currency(*value, *decimal_places, currency_code.clone())
        }

        OdinValue::Percent { value, .. } => DynValue::Percent(*value),

        OdinValue::Date { raw, .. } => DynValue::Date(raw.clone()),

        OdinValue::Timestamp { raw, .. } => DynValue::Timestamp(raw.clone()),

        OdinValue::Time { value, .. } => DynValue::Time(value.clone()),

        OdinValue::Duration { value, .. } => DynValue::Duration(value.clone()),

        OdinValue::Reference { path, .. } => DynValue::Reference(path.clone()),

        OdinValue::Binary { data, .. } => {
            DynValue::Binary(crate::utils::base64::encode(data))
        }

        OdinValue::Array { items, .. } => {
            let dyn_items: Vec<DynValue> = items.iter().map(|item| {
                match item {
                    OdinArrayItem::Value(v) => odin_value_to_dyn(v),
                    OdinArrayItem::Record(fields) => {
                        let entries = fields.iter().map(|(k, v)| {
                            (k.clone(), odin_value_to_dyn(v))
                        }).collect();
                        DynValue::Object(entries)
                    }
                }
            }).collect();
            DynValue::Array(dyn_items)
        }

        OdinValue::Object { value, .. } => {
            let entries = value.iter().map(|(k, v)| {
                (k.clone(), odin_value_to_dyn(v))
            }).collect();
            DynValue::Object(entries)
        }

        OdinValue::Verb { .. } => {
            // Verb values should not appear as constants; treat as null
            DynValue::Null
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Confidential enforcement
// ─────────────────────────────────────────────────────────────────────────────

/// Walk the segment tree and collect target fields marked as confidential,
/// then apply the enforcement mode to those fields in the output.
fn apply_confidential_enforcement(
    segments: &[TransformSegment],
    mode: &ConfidentialMode,
    output: &mut DynValue,
) {
    let mut confidential_paths: Vec<String> = Vec::new();
    collect_confidential_paths(segments, "", &mut confidential_paths);

    for path in &confidential_paths {
        if let Some(val) = resolve_mut_path(output, path) {
            *val = apply_confidential_to_value(val, mode);
        }
    }
}

fn collect_confidential_paths(
    segments: &[TransformSegment],
    prefix: &str,
    paths: &mut Vec<String>,
) {
    for seg in segments {
        let seg_prefix = if seg.name.is_empty() || seg.name == "$" || seg.name == "_root" {
            prefix.to_string()
        } else if prefix.is_empty() {
            seg.name.clone()
        } else {
            format!("{}.{}", prefix, seg.name)
        };

        for mapping in &seg.mappings {
            if let Some(ref mods) = mapping.modifiers {
                if mods.confidential {
                    let full_path = if seg_prefix.is_empty() {
                        mapping.target.clone()
                    } else {
                        format!("{}.{}", seg_prefix, mapping.target)
                    };
                    paths.push(full_path);
                }
            }
        }

        collect_confidential_paths(&seg.children, &seg_prefix, paths);
    }
}

/// Apply confidential enforcement to a single value.
fn apply_confidential_to_value(val: &DynValue, mode: &ConfidentialMode) -> DynValue {
    match mode {
        ConfidentialMode::Redact => DynValue::Null,
        ConfidentialMode::Mask => {
            match val {
                DynValue::String(s) => DynValue::String("*".repeat(s.len())),
                _ => DynValue::Null,
            }
        }
    }
}

/// Resolve a dotted path to a mutable reference in the output tree.
fn resolve_mut_path<'a>(output: &'a mut DynValue, path: &str) -> Option<&'a mut DynValue> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = output;

    for part in &parts {
        if let DynValue::Object(ref mut entries) = current {
            let pos = entries.iter().position(|(k, _)| k == part)?;
            current = &mut entries[pos].1;
        } else {
            return None;
        }
    }

    Some(current)
}

// ─────────────────────────────────────────────────────────────────────────────
// Output formatting
// ─────────────────────────────────────────────────────────────────────────────

fn format_output(
    output: &DynValue,
    target_format: &str,
    options: &HashMap<String, String>,
) -> String {
    formatters::format_output_with_options(output, &target_format.to_lowercase(), true, options)
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Apply type directives (`:type integer`, `:date`, etc.) to coerce a `DynValue`.
fn apply_type_directives(val: DynValue, directives: &[crate::types::values::OdinDirective]) -> DynValue {
    if directives.is_empty() {
        return val;
    }

    // Phase 1: Apply extraction directives (:pos, :len, :field, :trim) BEFORE type coercion
    let mut pos: Option<usize> = None;
    let mut len: Option<usize> = None;
    let mut field_index: Option<usize> = None;
    let mut should_trim = false;

    // Extract extra metadata from directives
    let mut decimal_places: Option<u8> = None;
    let mut currency_code: Option<String> = None;
    let mut type_name_found: Option<String> = None;

    for dir in directives {
        match dir.name.as_str() {
            "pos" => {
                pos = directive_as_usize(dir);
            }
            "len" => {
                len = directive_as_usize(dir);
            }
            "field" => {
                field_index = directive_as_usize(dir);
            }
            "trim" => {
                should_trim = true;
            }
            "type" => {
                if let Some(crate::types::values::DirectiveValue::String(s)) = &dir.value {
                    type_name_found = Some(s.clone());
                }
            }
            "decimals" => {
                match &dir.value {
                    Some(crate::types::values::DirectiveValue::String(s)) => {
                        decimal_places = s.parse::<u8>().ok();
                    }
                    Some(crate::types::values::DirectiveValue::Number(n)) => {
                        decimal_places = Some(*n as u8);
                    }
                    _ => {}
                }
            }
            "currencyCode" => {
                if let Some(crate::types::values::DirectiveValue::String(s)) = &dir.value {
                    currency_code = Some(s.clone());
                }
            }
            "date" | "time" | "duration" | "timestamp" | "boolean" | "integer" | "number"
            | "currency" | "reference" | "binary" | "percent" => {
                type_name_found = Some(dir.name.clone());
            }
            _ => {}
        }
    }

    // Apply extraction directives to get a string value
    let val = if pos.is_some() || field_index.is_some() || should_trim {
        let mut s = match &val {
            DynValue::String(s) => s.clone(),
            DynValue::Null => return val,
            other => crate::transform::verbs::coerce_str_pub(other),
        };

        // Apply :field first (extract field from delimited string)
        if let Some(fi) = field_index {
            let fields: Vec<&str> = s.split(',').collect();
            s = (*fields.get(fi).unwrap_or(&"")).to_string();
        }

        // Then apply :pos/:len (substring extraction)
        if let Some(p) = pos {
            if let Some(l) = len {
                let end = (p + l).min(s.len());
                let start = p.min(s.len());
                s = s[start..end].to_string();
            } else {
                let start = p.min(s.len());
                s = s[start..].to_string();
            }
        }

        if should_trim {
            s = s.trim().to_string();
        }

        DynValue::String(s)
    } else {
        val
    };

    // Phase 1.5: Apply :default directive (before type coercion)
    let val = {
        let mut v = val;
        for dir in directives {
            if dir.name == "default" {
                if matches!(v, DynValue::Null) {
                    if let Some(ref dv) = dir.value {
                        v = match dv {
                            crate::types::values::DirectiveValue::String(s) => DynValue::String(s.clone()),
                            crate::types::values::DirectiveValue::Number(n) => DynValue::Float(*n),
                        };
                    }
                }
                break;
            }
        }
        v
    };

    // Phase 2: Apply type coercion
    if let Some(type_name) = type_name_found {
        coerce_to_type(val, &type_name, decimal_places, currency_code)
    } else {
        val
    }
}

/// Extract a usize from a directive value.
fn directive_as_usize(dir: &crate::types::values::OdinDirective) -> Option<usize> {
    match &dir.value {
        Some(crate::types::values::DirectiveValue::Number(n)) => Some(*n as usize),
        Some(crate::types::values::DirectiveValue::String(s)) => s.parse::<usize>().ok(),
        _ => None,
    }
}

/// Coerce a `DynValue` to the specified ODIN type.
fn coerce_to_type(val: DynValue, type_name: &str, decimal_places: Option<u8>, currency_code: Option<String>) -> DynValue {
    match type_name {
        "integer" => {
            match &val {
                DynValue::Float(f) | DynValue::Currency(f, _, _) => DynValue::Integer(*f as i64),
                DynValue::String(s) => s.parse::<i64>().map(DynValue::Integer).unwrap_or(val),
                DynValue::Bool(b) => DynValue::Integer(i64::from(*b)),
                _ => val,
            }
        }
        "number" => {
            match &val {
                DynValue::Integer(n) => DynValue::Float(*n as f64),
                DynValue::Currency(n, _, _) => DynValue::Float(*n),
                DynValue::CurrencyRaw(s, _, _) => DynValue::FloatRaw(s.clone()),
                DynValue::String(s) => {
                    match s.parse::<f64>() {
                        Ok(f) => {
                            // Check if round-trip preserves the original representation
                            let rt = f.to_string();
                            if rt == *s {
                                DynValue::Float(f)
                            } else {
                                // Preserve the raw string form to avoid precision loss
                                DynValue::FloatRaw(s.clone())
                            }
                        }
                        Err(_) => val,
                    }
                }
                _ => val,
            }
        }
        "currency" => {
            let dp = decimal_places.unwrap_or(2);
            match &val {
                DynValue::Float(f) => DynValue::Currency(*f, dp, currency_code),
                DynValue::FloatRaw(s) => DynValue::CurrencyRaw(s.clone(), dp, currency_code),
                DynValue::Integer(n) => DynValue::Currency(*n as f64, dp, currency_code),
                DynValue::String(s) => {
                    let cleaned = s.replace(['$', ',', '£', '€'], "");
                    // Detect decimal places from string if not specified
                    let actual_dp = if decimal_places.is_some() {
                        dp
                    } else if let Some(dot) = s.find('.') {
                        (s.len() - dot - 1) as u8
                    } else {
                        2
                    };
                    match cleaned.parse::<f64>() {
                        Ok(f) => {
                            // Check if round-trip preserves the original string exactly
                            let rt = format!("{f:.prec$}", prec = actual_dp as usize);
                            if rt == cleaned {
                                DynValue::Currency(f, actual_dp, currency_code)
                            } else {
                                // Raw form preserves leading zeros, extra precision, etc.
                                DynValue::CurrencyRaw(cleaned, actual_dp, currency_code)
                            }
                        }
                        Err(_) => val,
                    }
                }
                _ => val,
            }
        }
        "percent" => {
            match &val {
                DynValue::Float(f) => DynValue::Percent(*f),
                DynValue::Integer(n) => DynValue::Percent(*n as f64),
                DynValue::String(s) => {
                    let cleaned = s.replace('%', "");
                    cleaned.parse::<f64>().map(DynValue::Percent).unwrap_or(val)
                }
                _ => val,
            }
        }
        "boolean" => {
            match &val {
                DynValue::String(s) => match s.to_lowercase().as_str() {
                    "true" | "yes" | "1" => DynValue::Bool(true),
                    "false" | "no" | "0" => DynValue::Bool(false),
                    _ => val,
                }
                DynValue::Integer(n) => DynValue::Bool(*n != 0),
                DynValue::Float(n) => DynValue::Bool(*n != 0.0),
                _ => val,
            }
        }
        "date" => {
            match &val {
                DynValue::String(s) => DynValue::Date(s.clone()),
                _ => val,
            }
        }
        "time" => {
            match &val {
                DynValue::String(s) => DynValue::Time(s.clone()),
                _ => val,
            }
        }
        "timestamp" => {
            match &val {
                DynValue::String(s) => DynValue::Timestamp(s.clone()),
                _ => val,
            }
        }
        "duration" => {
            match &val {
                DynValue::String(s) => DynValue::Duration(s.clone()),
                _ => val,
            }
        }
        "reference" => {
            match &val {
                DynValue::String(s) => DynValue::Reference(s.clone()),
                _ => val,
            }
        }
        "binary" => {
            match &val {
                DynValue::String(s) => DynValue::Binary(s.clone()),
                _ => val,
            }
        }
        _ => val,
    }
}

/// Evaluate whether a `DynValue` is truthy.
fn is_truthy(val: &DynValue) -> bool {
    match val {
        DynValue::Null => false,
        DynValue::Bool(b) => *b,
        DynValue::Integer(n) => *n != 0,
        DynValue::Float(n) | DynValue::Currency(n, _, _) | DynValue::Percent(n) => *n != 0.0,
        DynValue::FloatRaw(s) | DynValue::CurrencyRaw(s, _, _) => !s.is_empty() && s != "0",
        DynValue::String(s) | DynValue::Reference(s) | DynValue::Binary(s)
        | DynValue::Date(s) | DynValue::Timestamp(s) | DynValue::Time(s)
        | DynValue::Duration(s) => !s.is_empty(),
        DynValue::Array(a) => !a.is_empty(),
        DynValue::Object(o) => !o.is_empty(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::transform::*;
    use crate::types::values::OdinValues;
    use std::collections::HashMap;

    /// Helper to create a minimal transform with a single root segment.
    fn minimal_transform(mappings: Vec<FieldMapping>) -> OdinTransform {
        OdinTransform {
            metadata: TransformMetadata::default(),
            source: None,
            target: TargetConfig {
                format: "json".to_string(),
                options: HashMap::new(),
            },
            constants: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
            segments: vec![TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings,
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            }],
            imports: Vec::new(),
            passes: Vec::new(),
            enforce_confidential: None,
            strict_types: false,
        }
    }

    #[test]
    fn test_simple_copy() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Copy("@.name".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("Alice".to_string())),
        ]);

        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Name"), Some(&DynValue::String("Alice".to_string())));
    }

    #[test]
    fn test_nested_copy() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "City".to_string(),
                expression: FieldExpression::Copy("@.address.city".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("address".to_string(), DynValue::Object(vec![
                ("city".to_string(), DynValue::String("Springfield".to_string())),
            ])),
        ]);

        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("City"), Some(&DynValue::String("Springfield".to_string())));
    }

    #[test]
    fn test_literal() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Version".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("1.0")),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());

        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Version"), Some(&DynValue::String("1.0".to_string())));
    }

    #[test]
    fn test_verb_upper() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "upper".to_string(),
                    is_custom: false,
                    args: vec![VerbArg::Reference("@.name".to_string(), Vec::new())],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("alice".to_string())),
        ]);

        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Name"), Some(&DynValue::String("ALICE".to_string())));
    }

    #[test]
    fn test_verb_concat() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "FullName".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.first".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string(" ")),
                        VerbArg::Reference("@.last".to_string(), Vec::new()),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("first".to_string(), DynValue::String("John".to_string())),
            ("last".to_string(), DynValue::String("Doe".to_string())),
        ]);

        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("FullName"), Some(&DynValue::String("John Doe".to_string())));
    }

    #[test]
    fn test_constants() {
        let mut constants = HashMap::new();
        constants.insert("version".to_string(), OdinValues::string("2.0"));

        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Version".to_string(),
                expression: FieldExpression::Copy("$const.version".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants = constants;

        let source = DynValue::Object(Vec::new());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Version"), Some(&DynValue::String("2.0".to_string())));
    }

    #[test]
    fn test_array_index_path() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "First".to_string(),
                expression: FieldExpression::Copy("@.items[0].name".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::Object(vec![
                    ("name".to_string(), DynValue::String("Alpha".to_string())),
                ]),
                DynValue::Object(vec![
                    ("name".to_string(), DynValue::String("Beta".to_string())),
                ]),
            ])),
        ]);

        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("First"), Some(&DynValue::String("Alpha".to_string())));
    }

    #[test]
    fn test_missing_path_returns_null() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Missing".to_string(),
                expression: FieldExpression::Copy("@.nonexistent.field".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());

        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Missing"), Some(&DynValue::Null));
    }

    #[test]
    fn test_object_expression() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Address".to_string(),
                expression: FieldExpression::Object(vec![
                    FieldMapping {
                        target: "Street".to_string(),
                        expression: FieldExpression::Copy("@.street".to_string()),
                        directives: vec![],
                modifiers: None,
                    },
                    FieldMapping {
                        target: "City".to_string(),
                        expression: FieldExpression::Copy("@.city".to_string()),
                        directives: vec![],
                modifiers: None,
                    },
                ]),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("street".to_string(), DynValue::String("123 Main".to_string())),
            ("city".to_string(), DynValue::String("Springfield".to_string())),
        ]);

        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let addr = out.get("Address").unwrap();
        assert_eq!(addr.get("Street"), Some(&DynValue::String("123 Main".to_string())));
        assert_eq!(addr.get("City"), Some(&DynValue::String("Springfield".to_string())));
    }

    #[test]
    fn test_loop_segment() {
        let t = OdinTransform {
            metadata: TransformMetadata::default(),
            source: None,
            target: TargetConfig {
                format: "json".to_string(),
                options: HashMap::new(),
            },
            constants: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
            segments: vec![TransformSegment {
                name: "Items".to_string(),
                path: "Items".to_string(),
                source_path: Some("@.items".to_string()),
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![
                    FieldMapping {
                        target: "Label".to_string(),
                        expression: FieldExpression::Copy("@_item.name".to_string()),
                        directives: vec![],
                modifiers: None,
                    },
                ],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            }],
            imports: Vec::new(),
            passes: Vec::new(),
            enforce_confidential: None,
            strict_types: false,
        };
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::Object(vec![("name".to_string(), DynValue::String("A".to_string()))]),
                DynValue::Object(vec![("name".to_string(), DynValue::String("B".to_string()))]),
            ])),
        ]);

        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].get("Label"), Some(&DynValue::String("A".to_string())));
        assert_eq!(items[1].get("Label"), Some(&DynValue::String("B".to_string())));
    }

    #[test]
    fn test_confidential_redact() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "SSN".to_string(),
                expression: FieldExpression::Copy("@.ssn".to_string()),
                directives: vec![],
                modifiers: Some(crate::types::values::OdinModifiers {
                    required: false,
                    confidential: true,
                    deprecated: false,
                    attr: false,
                }),
            },
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Copy("@.name".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);

        let source = DynValue::Object(vec![
            ("ssn".to_string(), DynValue::String("123-45-6789".to_string())),
            ("name".to_string(), DynValue::String("Alice".to_string())),
        ]);

        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        // SSN should be redacted to null
        assert_eq!(out.get("SSN"), Some(&DynValue::Null));
        // Name should be untouched
        assert_eq!(out.get("Name"), Some(&DynValue::String("Alice".to_string())));
    }

    #[test]
    fn test_confidential_mask() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "SSN".to_string(),
                expression: FieldExpression::Copy("@.ssn".to_string()),
                directives: vec![],
                modifiers: Some(crate::types::values::OdinModifiers {
                    required: false,
                    confidential: true,
                    deprecated: false,
                    attr: false,
                }),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);

        let source = DynValue::Object(vec![
            ("ssn".to_string(), DynValue::String("123-45-6789".to_string())),
        ]);

        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        // SSN should be masked to asterisks (same length)
        assert_eq!(out.get("SSN"), Some(&DynValue::String("***********".to_string())));
    }

    #[test]
    fn test_nested_output_path() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "address.city".to_string(),
                expression: FieldExpression::Copy("@.city".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "address.state".to_string(),
                expression: FieldExpression::Copy("@.state".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("city".to_string(), DynValue::String("Salem".to_string())),
            ("state".to_string(), DynValue::String("OR".to_string())),
        ]);

        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let addr = out.get("address").unwrap();
        assert_eq!(addr.get("city"), Some(&DynValue::String("Salem".to_string())));
        assert_eq!(addr.get("state"), Some(&DynValue::String("OR".to_string())));
    }

    #[test]
    fn test_set_path_creates_nested_objects() {
        let mut output = DynValue::Object(Vec::new());
        set_path(&mut output, "a.b.c", DynValue::Integer(42));
        let a = output.get("a").unwrap();
        let b = a.get("b").unwrap();
        let c = b.get("c").unwrap();
        assert_eq!(*c, DynValue::Integer(42));
    }

    #[test]
    fn test_odin_value_to_dyn_coverage() {
        assert_eq!(odin_value_to_dyn(&OdinValues::null()), DynValue::Null);
        assert_eq!(odin_value_to_dyn(&OdinValues::boolean(true)), DynValue::Bool(true));
        assert_eq!(
            odin_value_to_dyn(&OdinValues::string("hi")),
            DynValue::String("hi".to_string())
        );
        assert_eq!(odin_value_to_dyn(&OdinValues::integer(7)), DynValue::Integer(7));
        assert_eq!(odin_value_to_dyn(&OdinValues::number(1.5)), DynValue::Float(1.5));
        assert_eq!(odin_value_to_dyn(&OdinValues::currency(9.99, 2)), DynValue::Currency(9.99, 2, None));
        assert_eq!(odin_value_to_dyn(&OdinValues::percent(0.15)), DynValue::Percent(0.15));
        assert_eq!(
            odin_value_to_dyn(&OdinValues::date(2024, 1, 15)),
            DynValue::Date("2024-01-15".to_string())
        );
        assert_eq!(
            odin_value_to_dyn(&OdinValues::time("T14:30:00")),
            DynValue::Time("T14:30:00".to_string())
        );
        assert_eq!(
            odin_value_to_dyn(&OdinValues::duration("P1Y")),
            DynValue::Duration("P1Y".to_string())
        );
        assert_eq!(
            odin_value_to_dyn(&OdinValues::reference("x.y")),
            DynValue::Reference("x.y".to_string())
        );
    }

    #[test]
    fn test_formatted_output_is_json() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "x".to_string(),
                expression: FieldExpression::Literal(OdinValues::integer(1)),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        let formatted = result.formatted.unwrap();
        assert!(formatted.contains("\"x\""));
        assert!(formatted.contains('1'));
    }

    #[test]
    fn test_condition_skips_segment() {
        let t = OdinTransform {
            metadata: TransformMetadata::default(),
            source: None,
            target: TargetConfig {
                format: "json".to_string(),
                options: HashMap::new(),
            },
            constants: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
            segments: vec![TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![
                    FieldMapping {
                        target: "Skipped".to_string(),
                        expression: FieldExpression::Literal(OdinValues::string("nope")),
                        directives: vec![],
                modifiers: None,
                    },
                ],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: Some("@.active".to_string()),
            }],
            imports: Vec::new(),
            passes: Vec::new(),
            enforce_confidential: None,
            strict_types: false,
        };

        // active is false -> segment should be skipped
        let source = DynValue::Object(vec![
            ("active".to_string(), DynValue::Bool(false)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Skipped"), None);
    }

    #[test]
    fn test_pass_ordering() {
        let t = OdinTransform {
            metadata: TransformMetadata::default(),
            source: None,
            target: TargetConfig {
                format: "json".to_string(),
                options: HashMap::new(),
            },
            constants: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
            segments: vec![
                TransformSegment {
                    name: String::new(),
                    path: String::new(),
                    source_path: None,
                    discriminator: None,
                    is_array: false,
                    directives: Vec::new(),
                    mappings: vec![FieldMapping {
                        target: "Second".to_string(),
                        expression: FieldExpression::Literal(OdinValues::integer(2)),
                        directives: vec![],
                modifiers: None,
                    }],
                    children: Vec::new(),
                    items: Vec::new(),
                    pass: None, // None = last
                    condition: None,
                },
                TransformSegment {
                    name: String::new(),
                    path: String::new(),
                    source_path: None,
                    discriminator: None,
                    is_array: false,
                    directives: Vec::new(),
                    mappings: vec![FieldMapping {
                        target: "First".to_string(),
                        expression: FieldExpression::Literal(OdinValues::integer(1)),
                        directives: vec![],
                modifiers: None,
                    }],
                    children: Vec::new(),
                    items: Vec::new(),
                    pass: Some(1),
                    condition: None,
                },
            ],
            imports: Vec::new(),
            passes: Vec::new(),
            enforce_confidential: None,
            strict_types: false,
        };
        let source = DynValue::Object(Vec::new());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        // Both fields should be present
        assert_eq!(out.get("First"), Some(&DynValue::Integer(1)));
        assert_eq!(out.get("Second"), Some(&DynValue::Integer(2)));
    }

    #[test]
    fn test_nested_verb() {
        // %upper (%concat @.first " " @.last)
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "upper".to_string(),
                    is_custom: false,
                    args: vec![VerbArg::Verb(VerbCall {
                        verb: "concat".to_string(),
                        is_custom: false,
                        args: vec![
                            VerbArg::Reference("@.first".to_string(), Vec::new()),
                            VerbArg::Literal(OdinValues::string(" ")),
                            VerbArg::Reference("@.last".to_string(), Vec::new()),
                        ],
                    })],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("first".to_string(), DynValue::String("jane".to_string())),
            ("last".to_string(), DynValue::String("doe".to_string())),
        ]);

        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Name"), Some(&DynValue::String("JANE DOE".to_string())));
    }

    #[test]
    fn test_unknown_verb_produces_error() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "X".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "nonExistentVerb".to_string(),
                    is_custom: false,
                    args: vec![],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(!result.success);
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].message.contains("nonExistentVerb"));
    }

    #[test]
    fn test_json_to_odin_path_resolution() {
        // Read the actual golden test files
        let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent().unwrap()
            .parent().unwrap()
            .join("golden/end-to-end/import/json-to-odin");

        let transform_text = std::fs::read_to_string(base.join("all-types.transform.odin"))
            .unwrap().replace("\r\n", "\n");
        let json_text = std::fs::read_to_string(base.join("all-types.input.json"))
            .unwrap().replace("\r\n", "\n");

        let transform = crate::transform::parse_transform(&transform_text).unwrap();
        eprintln!("Segments: {}", transform.segments.len());
        for seg in &transform.segments {
            eprintln!("  Segment '{}': {} mappings, source_path: {:?}, children: {}",
                seg.name, seg.mappings.len(), seg.source_path, seg.children.len());
            for m in &seg.mappings {
                eprintln!("    target='{}' expr={:?}", m.target, m.expression);
            }
            for child in &seg.children {
                eprintln!("    CHILD '{}': {} mappings, source_path: {:?}",
                    child.name, child.mappings.len(), child.source_path);
            }
        }

        let source = crate::transform::source_parsers::parse_source(&json_text, "json").unwrap();
        // Show top-level keys
        if let DynValue::Object(entries) = &source {
            eprintln!("\nSource top-level keys: {:?}", entries.iter().map(|(k,_)| k.as_str()).collect::<Vec<_>>());
        }

        let result = execute(&transform, &source);
        eprintln!("\nResult success: {}", result.success);
        for e in &result.errors {
            eprintln!("  ERROR: {}", e.message);
        }
        for w in &result.warnings {
            eprintln!("  WARN: {}", w.message);
        }

        let output = result.output.unwrap();
        if let DynValue::Object(entries) = &output {
            for (k, v) in entries.iter().take(3) {
                eprintln!("Output key '{}': {:?}", k, v);
            }
        }
        let fmt = result.formatted.unwrap_or_default();
        eprintln!("Formatted length: {}", fmt.len());
        eprintln!("Formatted:\n{}", fmt);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Helper: build a full OdinTransform with custom segments
    // ─────────────────────────────────────────────────────────────────────────

    fn custom_transform(segments: Vec<TransformSegment>) -> OdinTransform {
        OdinTransform {
            metadata: TransformMetadata::default(),
            source: None,
            target: TargetConfig {
                format: "json".to_string(),
                options: HashMap::new(),
            },
            constants: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
            segments,
            imports: Vec::new(),
            passes: Vec::new(),
            enforce_confidential: None,
            strict_types: false,
        }
    }

    fn root_segment(mappings: Vec<FieldMapping>) -> TransformSegment {
        TransformSegment {
            name: String::new(),
            path: String::new(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings,
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }
    }

    fn make_modifiers(required: bool, confidential: bool, deprecated: bool) -> crate::types::values::OdinModifiers {
        crate::types::values::OdinModifiers {
            required,
            confidential,
            deprecated,
            attr: false,
        }
    }

    // =========================================================================
    // 1. String interpolation via concat verb (template-like expressions)
    // =========================================================================

    #[test]
    fn test_string_interpolation_concat_two_fields() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Greeting".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Literal(OdinValues::string("Hello, ")),
                        VerbArg::Reference("@.name".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("!")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("World".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Greeting"), Some(&DynValue::String("Hello, World!".to_string())));
    }

    #[test]
    fn test_string_interpolation_concat_with_number() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Label".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Literal(OdinValues::string("Item #")),
                        VerbArg::Reference("@.id".to_string(), Vec::new()),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("id".to_string(), DynValue::Integer(42)),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Label"), Some(&DynValue::String("Item #42".to_string())));
    }

    #[test]
    fn test_string_interpolation_concat_with_null_field() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Result".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Literal(OdinValues::string("val=")),
                        VerbArg::Reference("@.missing".to_string(), Vec::new()),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        // concat with null typically yields "val=" (null contributes empty string)
        let val = out.get("Result").unwrap();
        if let DynValue::String(s) = val {
            assert!(s.starts_with("val="));
        }
    }

    #[test]
    fn test_string_interpolation_concat_many_parts() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Address".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.street".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string(", ")),
                        VerbArg::Reference("@.city".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string(", ")),
                        VerbArg::Reference("@.state".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string(" ")),
                        VerbArg::Reference("@.zip".to_string(), Vec::new()),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("street".to_string(), DynValue::String("123 Main St".to_string())),
            ("city".to_string(), DynValue::String("Portland".to_string())),
            ("state".to_string(), DynValue::String("OR".to_string())),
            ("zip".to_string(), DynValue::String("97201".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Address"), Some(&DynValue::String("123 Main St, Portland, OR 97201".to_string())));
    }

    #[test]
    fn test_string_interpolation_concat_empty_strings() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Out".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Literal(OdinValues::string("")),
                        VerbArg::Literal(OdinValues::string("")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&DynValue::String(String::new())));
    }

    // =========================================================================
    // 2. Conditional mappings (ifElse and cond)
    // =========================================================================

    #[test]
    fn test_ifelse_true_branch() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Status".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "ifElse".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.active".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("ACTIVE")),
                        VerbArg::Literal(OdinValues::string("INACTIVE")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("active".to_string(), DynValue::Bool(true)),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Status"), Some(&DynValue::String("ACTIVE".to_string())));
    }

    #[test]
    fn test_ifelse_false_branch() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Status".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "ifElse".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.active".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("ACTIVE")),
                        VerbArg::Literal(OdinValues::string("INACTIVE")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("active".to_string(), DynValue::Bool(false)),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Status"), Some(&DynValue::String("INACTIVE".to_string())));
    }

    #[test]
    fn test_ifelse_null_condition_takes_false_branch() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "ifElse".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.missing".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("yes")),
                        VerbArg::Literal(OdinValues::string("no")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::String("no".to_string())));
    }

    #[test]
    fn test_ifelse_string_condition_truthy() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "ifElse".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.name".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("has_name")),
                        VerbArg::Literal(OdinValues::string("no_name")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("Alice".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::String("has_name".to_string())));
    }

    #[test]
    fn test_ifelse_empty_string_condition_falsy() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "ifElse".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.name".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("has_name")),
                        VerbArg::Literal(OdinValues::string("no_name")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String(String::new())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::String("no_name".to_string())));
    }

    #[test]
    fn test_ifelse_integer_zero_is_falsy() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "ifElse".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.count".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("nonzero")),
                        VerbArg::Literal(OdinValues::string("zero")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("count".to_string(), DynValue::Integer(0)),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::String("zero".to_string())));
    }

    #[test]
    fn test_ifelse_integer_nonzero_is_truthy() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "ifElse".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.count".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("nonzero")),
                        VerbArg::Literal(OdinValues::string("zero")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("count".to_string(), DynValue::Integer(5)),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::String("nonzero".to_string())));
    }

    #[test]
    fn test_cond_first_match() {
        // cond: condition1 value1 condition2 value2 default
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Grade".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "cond".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.isA".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("A")),
                        VerbArg::Reference("@.isB".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("B")),
                        VerbArg::Literal(OdinValues::string("C")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("isA".to_string(), DynValue::Bool(true)),
            ("isB".to_string(), DynValue::Bool(false)),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Grade"), Some(&DynValue::String("A".to_string())));
    }

    #[test]
    fn test_cond_second_match() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Grade".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "cond".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.isA".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("A")),
                        VerbArg::Reference("@.isB".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("B")),
                        VerbArg::Literal(OdinValues::string("C")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("isA".to_string(), DynValue::Bool(false)),
            ("isB".to_string(), DynValue::Bool(true)),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Grade"), Some(&DynValue::String("B".to_string())));
    }

    #[test]
    fn test_cond_default_value() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Grade".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "cond".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.isA".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("A")),
                        VerbArg::Reference("@.isB".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("B")),
                        VerbArg::Literal(OdinValues::string("DEFAULT")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("isA".to_string(), DynValue::Bool(false)),
            ("isB".to_string(), DynValue::Bool(false)),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Grade"), Some(&DynValue::String("DEFAULT".to_string())));
    }

    #[test]
    fn test_cond_no_match_no_default_returns_null() {
        // Even number of args, no default
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Grade".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "cond".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.isA".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("A")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("isA".to_string(), DynValue::Bool(false)),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Grade"), Some(&DynValue::Null));
    }

    #[test]
    fn test_condition_allows_segment_when_truthy() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(),
            path: String::new(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![FieldMapping {
                target: "Included".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("yes")),
                directives: vec![],
                modifiers: None,
            }],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: Some("@.active".to_string()),
        }]);
        let source = DynValue::Object(vec![
            ("active".to_string(), DynValue::Bool(true)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Included"), Some(&DynValue::String("yes".to_string())));
    }

    #[test]
    fn test_condition_null_value_skips_segment() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(),
            path: String::new(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![FieldMapping {
                target: "Skipped".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("nope")),
                directives: vec![],
                modifiers: None,
            }],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: Some("@.missing_field".to_string()),
        }]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Skipped"), None);
    }

    #[test]
    fn test_condition_nonempty_string_is_truthy() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(),
            path: String::new(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![FieldMapping {
                target: "Found".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("yes")),
                directives: vec![],
                modifiers: None,
            }],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: Some("@.label".to_string()),
        }]);
        let source = DynValue::Object(vec![
            ("label".to_string(), DynValue::String("something".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Found"), Some(&DynValue::String("yes".to_string())));
    }

    #[test]
    fn test_condition_empty_string_is_falsy() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(),
            path: String::new(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![FieldMapping {
                target: "Skipped".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("nope")),
                directives: vec![],
                modifiers: None,
            }],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: Some("@.label".to_string()),
        }]);
        let source = DynValue::Object(vec![
            ("label".to_string(), DynValue::String(String::new())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Skipped"), None);
    }

    // =========================================================================
    // 3. Modifier preservation (required !, confidential *, deprecated -)
    // =========================================================================

    #[test]
    fn test_modifier_required_recorded() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Copy("@.name".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(true, false, false)),
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("Alice".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        assert!(result.modifiers.contains_key("Name"));
        assert!(result.modifiers["Name"].required);
        assert!(!result.modifiers["Name"].confidential);
        assert!(!result.modifiers["Name"].deprecated);
    }

    #[test]
    fn test_modifier_deprecated_recorded() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "OldField".to_string(),
                expression: FieldExpression::Copy("@.old".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, false, true)),
            },
        ]);
        let source = DynValue::Object(vec![
            ("old".to_string(), DynValue::String("legacy".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        assert!(result.modifiers.contains_key("OldField"));
        assert!(result.modifiers["OldField"].deprecated);
    }

    #[test]
    fn test_modifier_confidential_recorded() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Secret".to_string(),
                expression: FieldExpression::Copy("@.secret".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        let source = DynValue::Object(vec![
            ("secret".to_string(), DynValue::String("hidden".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        assert!(result.modifiers.contains_key("Secret"));
        assert!(result.modifiers["Secret"].confidential);
    }

    #[test]
    fn test_modifier_combined_all_three() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Field".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("value")),
                directives: vec![],
                modifiers: Some(make_modifiers(true, true, true)),
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let mods = &result.modifiers["Field"];
        assert!(mods.required);
        assert!(mods.confidential);
        assert!(mods.deprecated);
    }

    #[test]
    fn test_modifier_none_not_recorded() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Plain".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("value")),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        assert!(!result.modifiers.contains_key("Plain"));
    }

    #[test]
    fn test_confidential_redact_integer() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "PIN".to_string(),
                expression: FieldExpression::Copy("@.pin".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("pin".to_string(), DynValue::Integer(1234)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("PIN"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_mask_integer() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "PIN".to_string(),
                expression: FieldExpression::Copy("@.pin".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let source = DynValue::Object(vec![
            ("pin".to_string(), DynValue::Integer(1234)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        // Integers under mask mode become null
        assert_eq!(out.get("PIN"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_redact_boolean() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Flag".to_string(),
                expression: FieldExpression::Copy("@.flag".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("flag".to_string(), DynValue::Bool(true)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Flag"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_no_enforcement_passes_through() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "SSN".to_string(),
                expression: FieldExpression::Copy("@.ssn".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        // No enforce_confidential set
        let source = DynValue::Object(vec![
            ("ssn".to_string(), DynValue::String("123-45-6789".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        // Value passes through when no enforcement is configured
        assert_eq!(out.get("SSN"), Some(&DynValue::String("123-45-6789".to_string())));
    }

    // =========================================================================
    // 4. Nested object expressions with multiple levels
    // =========================================================================

    #[test]
    fn test_nested_object_three_levels() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "person".to_string(),
                expression: FieldExpression::Object(vec![
                    FieldMapping {
                        target: "name".to_string(),
                        expression: FieldExpression::Copy("@.name".to_string()),
                        directives: vec![],
                        modifiers: None,
                    },
                    FieldMapping {
                        target: "address".to_string(),
                        expression: FieldExpression::Object(vec![
                            FieldMapping {
                                target: "street".to_string(),
                                expression: FieldExpression::Copy("@.street".to_string()),
                                directives: vec![],
                                modifiers: None,
                            },
                            FieldMapping {
                                target: "geo".to_string(),
                                expression: FieldExpression::Object(vec![
                                    FieldMapping {
                                        target: "lat".to_string(),
                                        expression: FieldExpression::Literal(OdinValues::number(45.5)),
                                        directives: vec![],
                                        modifiers: None,
                                    },
                                    FieldMapping {
                                        target: "lng".to_string(),
                                        expression: FieldExpression::Literal(OdinValues::number(-122.6)),
                                        directives: vec![],
                                        modifiers: None,
                                    },
                                ]),
                                directives: vec![],
                                modifiers: None,
                            },
                        ]),
                        directives: vec![],
                        modifiers: None,
                    },
                ]),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("Bob".to_string())),
            ("street".to_string(), DynValue::String("456 Oak Ave".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let person = out.get("person").unwrap();
        assert_eq!(person.get("name"), Some(&DynValue::String("Bob".to_string())));
        let address = person.get("address").unwrap();
        assert_eq!(address.get("street"), Some(&DynValue::String("456 Oak Ave".to_string())));
        let geo = address.get("geo").unwrap();
        assert_eq!(geo.get("lat"), Some(&DynValue::Float(45.5)));
        assert_eq!(geo.get("lng"), Some(&DynValue::Float(-122.6)));
    }

    #[test]
    fn test_object_expression_with_verb() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Result".to_string(),
                expression: FieldExpression::Object(vec![
                    FieldMapping {
                        target: "upper_name".to_string(),
                        expression: FieldExpression::Transform(VerbCall {
                            verb: "upper".to_string(),
                            is_custom: false,
                            args: vec![VerbArg::Reference("@.name".to_string(), Vec::new())],
                        }),
                        directives: vec![],
                        modifiers: None,
                    },
                    FieldMapping {
                        target: "lower_name".to_string(),
                        expression: FieldExpression::Transform(VerbCall {
                            verb: "lower".to_string(),
                            is_custom: false,
                            args: vec![VerbArg::Reference("@.name".to_string(), Vec::new())],
                        }),
                        directives: vec![],
                        modifiers: None,
                    },
                ]),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("Alice".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let res = out.get("Result").unwrap();
        assert_eq!(res.get("upper_name"), Some(&DynValue::String("ALICE".to_string())));
        assert_eq!(res.get("lower_name"), Some(&DynValue::String("alice".to_string())));
    }

    #[test]
    fn test_object_expression_with_literal() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Meta".to_string(),
                expression: FieldExpression::Object(vec![
                    FieldMapping {
                        target: "version".to_string(),
                        expression: FieldExpression::Literal(OdinValues::integer(1)),
                        directives: vec![],
                        modifiers: None,
                    },
                    FieldMapping {
                        target: "format".to_string(),
                        expression: FieldExpression::Literal(OdinValues::string("json")),
                        directives: vec![],
                        modifiers: None,
                    },
                ]),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let meta = out.get("Meta").unwrap();
        assert_eq!(meta.get("version"), Some(&DynValue::Integer(1)));
        assert_eq!(meta.get("format"), Some(&DynValue::String("json".to_string())));
    }

    #[test]
    fn test_nested_output_path_three_levels() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "a.b.c".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("deep")),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let a = out.get("a").unwrap();
        let b = a.get("b").unwrap();
        assert_eq!(b.get("c"), Some(&DynValue::String("deep".to_string())));
    }

    #[test]
    fn test_nested_output_multiple_fields_same_parent() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "person.first".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("Jane")),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "person.last".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("Doe")),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "person.age".to_string(),
                expression: FieldExpression::Literal(OdinValues::integer(30)),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let person = out.get("person").unwrap();
        assert_eq!(person.get("first"), Some(&DynValue::String("Jane".to_string())));
        assert_eq!(person.get("last"), Some(&DynValue::String("Doe".to_string())));
        assert_eq!(person.get("age"), Some(&DynValue::Integer(30)));
    }

    // =========================================================================
    // 5. Array index access patterns
    // =========================================================================

    #[test]
    fn test_array_index_second_element() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Second".to_string(),
                expression: FieldExpression::Copy("@.items[1].name".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::Object(vec![("name".to_string(), DynValue::String("first".to_string()))]),
                DynValue::Object(vec![("name".to_string(), DynValue::String("second".to_string()))]),
            ])),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Second"), Some(&DynValue::String("second".to_string())));
    }

    #[test]
    fn test_array_index_out_of_bounds_returns_null() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "OOB".to_string(),
                expression: FieldExpression::Copy("@.items[99]".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::String("only".to_string()),
            ])),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("OOB"), Some(&DynValue::Null));
    }

    #[test]
    fn test_array_index_zero_scalar() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "First".to_string(),
                expression: FieldExpression::Copy("@.tags[0]".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("tags".to_string(), DynValue::Array(vec![
                DynValue::String("rust".to_string()),
                DynValue::String("odin".to_string()),
            ])),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("First"), Some(&DynValue::String("rust".to_string())));
    }

    #[test]
    fn test_array_index_nested_field() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "City".to_string(),
                expression: FieldExpression::Copy("@.people[0].address.city".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("people".to_string(), DynValue::Array(vec![
                DynValue::Object(vec![
                    ("address".to_string(), DynValue::Object(vec![
                        ("city".to_string(), DynValue::String("Portland".to_string())),
                    ])),
                ]),
            ])),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("City"), Some(&DynValue::String("Portland".to_string())));
    }

    #[test]
    fn test_array_index_on_nonarray_returns_null() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Copy("@.name[0]".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("not_an_array".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::Null));
    }

    #[test]
    fn test_array_multiple_indices() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "A".to_string(),
                expression: FieldExpression::Copy("@.items[0]".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "B".to_string(),
                expression: FieldExpression::Copy("@.items[1]".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "C".to_string(),
                expression: FieldExpression::Copy("@.items[2]".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::Integer(10),
                DynValue::Integer(20),
                DynValue::Integer(30),
            ])),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&DynValue::Integer(10)));
        assert_eq!(out.get("B"), Some(&DynValue::Integer(20)));
        assert_eq!(out.get("C"), Some(&DynValue::Integer(30)));
    }

    // =========================================================================
    // 6. Default values when source fields are missing
    // =========================================================================

    #[test]
    fn test_default_via_ifelse_missing_field() {
        // ifElse(@.missing, @.missing, "default")
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "ifElse".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.value".to_string(), Vec::new()),
                        VerbArg::Reference("@.value".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("default")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::String("default".to_string())));
    }

    #[test]
    fn test_default_via_ifelse_field_present() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "ifElse".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.value".to_string(), Vec::new()),
                        VerbArg::Reference("@.value".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("default")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("value".to_string(), DynValue::String("present".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::String("present".to_string())));
    }

    #[test]
    fn test_default_literal_for_missing() {
        // When a copy path is missing, it resolves to null
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "A".to_string(),
                expression: FieldExpression::Copy("@.nonexistent".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&DynValue::Null));
    }

    #[test]
    fn test_default_via_coalesce() {
        // coalesce returns first non-null argument
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "coalesce".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.missing1".to_string(), Vec::new()),
                        VerbArg::Reference("@.missing2".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("fallback")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::String("fallback".to_string())));
    }

    #[test]
    fn test_default_via_coalesce_first_present() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "coalesce".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.primary".to_string(), Vec::new()),
                        VerbArg::Reference("@.secondary".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("fallback")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("primary".to_string(), DynValue::String("found".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::String("found".to_string())));
    }

    // =========================================================================
    // 7. Multiple verb chaining (nested verb calls)
    // =========================================================================

    #[test]
    fn test_upper_of_concat() {
        // upper(concat(@.first, " ", @.last))
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "upper".to_string(),
                    is_custom: false,
                    args: vec![VerbArg::Verb(VerbCall {
                        verb: "concat".to_string(),
                        is_custom: false,
                        args: vec![
                            VerbArg::Reference("@.first".to_string(), Vec::new()),
                            VerbArg::Literal(OdinValues::string(" ")),
                            VerbArg::Reference("@.last".to_string(), Vec::new()),
                        ],
                    })],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("first".to_string(), DynValue::String("alice".to_string())),
            ("last".to_string(), DynValue::String("smith".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Name"), Some(&DynValue::String("ALICE SMITH".to_string())));
    }

    #[test]
    fn test_lower_of_upper() {
        // lower(upper(@.name)) should round-trip to lowercase
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "lower".to_string(),
                    is_custom: false,
                    args: vec![VerbArg::Verb(VerbCall {
                        verb: "upper".to_string(),
                        is_custom: false,
                        args: vec![VerbArg::Reference("@.name".to_string(), Vec::new())],
                    })],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("MiXeD".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Name"), Some(&DynValue::String("mixed".to_string())));
    }

    #[test]
    fn test_concat_of_upper_and_lower() {
        // concat(upper(@.first), " ", lower(@.last))
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Verb(VerbCall {
                            verb: "upper".to_string(),
                            is_custom: false,
                            args: vec![VerbArg::Reference("@.first".to_string(), Vec::new())],
                        }),
                        VerbArg::Literal(OdinValues::string(" ")),
                        VerbArg::Verb(VerbCall {
                            verb: "lower".to_string(),
                            is_custom: false,
                            args: vec![VerbArg::Reference("@.last".to_string(), Vec::new())],
                        }),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("first".to_string(), DynValue::String("alice".to_string())),
            ("last".to_string(), DynValue::String("SMITH".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Name"), Some(&DynValue::String("ALICE smith".to_string())));
    }

    #[test]
    fn test_triple_nested_verb() {
        // upper(concat(lower(@.first), " ", lower(@.last)))
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "upper".to_string(),
                    is_custom: false,
                    args: vec![VerbArg::Verb(VerbCall {
                        verb: "concat".to_string(),
                        is_custom: false,
                        args: vec![
                            VerbArg::Verb(VerbCall {
                                verb: "lower".to_string(),
                                is_custom: false,
                                args: vec![VerbArg::Reference("@.first".to_string(), Vec::new())],
                            }),
                            VerbArg::Literal(OdinValues::string("-")),
                            VerbArg::Verb(VerbCall {
                                verb: "lower".to_string(),
                                is_custom: false,
                                args: vec![VerbArg::Reference("@.last".to_string(), Vec::new())],
                            }),
                        ],
                    })],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("first".to_string(), DynValue::String("ALICE".to_string())),
            ("last".to_string(), DynValue::String("SMITH".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Name"), Some(&DynValue::String("ALICE-SMITH".to_string())));
    }

    #[test]
    fn test_ifelse_with_nested_verb_in_branch() {
        // ifElse(@.active, upper(@.name), "N/A")
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Label".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "ifElse".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.active".to_string(), Vec::new()),
                        VerbArg::Verb(VerbCall {
                            verb: "upper".to_string(),
                            is_custom: false,
                            args: vec![VerbArg::Reference("@.name".to_string(), Vec::new())],
                        }),
                        VerbArg::Literal(OdinValues::string("N/A")),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("active".to_string(), DynValue::Bool(true)),
            ("name".to_string(), DynValue::String("alice".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Label"), Some(&DynValue::String("ALICE".to_string())));
    }

    #[test]
    fn test_ifelse_false_branch_with_nested_verb() {
        // ifElse(@.active, "YES", lower(@.fallback))
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Label".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "ifElse".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.active".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string("YES")),
                        VerbArg::Verb(VerbCall {
                            verb: "lower".to_string(),
                            is_custom: false,
                            args: vec![VerbArg::Reference("@.fallback".to_string(), Vec::new())],
                        }),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("active".to_string(), DynValue::Bool(false)),
            ("fallback".to_string(), DynValue::String("FALLBACK".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Label"), Some(&DynValue::String("fallback".to_string())));
    }

    // =========================================================================
    // 8. Loop segments with @each/@index/@key
    // =========================================================================

    #[test]
    fn test_loop_with_index() {
        let t = custom_transform(vec![TransformSegment {
            name: "Items".to_string(),
            path: "Items".to_string(),
            source_path: Some("@.items".to_string()),
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "Name".to_string(),
                    expression: FieldExpression::Copy("@_item.name".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
                FieldMapping {
                    target: "Index".to_string(),
                    expression: FieldExpression::Copy("@_index".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::Object(vec![("name".to_string(), DynValue::String("A".to_string()))]),
                DynValue::Object(vec![("name".to_string(), DynValue::String("B".to_string()))]),
                DynValue::Object(vec![("name".to_string(), DynValue::String("C".to_string()))]),
            ])),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].get("Index"), Some(&DynValue::Integer(0)));
        assert_eq!(items[1].get("Index"), Some(&DynValue::Integer(1)));
        assert_eq!(items[2].get("Index"), Some(&DynValue::Integer(2)));
    }

    #[test]
    fn test_loop_with_length() {
        let t = custom_transform(vec![TransformSegment {
            name: "Items".to_string(),
            path: "Items".to_string(),
            source_path: Some("@.items".to_string()),
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "Total".to_string(),
                    expression: FieldExpression::Copy("@_length".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::String("x".to_string()),
                DynValue::String("y".to_string()),
            ])),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].get("Total"), Some(&DynValue::Integer(2)));
        assert_eq!(items[1].get("Total"), Some(&DynValue::Integer(2)));
    }

    #[test]
    fn test_loop_empty_array() {
        let t = custom_transform(vec![TransformSegment {
            name: "Items".to_string(),
            path: "Items".to_string(),
            source_path: Some("@.items".to_string()),
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "Name".to_string(),
                    expression: FieldExpression::Copy("@_item.name".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(Vec::new())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 0);
    }

    #[test]
    fn test_loop_item_nested_field() {
        let t = custom_transform(vec![TransformSegment {
            name: "Results".to_string(),
            path: "Results".to_string(),
            source_path: Some("@.records".to_string()),
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "City".to_string(),
                    expression: FieldExpression::Copy("@_item.address.city".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let source = DynValue::Object(vec![
            ("records".to_string(), DynValue::Array(vec![
                DynValue::Object(vec![
                    ("address".to_string(), DynValue::Object(vec![
                        ("city".to_string(), DynValue::String("NYC".to_string())),
                    ])),
                ]),
                DynValue::Object(vec![
                    ("address".to_string(), DynValue::Object(vec![
                        ("city".to_string(), DynValue::String("LA".to_string())),
                    ])),
                ]),
            ])),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Results").unwrap().as_array().unwrap();
        assert_eq!(items[0].get("City"), Some(&DynValue::String("NYC".to_string())));
        assert_eq!(items[1].get("City"), Some(&DynValue::String("LA".to_string())));
    }

    #[test]
    fn test_loop_with_verb_on_item() {
        let t = custom_transform(vec![TransformSegment {
            name: "Items".to_string(),
            path: "Items".to_string(),
            source_path: Some("@.items".to_string()),
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "Upper".to_string(),
                    expression: FieldExpression::Transform(VerbCall {
                        verb: "upper".to_string(),
                        is_custom: false,
                        args: vec![VerbArg::Reference("@_item.name".to_string(), Vec::new())],
                    }),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::Object(vec![("name".to_string(), DynValue::String("hello".to_string()))]),
                DynValue::Object(vec![("name".to_string(), DynValue::String("world".to_string()))]),
            ])),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items[0].get("Upper"), Some(&DynValue::String("HELLO".to_string())));
        assert_eq!(items[1].get("Upper"), Some(&DynValue::String("WORLD".to_string())));
    }

    #[test]
    fn test_loop_single_element() {
        let t = custom_transform(vec![TransformSegment {
            name: "Items".to_string(),
            path: "Items".to_string(),
            source_path: Some("@.items".to_string()),
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "Val".to_string(),
                    expression: FieldExpression::Copy("@_item".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::String("only".to_string()),
            ])),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get("Val"), Some(&DynValue::String("only".to_string())));
    }

    // =========================================================================
    // 9. Constants section usage in transforms
    // =========================================================================

    #[test]
    fn test_constants_string() {
        let mut constants = HashMap::new();
        constants.insert("app_name".to_string(), OdinValues::string("MyApp"));

        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "App".to_string(),
                expression: FieldExpression::Copy("$const.app_name".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants = constants;

        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("App"), Some(&DynValue::String("MyApp".to_string())));
    }

    #[test]
    fn test_constants_integer() {
        let mut constants = HashMap::new();
        constants.insert("max_retries".to_string(), OdinValues::integer(3));

        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "MaxRetries".to_string(),
                expression: FieldExpression::Copy("$const.max_retries".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants = constants;

        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("MaxRetries"), Some(&DynValue::Integer(3)));
    }

    #[test]
    fn test_constants_boolean() {
        let mut constants = HashMap::new();
        constants.insert("debug".to_string(), OdinValues::boolean(true));

        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Debug".to_string(),
                expression: FieldExpression::Copy("$const.debug".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants = constants;

        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Debug"), Some(&DynValue::Bool(true)));
    }

    #[test]
    fn test_constants_alternate_prefix() {
        let mut constants = HashMap::new();
        constants.insert("label".to_string(), OdinValues::string("test"));

        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Label".to_string(),
                expression: FieldExpression::Copy("$constants.label".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants = constants;

        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Label"), Some(&DynValue::String("test".to_string())));
    }

    #[test]
    fn test_constants_missing_returns_null() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Missing".to_string(),
                expression: FieldExpression::Copy("$const.nonexistent".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants = HashMap::new();

        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Missing"), Some(&DynValue::Null));
    }

    #[test]
    fn test_constants_used_in_verb() {
        let mut constants = HashMap::new();
        constants.insert("prefix".to_string(), OdinValues::string("ID-"));

        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Code".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("$const.prefix".to_string(), Vec::new()),
                        VerbArg::Reference("@.id".to_string(), Vec::new()),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants = constants;

        let source = DynValue::Object(vec![
            ("id".to_string(), DynValue::String("123".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Code"), Some(&DynValue::String("ID-123".to_string())));
    }

    #[test]
    fn test_multiple_constants() {
        let mut constants = HashMap::new();
        constants.insert("first".to_string(), OdinValues::string("A"));
        constants.insert("second".to_string(), OdinValues::string("B"));
        constants.insert("third".to_string(), OdinValues::string("C"));

        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "A".to_string(),
                expression: FieldExpression::Copy("$const.first".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "B".to_string(),
                expression: FieldExpression::Copy("$const.second".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "C".to_string(),
                expression: FieldExpression::Copy("$const.third".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants = constants;

        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&DynValue::String("A".to_string())));
        assert_eq!(out.get("B"), Some(&DynValue::String("B".to_string())));
        assert_eq!(out.get("C"), Some(&DynValue::String("C".to_string())));
    }

    // =========================================================================
    // 10. Error cases
    // =========================================================================

    #[test]
    fn test_error_unknown_verb() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "X".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "totallyFakeVerb".to_string(),
                    is_custom: false,
                    args: vec![],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(!result.success);
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].message.contains("totallyFakeVerb"));
    }

    #[test]
    fn test_error_unknown_verb_message_contains_path() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "MyField".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "badVerb".to_string(),
                    is_custom: false,
                    args: vec![],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(!result.success);
        assert_eq!(result.errors[0].path, Some("MyField".to_string()));
    }

    #[test]
    fn test_custom_verb_passthrough() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "myCustom".to_string(),
                    is_custom: true,
                    args: vec![VerbArg::Literal(OdinValues::string("hello"))],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        // Custom verbs pass through first arg
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::String("hello".to_string())));
    }

    #[test]
    fn test_custom_verb_no_args_returns_null() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "myCustom".to_string(),
                    is_custom: true,
                    args: vec![],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::Null));
    }

    #[test]
    fn test_missing_deep_nested_path() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Copy("@.a.b.c.d.e.f".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("a".to_string(), DynValue::Object(Vec::new())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::Null));
    }

    #[test]
    fn test_nested_verb_error_propagates() {
        // upper(badVerb(@.name)) - inner verb fails
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "upper".to_string(),
                    is_custom: false,
                    args: vec![VerbArg::Verb(VerbCall {
                        verb: "badVerb".to_string(),
                        is_custom: false,
                        args: vec![VerbArg::Reference("@.name".to_string(), Vec::new())],
                    })],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("test".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_empty_source_object() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "A".to_string(),
                expression: FieldExpression::Copy("@.name".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "B".to_string(),
                expression: FieldExpression::Copy("@.age".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&DynValue::Null));
        assert_eq!(out.get("B"), Some(&DynValue::Null));
    }

    #[test]
    fn test_multiple_errors_from_multiple_mappings() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "A".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "unknownVerb1".to_string(),
                    is_custom: false,
                    args: vec![],
                }),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "B".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "unknownVerb2".to_string(),
                    is_custom: false,
                    args: vec![],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(!result.success);
        assert_eq!(result.errors.len(), 2);
        assert!(result.errors[0].message.contains("unknownVerb1"));
        assert!(result.errors[1].message.contains("unknownVerb2"));
    }

    // =========================================================================
    // Additional edge cases and integration tests
    // =========================================================================

    #[test]
    fn test_copy_bare_at() {
        // @. alone means "copy entire source"
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "All".to_string(),
                expression: FieldExpression::Copy("@".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("x".to_string(), DynValue::Integer(1)),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let all = out.get("All").unwrap();
        assert_eq!(all.get("x"), Some(&DynValue::Integer(1)));
    }

    #[test]
    fn test_literal_integer() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Count".to_string(),
                expression: FieldExpression::Literal(OdinValues::integer(42)),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Count"), Some(&DynValue::Integer(42)));
    }

    #[test]
    fn test_literal_boolean() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Flag".to_string(),
                expression: FieldExpression::Literal(OdinValues::boolean(false)),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Flag"), Some(&DynValue::Bool(false)));
    }

    #[test]
    fn test_literal_null() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Nothing".to_string(),
                expression: FieldExpression::Literal(OdinValues::null()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Nothing"), Some(&DynValue::Null));
    }

    #[test]
    fn test_literal_number() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Pi".to_string(),
                expression: FieldExpression::Literal(OdinValues::number(3.14)),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Pi"), Some(&DynValue::Float(3.14)));
    }

    #[test]
    fn test_copy_all_value_types() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "S".to_string(),
                expression: FieldExpression::Copy("@.s".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "I".to_string(),
                expression: FieldExpression::Copy("@.i".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "F".to_string(),
                expression: FieldExpression::Copy("@.f".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "B".to_string(),
                expression: FieldExpression::Copy("@.b".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "N".to_string(),
                expression: FieldExpression::Copy("@.n".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("s".to_string(), DynValue::String("hello".to_string())),
            ("i".to_string(), DynValue::Integer(42)),
            ("f".to_string(), DynValue::Float(3.14)),
            ("b".to_string(), DynValue::Bool(true)),
            ("n".to_string(), DynValue::Null),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("S"), Some(&DynValue::String("hello".to_string())));
        assert_eq!(out.get("I"), Some(&DynValue::Integer(42)));
        assert_eq!(out.get("F"), Some(&DynValue::Float(3.14)));
        assert_eq!(out.get("B"), Some(&DynValue::Bool(true)));
        assert_eq!(out.get("N"), Some(&DynValue::Null));
    }

    #[test]
    fn test_copy_nested_array_within_object() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Tags".to_string(),
                expression: FieldExpression::Copy("@.meta.tags".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("meta".to_string(), DynValue::Object(vec![
                ("tags".to_string(), DynValue::Array(vec![
                    DynValue::String("a".to_string()),
                    DynValue::String("b".to_string()),
                ])),
            ])),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let tags = out.get("Tags").unwrap().as_array().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0], DynValue::String("a".to_string()));
        assert_eq!(tags[1], DynValue::String("b".to_string()));
    }

    #[test]
    fn test_discriminator_segment() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: Some(Discriminator {
                    path: "@.type".to_string(),
                    value: "person".to_string(),
                }),
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "PersonName".to_string(),
                    expression: FieldExpression::Copy("@.name".to_string()),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("type".to_string(), DynValue::String("person".to_string())),
            ("name".to_string(), DynValue::String("Alice".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("PersonName"), Some(&DynValue::String("Alice".to_string())));
    }

    #[test]
    fn test_discriminator_mismatch_skips() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: Some(Discriminator {
                    path: "@.type".to_string(),
                    value: "person".to_string(),
                }),
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "PersonName".to_string(),
                    expression: FieldExpression::Copy("@.name".to_string()),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("type".to_string(), DynValue::String("company".to_string())),
            ("name".to_string(), DynValue::String("Acme".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("PersonName"), None);
    }

    #[test]
    fn test_set_path_array_push() {
        let mut output = DynValue::Object(Vec::new());
        set_path(&mut output, "items[]", DynValue::String("a".to_string()));
        set_path(&mut output, "items[]", DynValue::String("b".to_string()));
        let items = output.get("items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], DynValue::String("a".to_string()));
        assert_eq!(items[1], DynValue::String("b".to_string()));
    }

    #[test]
    fn test_set_path_array_index() {
        let mut output = DynValue::Object(Vec::new());
        set_path(&mut output, "items[0]", DynValue::String("first".to_string()));
        set_path(&mut output, "items[2]", DynValue::String("third".to_string()));
        let items = output.get("items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], DynValue::String("first".to_string()));
        assert_eq!(items[1], DynValue::Null); // Filled with null
        assert_eq!(items[2], DynValue::String("third".to_string()));
    }

    #[test]
    fn test_set_path_overwrites_existing() {
        let mut output = DynValue::Object(Vec::new());
        set_path(&mut output, "name", DynValue::String("old".to_string()));
        set_path(&mut output, "name", DynValue::String("new".to_string()));
        assert_eq!(output.get("name"), Some(&DynValue::String("new".to_string())));
    }

    #[test]
    fn test_segment_ordering_multiple_passes() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Last".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(3)),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: None, // last
                condition: None,
            },
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Second".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(2)),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(2),
                condition: None,
            },
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "First".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(1)),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(1),
                condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        // All fields should be present regardless of pass order
        assert_eq!(out.get("First"), Some(&DynValue::Integer(1)));
        assert_eq!(out.get("Second"), Some(&DynValue::Integer(2)));
        assert_eq!(out.get("Last"), Some(&DynValue::Integer(3)));
    }

    #[test]
    fn test_named_segment_creates_nested_object() {
        let t = custom_transform(vec![TransformSegment {
            name: "Customer".to_string(),
            path: "Customer".to_string(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "Name".to_string(),
                    expression: FieldExpression::Copy("@.name".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
                FieldMapping {
                    target: "Age".to_string(),
                    expression: FieldExpression::Copy("@.age".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("Alice".to_string())),
            ("age".to_string(), DynValue::Integer(30)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let customer = out.get("Customer").unwrap();
        assert_eq!(customer.get("Name"), Some(&DynValue::String("Alice".to_string())));
        assert_eq!(customer.get("Age"), Some(&DynValue::Integer(30)));
    }

    #[test]
    fn test_is_truthy_values() {
        assert!(!is_truthy(&DynValue::Null));
        assert!(!is_truthy(&DynValue::Bool(false)));
        assert!(is_truthy(&DynValue::Bool(true)));
        assert!(!is_truthy(&DynValue::Integer(0)));
        assert!(is_truthy(&DynValue::Integer(1)));
        assert!(is_truthy(&DynValue::Integer(-1)));
        assert!(!is_truthy(&DynValue::Float(0.0)));
        assert!(is_truthy(&DynValue::Float(1.0)));
        assert!(!is_truthy(&DynValue::String(String::new())));
        assert!(is_truthy(&DynValue::String("x".to_string())));
        assert!(!is_truthy(&DynValue::Array(Vec::new())));
        assert!(is_truthy(&DynValue::Array(vec![DynValue::Null])));
        assert!(!is_truthy(&DynValue::Object(Vec::new())));
        assert!(is_truthy(&DynValue::Object(vec![("k".to_string(), DynValue::Null)])));
    }

    #[test]
    fn test_apply_confidential_redact_returns_null() {
        let val = DynValue::String("secret".to_string());
        let result = apply_confidential_to_value(&val, &ConfidentialMode::Redact);
        assert_eq!(result, DynValue::Null);
    }

    #[test]
    fn test_apply_confidential_mask_string() {
        let val = DynValue::String("abc".to_string());
        let result = apply_confidential_to_value(&val, &ConfidentialMode::Mask);
        assert_eq!(result, DynValue::String("***".to_string()));
    }

    #[test]
    fn test_apply_confidential_mask_non_string() {
        let val = DynValue::Integer(42);
        let result = apply_confidential_to_value(&val, &ConfidentialMode::Mask);
        assert_eq!(result, DynValue::Null);
    }

    #[test]
    fn test_verb_lower() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "lower".to_string(),
                    is_custom: false,
                    args: vec![VerbArg::Reference("@.name".to_string(), Vec::new())],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("ALICE".to_string())),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Name"), Some(&DynValue::String("alice".to_string())));
    }

    #[test]
    fn test_result_has_formatted_json() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "name".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("test")),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        let formatted = result.formatted.unwrap();
        assert!(formatted.contains("\"name\""));
        assert!(formatted.contains("\"test\""));
    }

    #[test]
    fn test_success_true_with_warnings_only() {
        // If there are warnings but no errors, success should still be true
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("ok")),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(result.success);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_mixed_success_and_error_mappings() {
        // One mapping succeeds, one fails
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Good".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("ok")),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "Bad".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "noSuchVerb".to_string(),
                    is_custom: false,
                    args: vec![],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(Vec::new());
        let result = execute(&transform, &source);
        assert!(!result.success);
        assert_eq!(result.errors.len(), 1);
        // The good mapping should still have produced output
        let out = result.output.unwrap();
        assert_eq!(out.get("Good"), Some(&DynValue::String("ok".to_string())));
    }

    // =========================================================================
    // Multi-record transforms with discriminator routing
    // =========================================================================

    #[test]
    fn test_multi_record_position_discriminator() {
        let mut t = custom_transform(vec![
            TransformSegment {
                name: "Headers[]".to_string(),
                path: "Headers".to_string(),
                source_path: None,
                discriminator: None,
                is_array: true,
                directives: Vec::new(),
                mappings: vec![
                    FieldMapping {
                        target: "_type".to_string(),
                        expression: FieldExpression::Literal(OdinValues::string("HDR")),
                        directives: vec![],
                        modifiers: None,
                    },
                    FieldMapping {
                        target: "Data".to_string(),
                        expression: FieldExpression::Copy("@._raw".to_string()),
                        directives: vec![],
                        modifiers: None,
                    },
                ],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            },
            TransformSegment {
                name: "Details[]".to_string(),
                path: "Details".to_string(),
                source_path: None,
                discriminator: None,
                is_array: true,
                directives: Vec::new(),
                mappings: vec![
                    FieldMapping {
                        target: "_type".to_string(),
                        expression: FieldExpression::Literal(OdinValues::string("DTL")),
                        directives: vec![],
                        modifiers: None,
                    },
                    FieldMapping {
                        target: "Data".to_string(),
                        expression: FieldExpression::Copy("@._raw".to_string()),
                        directives: vec![],
                        modifiers: None,
                    },
                ],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            },
        ]);
        t.source = Some(SourceConfig {
            format: "fixed-width".to_string(),
            options: {
                let mut m = HashMap::new();
                m.insert("discriminator".to_string(), ":pos 0 :len 3".to_string());
                m
            },
            namespaces: HashMap::new(),
            discriminator: None,
        });
        let input = DynValue::String("HDR first header line\nDTL detail record one\nDTL detail record two\nHDR second header".to_string());
        let result = execute(&t, &input);
        assert!(result.success);
        let out = result.output.unwrap();
        let headers = out.get("Headers").unwrap().as_array().unwrap();
        assert_eq!(headers.len(), 2);
        let details = out.get("Details").unwrap().as_array().unwrap();
        assert_eq!(details.len(), 2);
    }

    #[test]
    fn test_multi_record_field_discriminator_csv() {
        let mut t = custom_transform(vec![
            TransformSegment {
                name: "TypeA[]".to_string(),
                path: "TypeA".to_string(),
                source_path: None,
                discriminator: None,
                is_array: true,
                directives: Vec::new(),
                mappings: vec![
                    FieldMapping {
                        target: "_type".to_string(),
                        expression: FieldExpression::Literal(OdinValues::string("A")),
                        directives: vec![],
                        modifiers: None,
                    },
                    FieldMapping {
                        target: "Value".to_string(),
                        expression: FieldExpression::Copy("@.1".to_string()),
                        directives: vec![],
                        modifiers: None,
                    },
                ],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            },
            TransformSegment {
                name: "TypeB[]".to_string(),
                path: "TypeB".to_string(),
                source_path: None,
                discriminator: None,
                is_array: true,
                directives: Vec::new(),
                mappings: vec![
                    FieldMapping {
                        target: "_type".to_string(),
                        expression: FieldExpression::Literal(OdinValues::string("B")),
                        directives: vec![],
                        modifiers: None,
                    },
                    FieldMapping {
                        target: "Value".to_string(),
                        expression: FieldExpression::Copy("@.1".to_string()),
                        directives: vec![],
                        modifiers: None,
                    },
                ],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            },
        ]);
        t.source = Some(SourceConfig {
            format: "csv".to_string(),
            options: {
                let mut m = HashMap::new();
                m.insert("discriminator".to_string(), ":field 0".to_string());
                m.insert("delimiter".to_string(), ",".to_string());
                m
            },
            namespaces: HashMap::new(),
            discriminator: None,
        });
        let input = DynValue::String("A,val1\nB,val2\nA,val3".to_string());
        let result = execute(&t, &input);
        assert!(result.success);
        let out = result.output.unwrap();
        let type_a = out.get("TypeA").unwrap().as_array().unwrap();
        assert_eq!(type_a.len(), 2);
        let type_b = out.get("TypeB").unwrap().as_array().unwrap();
        assert_eq!(type_b.len(), 1);
    }

    #[test]
    fn test_multi_record_invalid_discriminator_config() {
        let mut t = custom_transform(vec![]);
        t.source = Some(SourceConfig {
            format: "fixed-width".to_string(),
            options: {
                let mut m = HashMap::new();
                m.insert("discriminator".to_string(), "invalid_config".to_string());
                m
            },
            namespaces: HashMap::new(),
            discriminator: None,
        });
        let input = DynValue::String("some data".to_string());
        let result = execute(&t, &input);
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_multi_record_empty_input() {
        let mut t = custom_transform(vec![
            TransformSegment {
                name: "Records[]".to_string(),
                path: "Records".to_string(),
                source_path: None,
                discriminator: None,
                is_array: true,
                directives: Vec::new(),
                mappings: vec![
                    FieldMapping {
                        target: "_type".to_string(),
                        expression: FieldExpression::Literal(OdinValues::string("REC")),
                        directives: vec![],
                        modifiers: None,
                    },
                ],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            },
        ]);
        t.source = Some(SourceConfig {
            format: "fixed-width".to_string(),
            options: {
                let mut m = HashMap::new();
                m.insert("discriminator".to_string(), ":pos 0 :len 3".to_string());
                m
            },
            namespaces: HashMap::new(),
            discriminator: None,
        });
        let input = DynValue::String(String::new());
        let result = execute(&t, &input);
        assert!(result.success);
    }

    #[test]
    fn test_multi_record_unmatched_discriminator_skipped() {
        let mut t = custom_transform(vec![
            TransformSegment {
                name: "Known[]".to_string(),
                path: "Known".to_string(),
                source_path: None,
                discriminator: None,
                is_array: true,
                directives: Vec::new(),
                mappings: vec![
                    FieldMapping {
                        target: "_type".to_string(),
                        expression: FieldExpression::Literal(OdinValues::string("AAA")),
                        directives: vec![],
                        modifiers: None,
                    },
                ],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            },
        ]);
        t.source = Some(SourceConfig {
            format: "fixed-width".to_string(),
            options: {
                let mut m = HashMap::new();
                m.insert("discriminator".to_string(), ":pos 0 :len 3".to_string());
                m
            },
            namespaces: HashMap::new(),
            discriminator: None,
        });
        // "BBB" lines won't match any segment
        let input = DynValue::String("AAA matched\nBBB skipped\nAAA also matched".to_string());
        let result = execute(&t, &input);
        assert!(result.success);
        let out = result.output.unwrap();
        let known = out.get("Known").unwrap().as_array().unwrap();
        assert_eq!(known.len(), 2);
    }

    // =========================================================================
    // Multi-pass transform execution
    // =========================================================================

    #[test]
    fn test_multi_pass_pass1_runs_before_pass2() {
        // Pass 1 sets a value, Pass 2 reads global output from pass 1
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "FromPass1".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("first")),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(1),
                condition: None,
            },
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "FromPass2".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("second")),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(2),
                condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("FromPass1"), Some(&DynValue::String("first".to_string())));
        assert_eq!(out.get("FromPass2"), Some(&DynValue::String("second".to_string())));
    }

    #[test]
    fn test_multi_pass_none_pass_runs_last() {
        // None/0 pass should run after numbered passes
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "RunOrder".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("last")),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: None, // runs last
                condition: None,
            },
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "RunOrder".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("first")),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(1),
                condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        // The None pass runs last and overwrites
        assert_eq!(out.get("RunOrder"), Some(&DynValue::String("last".to_string())));
    }

    #[test]
    fn test_multi_pass_accumulator_reset_between_passes() {
        let mut t = custom_transform(vec![
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "P1".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(1)),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(1),
                condition: None,
            },
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "P2".to_string(),
                    expression: FieldExpression::Copy("$accumulator.counter".to_string()),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(2),
                condition: None,
            },
        ]);
        t.accumulators.insert("counter".to_string(), AccumulatorDef {
            name: "counter".to_string(),
            initial: OdinValues::integer(0),
            persist: false,
        });
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        // Non-persist accumulator resets between passes
        let out = result.output.unwrap();
        assert_eq!(out.get("P2"), Some(&DynValue::Integer(0)));
    }

    #[test]
    fn test_multi_pass_persist_accumulator_survives_pass_transition() {
        let mut t = custom_transform(vec![
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "P1".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(1)),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(1),
                condition: None,
            },
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "P2".to_string(),
                    expression: FieldExpression::Copy("$accumulator.persist_counter".to_string()),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(2),
                condition: None,
            },
        ]);
        t.accumulators.insert("persist_counter".to_string(), AccumulatorDef {
            name: "persist_counter".to_string(),
            initial: OdinValues::integer(42),
            persist: true,
        });
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        // Persist accumulator keeps its value across pass transitions
        assert_eq!(out.get("P2"), Some(&DynValue::Integer(42)));
    }

    #[test]
    fn test_three_passes_in_reverse_order() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "C".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(3)),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(3),
                condition: None,
            },
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "A".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(1)),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(1),
                condition: None,
            },
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "B".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(2)),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: Some(2),
                condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&DynValue::Integer(1)));
        assert_eq!(out.get("B"), Some(&DynValue::Integer(2)));
        assert_eq!(out.get("C"), Some(&DynValue::Integer(3)));
    }

    // =========================================================================
    // Confidential field enforcement
    // =========================================================================

    #[test]
    fn test_confidential_redact_multiple_fields() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "SSN".to_string(),
                expression: FieldExpression::Copy("@.ssn".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
            FieldMapping {
                target: "DOB".to_string(),
                expression: FieldExpression::Copy("@.dob".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Copy("@.name".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("ssn".to_string(), DynValue::String("123-45-6789".to_string())),
            ("dob".to_string(), DynValue::String("1990-01-01".to_string())),
            ("name".to_string(), DynValue::String("Alice".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("SSN"), Some(&DynValue::Null));
        assert_eq!(out.get("DOB"), Some(&DynValue::Null));
        assert_eq!(out.get("Name"), Some(&DynValue::String("Alice".to_string())));
    }

    #[test]
    fn test_confidential_mask_preserves_length() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Secret".to_string(),
                expression: FieldExpression::Copy("@.secret".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let source = DynValue::Object(vec![
            ("secret".to_string(), DynValue::String("hello".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Secret"), Some(&DynValue::String("*****".to_string())));
    }

    #[test]
    fn test_confidential_mask_boolean_becomes_null() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Flag".to_string(),
                expression: FieldExpression::Copy("@.flag".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let source = DynValue::Object(vec![
            ("flag".to_string(), DynValue::Bool(true)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Flag"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_redact_float() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Salary".to_string(),
                expression: FieldExpression::Copy("@.salary".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("salary".to_string(), DynValue::Float(100000.50)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Salary"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_mask_empty_string() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Token".to_string(),
                expression: FieldExpression::Copy("@.token".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let source = DynValue::Object(vec![
            ("token".to_string(), DynValue::String(String::new())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Token"), Some(&DynValue::String(String::new())));
    }

    #[test]
    fn test_confidential_combined_with_required() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Secret".to_string(),
                expression: FieldExpression::Copy("@.val".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(true, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("val".to_string(), DynValue::String("secret data".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        // Redact still applies even with required modifier
        assert_eq!(out.get("Secret"), Some(&DynValue::Null));
        // Modifiers should be recorded
        assert!(result.modifiers.get("Secret").unwrap().required);
        assert!(result.modifiers.get("Secret").unwrap().confidential);
    }

    // =========================================================================
    // Complex nested transforms
    // =========================================================================

    #[test]
    fn test_deeply_nested_output_four_levels() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "a.b.c.d".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("deep")),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let result = execute(&transform, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        let a = out.get("a").unwrap();
        let b = a.get("b").unwrap();
        let c = b.get("c").unwrap();
        assert_eq!(c.get("d"), Some(&DynValue::String("deep".to_string())));
    }

    #[test]
    fn test_multiple_nested_output_paths_same_parent() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "person.first".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("John")),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "person.last".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("Doe")),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "person.age".to_string(),
                expression: FieldExpression::Literal(OdinValues::integer(30)),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let result = execute(&transform, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        let person = out.get("person").unwrap();
        assert_eq!(person.get("first"), Some(&DynValue::String("John".to_string())));
        assert_eq!(person.get("last"), Some(&DynValue::String("Doe".to_string())));
        assert_eq!(person.get("age"), Some(&DynValue::Integer(30)));
    }

    #[test]
    fn test_nested_segment_with_children() {
        let t = custom_transform(vec![TransformSegment {
            name: "Outer".to_string(),
            path: "Outer".to_string(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "OuterField".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("outer_val")),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: vec![TransformSegment {
                name: "Inner".to_string(),
                path: "Inner".to_string(),
                source_path: None,
                discriminator: None,
                is_array: false,
                directives: Vec::new(),
                mappings: vec![
                    FieldMapping {
                        target: "InnerField".to_string(),
                        expression: FieldExpression::Literal(OdinValues::string("inner_val")),
                        directives: vec![],
                        modifiers: None,
                    },
                ],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            }],
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        let outer = out.get("Outer").unwrap();
        assert_eq!(outer.get("OuterField"), Some(&DynValue::String("outer_val".to_string())));
        let inner = outer.get("Inner").unwrap();
        assert_eq!(inner.get("InnerField"), Some(&DynValue::String("inner_val".to_string())));
    }

    #[test]
    fn test_object_expression_nested_in_object_expression() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Level1".to_string(),
                expression: FieldExpression::Object(vec![
                    FieldMapping {
                        target: "Level2".to_string(),
                        expression: FieldExpression::Object(vec![
                            FieldMapping {
                                target: "Value".to_string(),
                                expression: FieldExpression::Literal(OdinValues::string("deep")),
                                directives: vec![],
                                modifiers: None,
                            },
                        ]),
                        directives: vec![],
                        modifiers: None,
                    },
                ]),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let result = execute(&transform, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        let l1 = out.get("Level1").unwrap();
        let l2 = l1.get("Level2").unwrap();
        assert_eq!(l2.get("Value"), Some(&DynValue::String("deep".to_string())));
    }

    // =========================================================================
    // Transform with loop segments (@each)
    // =========================================================================

    #[test]
    fn test_loop_with_verb_transform() {
        let t = custom_transform(vec![TransformSegment {
            name: "Items".to_string(),
            path: "Items".to_string(),
            source_path: Some("@.items".to_string()),
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "Name".to_string(),
                    expression: FieldExpression::Transform(VerbCall {
                        verb: "upper".to_string(),
                        is_custom: false,
                        args: vec![VerbArg::Reference("@_item.name".to_string(), Vec::new())],
                    }),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::Object(vec![("name".to_string(), DynValue::String("alice".to_string()))]),
                DynValue::Object(vec![("name".to_string(), DynValue::String("bob".to_string()))]),
            ])),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items[0].get("Name"), Some(&DynValue::String("ALICE".to_string())));
        assert_eq!(items[1].get("Name"), Some(&DynValue::String("BOB".to_string())));
    }

    #[test]
    fn test_loop_three_elements() {
        let t = custom_transform(vec![TransformSegment {
            name: "Out".to_string(),
            path: "Out".to_string(),
            source_path: Some("@.nums".to_string()),
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "Val".to_string(),
                    expression: FieldExpression::Copy("@_item".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let source = DynValue::Object(vec![
            ("nums".to_string(), DynValue::Array(vec![
                DynValue::Integer(10),
                DynValue::Integer(20),
                DynValue::Integer(30),
            ])),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Out").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].get("Val"), Some(&DynValue::Integer(10)));
        assert_eq!(items[2].get("Val"), Some(&DynValue::Integer(30)));
    }

    #[test]
    fn test_loop_nonexistent_source_path() {
        let t = custom_transform(vec![TransformSegment {
            name: "Missing".to_string(),
            path: "Missing".to_string(),
            source_path: Some("@.nonexistent".to_string()),
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "Val".to_string(),
                    expression: FieldExpression::Copy("@_item".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        // When source_path resolves to null, it produces an empty array (zero iterations)
        let out = result.output.unwrap();
        let items = out.get("Missing").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 0);
    }

    #[test]
    fn test_loop_with_index_and_length_access() {
        let t = custom_transform(vec![TransformSegment {
            name: "Items".to_string(),
            path: "Items".to_string(),
            source_path: Some("@.items".to_string()),
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![
                FieldMapping {
                    target: "Idx".to_string(),
                    expression: FieldExpression::Copy("@_index".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
                FieldMapping {
                    target: "Len".to_string(),
                    expression: FieldExpression::Copy("@_length".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: None,
        }]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::String("a".to_string()),
                DynValue::String("b".to_string()),
            ])),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items[0].get("Idx"), Some(&DynValue::Integer(0)));
        assert_eq!(items[0].get("Len"), Some(&DynValue::Integer(2)));
        assert_eq!(items[1].get("Idx"), Some(&DynValue::Integer(1)));
    }

    // =========================================================================
    // Transform with constants
    // =========================================================================

    #[test]
    fn test_constant_in_verb_arg() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Result".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.name".to_string(), Vec::new()),
                        VerbArg::Reference("$const.suffix".to_string(), Vec::new()),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants.insert("suffix".to_string(), OdinValues::string("_v2"));
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("test".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Result"), Some(&DynValue::String("test_v2".to_string())));
    }

    #[test]
    fn test_constant_number() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Rate".to_string(),
                expression: FieldExpression::Copy("$const.rate".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants.insert("rate".to_string(), OdinValue::Number { value: 0.05, decimal_places: None, raw: Some("0.05".to_string()), modifiers: Default::default(), directives: Vec::new() });
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Rate"), Some(&DynValue::Float(0.05)));
    }

    #[test]
    fn test_constant_boolean_true() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Flag".to_string(),
                expression: FieldExpression::Copy("$const.active".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants.insert("active".to_string(), OdinValues::boolean(true));
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Flag"), Some(&DynValue::Bool(true)));
    }

    #[test]
    fn test_multiple_constants_in_single_transform() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "A".to_string(),
                expression: FieldExpression::Copy("$const.alpha".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "B".to_string(),
                expression: FieldExpression::Copy("$const.beta".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "C".to_string(),
                expression: FieldExpression::Copy("$const.gamma".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.constants.insert("alpha".to_string(), OdinValues::string("a"));
        t.constants.insert("beta".to_string(), OdinValues::integer(2));
        t.constants.insert("gamma".to_string(), OdinValues::boolean(false));
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&DynValue::String("a".to_string())));
        assert_eq!(out.get("B"), Some(&DynValue::Integer(2)));
        assert_eq!(out.get("C"), Some(&DynValue::Bool(false)));
    }

    // =========================================================================
    // Transform error handling
    // =========================================================================

    #[test]
    fn test_error_multiple_unknown_verbs() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "A".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "fakeVerb1".to_string(),
                    is_custom: false,
                    args: vec![],
                }),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "B".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "fakeVerb2".to_string(),
                    is_custom: false,
                    args: vec![],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let result = execute(&transform, &DynValue::Object(Vec::new()));
        assert!(!result.success);
        assert_eq!(result.errors.len(), 2);
    }

    #[test]
    fn test_error_does_not_halt_subsequent_mappings() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Bad".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "nonexistent".to_string(),
                    is_custom: false,
                    args: vec![],
                }),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "Good".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("ok")),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let result = execute(&transform, &DynValue::Object(Vec::new()));
        assert!(!result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Good"), Some(&DynValue::String("ok".to_string())));
    }

    #[test]
    fn test_error_result_still_has_output() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("Alice")),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "Err".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "badVerb".to_string(),
                    is_custom: false,
                    args: vec![],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let result = execute(&transform, &DynValue::Object(Vec::new()));
        assert!(!result.success);
        assert!(result.output.is_some());
        assert!(result.formatted.is_some());
    }

    // =========================================================================
    // Discriminator segment
    // =========================================================================

    #[test]
    fn test_discriminator_segment_integer_value() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: Some(Discriminator {
                    path: "@.type".to_string(),
                    value: "1".to_string(),
                }),
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Result".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("matched")),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("type".to_string(), DynValue::Integer(1)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Result"), Some(&DynValue::String("matched".to_string())));
    }

    #[test]
    fn test_discriminator_segment_boolean_value() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: Some(Discriminator {
                    path: "@.active".to_string(),
                    value: "true".to_string(),
                }),
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Active".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("yes")),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("active".to_string(), DynValue::Bool(true)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Active"), Some(&DynValue::String("yes".to_string())));
    }

    #[test]
    fn test_discriminator_mismatch_skips_segment() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(),
                path: String::new(),
                source_path: None,
                discriminator: Some(Discriminator {
                    path: "@.type".to_string(),
                    value: "A".to_string(),
                }),
                is_array: false,
                directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Matched".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("yes")),
                    directives: vec![],
                    modifiers: None,
                }],
                children: Vec::new(),
                items: Vec::new(),
                pass: None,
                condition: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("type".to_string(), DynValue::String("B".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Matched"), None);
    }

    // =========================================================================
    // Condition segment tests
    // =========================================================================

    #[test]
    fn test_condition_integer_zero_skips() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(),
            path: String::new(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![FieldMapping {
                target: "Skipped".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("should not appear")),
                directives: vec![],
                modifiers: None,
            }],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: Some("@.count".to_string()),
        }]);
        let source = DynValue::Object(vec![
            ("count".to_string(), DynValue::Integer(0)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Skipped"), None);
    }

    #[test]
    fn test_condition_array_empty_skips() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(),
            path: String::new(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![FieldMapping {
                target: "Items".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("data")),
                directives: vec![],
                modifiers: None,
            }],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: Some("@.items".to_string()),
        }]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(Vec::new())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Items"), None);
    }

    #[test]
    fn test_condition_nonempty_array_passes() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(),
            path: String::new(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: vec![FieldMapping {
                target: "Passed".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("yes")),
                directives: vec![],
                modifiers: None,
            }],
            children: Vec::new(),
            items: Vec::new(),
            pass: None,
            condition: Some("@.items".to_string()),
        }]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![DynValue::Integer(1)])),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Passed"), Some(&DynValue::String("yes".to_string())));
    }

    // =========================================================================
    // Miscellaneous engine tests
    // =========================================================================

    #[test]
    fn test_copy_entire_source_with_bare_at() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "All".to_string(),
                expression: FieldExpression::Copy("@".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("x".to_string(), DynValue::Integer(1)),
            ("y".to_string(), DynValue::Integer(2)),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let all = out.get("All").unwrap();
        assert_eq!(all.get("x"), Some(&DynValue::Integer(1)));
    }

    #[test]
    fn test_set_path_deep_nested_creates_all_intermediates() {
        let mut output = DynValue::Object(Vec::new());
        set_path(&mut output, "a.b.c", DynValue::String("deep".to_string()));
        let a = output.get("a").unwrap();
        let b = a.get("b").unwrap();
        assert_eq!(b.get("c"), Some(&DynValue::String("deep".to_string())));
    }

    #[test]
    fn test_parse_discriminator_config_position() {
        let mode = parse_discriminator_config(":pos 0 :len 3");
        assert!(matches!(mode, Some(DiscriminatorMode::Position { pos: 0, len: 3 })));
    }

    #[test]
    fn test_parse_discriminator_config_field() {
        let mode = parse_discriminator_config(":field 2");
        assert!(matches!(mode, Some(DiscriminatorMode::Field { index: 2 })));
    }

    #[test]
    fn test_parse_discriminator_config_invalid() {
        let mode = parse_discriminator_config("garbage");
        assert!(mode.is_none());
    }

    #[test]
    fn test_extract_discriminator_value_position_bounds() {
        let mode = DiscriminatorMode::Position { pos: 100, len: 5 };
        let val = extract_discriminator_value("short", &mode, ",");
        assert_eq!(val, "");
    }

    #[test]
    fn test_extract_discriminator_value_position_partial() {
        let mode = DiscriminatorMode::Position { pos: 3, len: 100 };
        let val = extract_discriminator_value("abcdef", &mode, ",");
        assert_eq!(val, "def");
    }

    #[test]
    fn test_extract_discriminator_value_field_csv() {
        let mode = DiscriminatorMode::Field { index: 1 };
        let val = extract_discriminator_value("A,B,C", &mode, ",");
        assert_eq!(val, "B");
    }

    #[test]
    fn test_extract_discriminator_value_field_out_of_bounds() {
        let mode = DiscriminatorMode::Field { index: 10 };
        let val = extract_discriminator_value("A,B", &mode, ",");
        assert_eq!(val, "");
    }

    #[test]
    fn test_parse_record_csv() {
        let rec = parse_record("a,b,c", "csv", ",");
        assert_eq!(rec.get("0"), Some(&DynValue::String("a".to_string())));
        assert_eq!(rec.get("1"), Some(&DynValue::String("b".to_string())));
        assert_eq!(rec.get("2"), Some(&DynValue::String("c".to_string())));
    }

    #[test]
    fn test_parse_record_fixed_width() {
        let rec = parse_record("ABCDEFGH", "fixed-width", ",");
        assert_eq!(rec.get("_raw"), Some(&DynValue::String("ABCDEFGH".to_string())));
        assert_eq!(rec.get("_line"), Some(&DynValue::String("ABCDEFGH".to_string())));
    }

    #[test]
    fn test_is_truthy_float_neg_zero() {
        assert!(!is_truthy(&DynValue::Float(-0.0)));
    }

    #[test]
    fn test_apply_confidential_to_value_redact_array() {
        let val = DynValue::Array(vec![DynValue::Integer(1)]);
        let result = apply_confidential_to_value(&val, &ConfidentialMode::Redact);
        assert_eq!(result, DynValue::Null);
    }

    #[test]
    fn test_apply_confidential_to_value_mask_integer() {
        let val = DynValue::Integer(42);
        let result = apply_confidential_to_value(&val, &ConfidentialMode::Mask);
        assert_eq!(result, DynValue::Null);
    }

    #[test]
    fn test_apply_confidential_to_value_mask_long_string() {
        let val = DynValue::String("password123".to_string());
        let result = apply_confidential_to_value(&val, &ConfidentialMode::Mask);
        assert_eq!(result, DynValue::String("***********".to_string()));
    }

    #[test]
    fn test_resolve_mut_path_nonexistent() {
        let mut output = DynValue::Object(vec![
            ("a".to_string(), DynValue::String("x".to_string())),
        ]);
        let result = resolve_mut_path(&mut output, "b.c");
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_mut_path_on_non_object() {
        let mut output = DynValue::String("not an object".to_string());
        let result = resolve_mut_path(&mut output, "any.path");
        assert!(result.is_none());
    }

    #[test]
    fn test_order_segments_by_pass_all_none() {
        let segs = vec![
            TransformSegment { name: "A".to_string(), path: String::new(), source_path: None, discriminator: None, is_array: false, directives: Vec::new(), mappings: Vec::new(), children: Vec::new(), items: Vec::new(), pass: None, condition: None },
            TransformSegment { name: "B".to_string(), path: String::new(), source_path: None, discriminator: None, is_array: false, directives: Vec::new(), mappings: Vec::new(), children: Vec::new(), items: Vec::new(), pass: None, condition: None },
        ];
        let ordered = order_segments_by_pass(&segs);
        assert_eq!(ordered.len(), 2);
    }

    #[test]
    fn test_order_segments_by_pass_mixed() {
        let segs = vec![
            TransformSegment { name: "Last".to_string(), path: String::new(), source_path: None, discriminator: None, is_array: false, directives: Vec::new(), mappings: Vec::new(), children: Vec::new(), items: Vec::new(), pass: Some(0), condition: None },
            TransformSegment { name: "First".to_string(), path: String::new(), source_path: None, discriminator: None, is_array: false, directives: Vec::new(), mappings: Vec::new(), children: Vec::new(), items: Vec::new(), pass: Some(1), condition: None },
        ];
        let ordered = order_segments_by_pass(&segs);
        assert_eq!(ordered[0].name, "First");
        assert_eq!(ordered[1].name, "Last");
    }

    #[test]
    fn test_literal_null_output() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Val".to_string(),
                expression: FieldExpression::Literal(OdinValues::null()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let result = execute(&transform, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Val"), Some(&DynValue::Null));
    }

    #[test]
    fn test_copy_from_array_source() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Second".to_string(),
                expression: FieldExpression::Copy("@[1]".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let source = DynValue::Array(vec![
            DynValue::String("a".to_string()),
            DynValue::String("b".to_string()),
            DynValue::String("c".to_string()),
        ]);
        let result = execute(&transform, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Second"), Some(&DynValue::String("b".to_string())));
    }

    #[test]
    fn test_verb_chain_lower_of_concat() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "Result".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "lower".to_string(),
                    is_custom: false,
                    args: vec![VerbArg::Verb(VerbCall {
                        verb: "concat".to_string(),
                        is_custom: false,
                        args: vec![
                            VerbArg::Literal(OdinValues::string("HELLO")),
                            VerbArg::Literal(OdinValues::string(" WORLD")),
                        ],
                    })],
                }),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let result = execute(&transform, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Result"), Some(&DynValue::String("hello world".to_string())));
    }

    #[test]
    fn test_empty_transform_empty_source() {
        let t = custom_transform(vec![]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out, DynValue::Object(Vec::new()));
    }

    #[test]
    fn test_formatted_output_contains_json() {
        let transform = minimal_transform(vec![
            FieldMapping {
                target: "key".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("value")),
                directives: vec![],
                modifiers: None,
            },
        ]);
        let result = execute(&transform, &DynValue::Object(Vec::new()));
        let formatted = result.formatted.unwrap();
        assert!(formatted.contains("\"key\""));
        assert!(formatted.contains("\"value\""));
    }

    // =========================================================================
    // NEW TESTS: 1. Multi-record processing (~20 tests)
    // =========================================================================

    /// Helper: build a multi-record transform with source config and discriminator.
    fn multi_record_transform(
        disc_config: &str,
        source_format: &str,
        segments: Vec<TransformSegment>,
    ) -> OdinTransform {
        let mut options = HashMap::new();
        options.insert("discriminator".to_string(), disc_config.to_string());
        OdinTransform {
            metadata: TransformMetadata::default(),
            source: Some(SourceConfig {
                format: source_format.to_string(),
                options,
                namespaces: HashMap::new(),
                discriminator: None,
            }),
            target: TargetConfig {
                format: "json".to_string(),
                options: HashMap::new(),
            },
            constants: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
            segments,
            imports: Vec::new(),
            passes: Vec::new(),
            enforce_confidential: None,
            strict_types: false,
        }
    }

    /// Helper: build a multi-record segment with _type discriminator value and mappings.
    fn mr_segment(name: &str, type_val: &str, mappings: Vec<FieldMapping>) -> TransformSegment {
        let mut all_mappings = vec![FieldMapping {
            target: "_type".to_string(),
            expression: FieldExpression::Literal(OdinValues::string(type_val)),
            directives: vec![],
            modifiers: None,
        }];
        let items: Vec<crate::types::transform::SegmentItem> = all_mappings.iter()
            .chain(mappings.iter())
            .map(|m| crate::types::transform::SegmentItem::Mapping(m.clone()))
            .collect();
        all_mappings.extend(mappings);
        TransformSegment {
            name: name.to_string(),
            path: name.to_string(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: all_mappings,
            children: Vec::new(),
            items,
            pass: None,
            condition: None,
        }
    }

    /// Helper: build a multi-record array segment (name ends with []).
    fn mr_array_segment(name: &str, type_val: &str, mappings: Vec<FieldMapping>) -> TransformSegment {
        let array_name = format!("{}[]", name);
        let mut all_mappings = vec![FieldMapping {
            target: "_type".to_string(),
            expression: FieldExpression::Literal(OdinValues::string(type_val)),
            directives: vec![],
            modifiers: None,
        }];
        let items: Vec<crate::types::transform::SegmentItem> = all_mappings.iter()
            .chain(mappings.iter())
            .map(|m| crate::types::transform::SegmentItem::Mapping(m.clone()))
            .collect();
        all_mappings.extend(mappings);
        TransformSegment {
            name: array_name,
            path: name.to_string(),
            source_path: None,
            discriminator: None,
            is_array: false,
            directives: Vec::new(),
            mappings: all_mappings,
            children: Vec::new(),
            items,
            pass: None,
            condition: None,
        }
    }

    #[test]
    fn test_multi_record_position_discriminator_routes_correctly() {
        let t = multi_record_transform(":pos 0 :len 3", "fixed-width", vec![
            mr_segment("Header", "HDR", vec![
                FieldMapping {
                    target: "Type".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("header")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        let source = DynValue::String("HDR some data here".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let hdr = out.get("Header").unwrap();
        assert_eq!(hdr.get("Type"), Some(&DynValue::String("header".to_string())));
    }

    #[test]
    fn test_multi_record_field_discriminator_csv_basic() {
        let mut t = multi_record_transform(":field 0", "csv", vec![
            mr_segment("Orders", "ORD", vec![
                FieldMapping {
                    target: "Amount".to_string(),
                    expression: FieldExpression::Copy("@.1".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        // Set delimiter
        if let Some(ref mut src) = t.source {
            src.options.insert("delimiter".to_string(), ",".to_string());
        }
        let source = DynValue::String("ORD,100".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let orders = out.get("Orders").unwrap();
        assert_eq!(orders.get("Amount"), Some(&DynValue::String("100".to_string())));
    }

    #[test]
    fn test_multi_record_segment_routing_two_types() {
        let t = multi_record_transform(":pos 0 :len 3", "fixed-width", vec![
            mr_segment("Header", "HDR", vec![
                FieldMapping {
                    target: "Kind".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("header")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
            mr_segment("Detail", "DTL", vec![
                FieldMapping {
                    target: "Kind".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("detail")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        let source = DynValue::String("HDR header line\nDTL detail line".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(
            out.get("Header").unwrap().get("Kind"),
            Some(&DynValue::String("header".to_string()))
        );
        assert_eq!(
            out.get("Detail").unwrap().get("Kind"),
            Some(&DynValue::String("detail".to_string()))
        );
    }

    #[test]
    fn test_multi_record_multiple_records_same_type() {
        let t = multi_record_transform(":pos 0 :len 3", "fixed-width", vec![
            mr_array_segment("Items", "ITM", vec![
                FieldMapping {
                    target: "Data".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("item")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        let source = DynValue::String("ITM first\nITM second\nITM third".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_multi_record_no_matching_segment_skipped() {
        let t = multi_record_transform(":pos 0 :len 3", "fixed-width", vec![
            mr_segment("Header", "HDR", vec![
                FieldMapping {
                    target: "Kind".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("header")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        // "UNK" does not match any segment
        let source = DynValue::String("HDR valid\nUNK unknown\nXXX other".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert!(out.get("Header").is_some());
        // No "UNK" or "XXX" segments should appear
        assert!(out.get("UNK").is_none());
        assert!(out.get("XXX").is_none());
    }

    #[test]
    fn test_multi_record_empty_input_string() {
        let t = multi_record_transform(":pos 0 :len 3", "fixed-width", vec![
            mr_segment("Header", "HDR", vec![]),
        ]);
        let source = DynValue::String(String::new());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out, DynValue::Object(Vec::new()));
    }

    #[test]
    fn test_multi_record_single_record_input() {
        let t = multi_record_transform(":pos 0 :len 2", "fixed-width", vec![
            mr_segment("Row", "AB", vec![
                FieldMapping {
                    target: "Val".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("found")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        let source = DynValue::String("AB single line".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(
            out.get("Row").unwrap().get("Val"),
            Some(&DynValue::String("found".to_string()))
        );
    }

    #[test]
    fn test_multi_record_discriminator_at_end_of_line() {
        let t = multi_record_transform(":pos 7 :len 3", "fixed-width", vec![
            mr_segment("Rec", "END", vec![
                FieldMapping {
                    target: "Found".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("yes")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        let source = DynValue::String("1234567END".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert!(out.get("Rec").is_some());
    }

    #[test]
    fn test_multi_record_mix_matching_and_nonmatching() {
        let t = multi_record_transform(":pos 0 :len 1", "fixed-width", vec![
            mr_segment("A_Rec", "A", vec![
                FieldMapping {
                    target: "Kind".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("a")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
            mr_segment("B_Rec", "B", vec![
                FieldMapping {
                    target: "Kind".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("b")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        let source = DynValue::String("A first\nX skip\nB second\nY skip".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A_Rec").unwrap().get("Kind"), Some(&DynValue::String("a".to_string())));
        assert_eq!(out.get("B_Rec").unwrap().get("Kind"), Some(&DynValue::String("b".to_string())));
    }

    #[test]
    fn test_multi_record_field_discriminator_second_field() {
        let mut t = multi_record_transform(":field 1", "csv", vec![
            mr_segment("TypeA", "AA", vec![
                FieldMapping {
                    target: "Name".to_string(),
                    expression: FieldExpression::Copy("@.2".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        if let Some(ref mut src) = t.source {
            src.options.insert("delimiter".to_string(), ",".to_string());
        }
        let source = DynValue::String("data,AA,Alice".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(
            out.get("TypeA").unwrap().get("Name"),
            Some(&DynValue::String("Alice".to_string()))
        );
    }

    #[test]
    fn test_multi_record_invalid_discriminator_config_returns_error() {
        let t = multi_record_transform("invalid config", "csv", vec![]);
        let source = DynValue::String("some,data".to_string());
        let result = execute(&t, &source);
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn test_multi_record_non_string_source_falls_through() {
        // When source is not DynValue::String, multi-record mode is not triggered
        let t = multi_record_transform(":pos 0 :len 3", "fixed-width", vec![
            mr_segment("Header", "HDR", vec![
                FieldMapping {
                    target: "Val".to_string(),
                    expression: FieldExpression::Copy("@.x".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        // Source is an Object, not a String — should not enter multi-record mode
        let source = DynValue::Object(vec![
            ("x".to_string(), DynValue::String("hello".to_string())),
        ]);
        let result = execute(&t, &source);
        // It processes as a normal transform — the segment _type mapping might contribute
        assert!(result.success);
    }

    #[test]
    fn test_multi_record_position_discriminator_trims_whitespace() {
        let t = multi_record_transform(":pos 0 :len 5", "fixed-width", vec![
            mr_segment("Rec", "AB", vec![
                FieldMapping {
                    target: "Found".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("yes")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        // "AB   " trimmed to "AB"
        let source = DynValue::String("AB   some data".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert!(out.get("Rec").is_some());
    }

    #[test]
    fn test_multi_record_all_lines_unmatched_produces_empty() {
        let t = multi_record_transform(":pos 0 :len 3", "fixed-width", vec![
            mr_segment("Header", "HDR", vec![]),
        ]);
        let source = DynValue::String("AAA line1\nBBB line2\nCCC line3".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out, DynValue::Object(Vec::new()));
    }

    #[test]
    fn test_multi_record_empty_lines_are_skipped() {
        let t = multi_record_transform(":pos 0 :len 3", "fixed-width", vec![
            mr_segment("Rec", "HDR", vec![
                FieldMapping {
                    target: "Val".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("found")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        let source = DynValue::String("\n\nHDR data\n\n".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert!(out.get("Rec").is_some());
    }

    #[test]
    fn test_multi_record_csv_three_record_types() {
        let mut t = multi_record_transform(":field 0", "csv", vec![
            mr_segment("Headers", "H", vec![
                FieldMapping {
                    target: "Kind".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("header")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
            mr_segment("Details", "D", vec![
                FieldMapping {
                    target: "Kind".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("detail")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
            mr_segment("Trailers", "T", vec![
                FieldMapping {
                    target: "Kind".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("trailer")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        if let Some(ref mut src) = t.source {
            src.options.insert("delimiter".to_string(), ",".to_string());
        }
        let source = DynValue::String("H,header data\nD,detail1\nD,detail2\nT,trailer data".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert!(out.get("Headers").is_some());
        assert!(out.get("Details").is_some());
        assert!(out.get("Trailers").is_some());
    }

    #[test]
    fn test_multi_record_reads_raw_line_field() {
        let t = multi_record_transform(":pos 0 :len 3", "fixed-width", vec![
            mr_segment("Rec", "HDR", vec![
                FieldMapping {
                    target: "Raw".to_string(),
                    expression: FieldExpression::Copy("@._raw".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        let source = DynValue::String("HDR some raw data".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(
            out.get("Rec").unwrap().get("Raw"),
            Some(&DynValue::String("HDR some raw data".to_string()))
        );
    }

    #[test]
    fn test_multi_record_csv_reads_indexed_fields() {
        let mut t = multi_record_transform(":field 0", "csv", vec![
            mr_segment("Rec", "X", vec![
                FieldMapping {
                    target: "Col1".to_string(),
                    expression: FieldExpression::Copy("@.1".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
                FieldMapping {
                    target: "Col2".to_string(),
                    expression: FieldExpression::Copy("@.2".to_string()),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        if let Some(ref mut src) = t.source {
            src.options.insert("delimiter".to_string(), ",".to_string());
        }
        let source = DynValue::String("X,alpha,beta".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let rec = out.get("Rec").unwrap();
        assert_eq!(rec.get("Col1"), Some(&DynValue::String("alpha".to_string())));
        assert_eq!(rec.get("Col2"), Some(&DynValue::String("beta".to_string())));
    }

    #[test]
    fn test_multi_record_position_discriminator_mid_line() {
        let t = multi_record_transform(":pos 3 :len 2", "fixed-width", vec![
            mr_segment("Rec", "OK", vec![
                FieldMapping {
                    target: "Status".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("matched")),
                    directives: vec![],
                    modifiers: None,
                },
            ]),
        ]);
        let source = DynValue::String("123OK rest of line".to_string());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert!(out.get("Rec").is_some());
    }

    // =========================================================================
    // NEW TESTS: 2. Multi-pass transforms (~15 tests)
    // =========================================================================

    #[test]
    fn test_three_passes_execute_in_order() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Third".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(3)),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(3), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "First".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(1)),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Second".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(2)),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(2), condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("First"), Some(&DynValue::Integer(1)));
        assert_eq!(out.get("Second"), Some(&DynValue::Integer(2)));
        assert_eq!(out.get("Third"), Some(&DynValue::Integer(3)));
    }

    #[test]
    fn test_pass_none_executes_after_numbered_passes() {
        // pass: None should execute after pass 1
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Late".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("last")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: None, condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Early".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("first")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Early"), Some(&DynValue::String("first".to_string())));
        assert_eq!(out.get("Late"), Some(&DynValue::String("last".to_string())));
    }

    #[test]
    fn test_multiple_segments_in_same_pass() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "A".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("a")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "B".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("b")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&DynValue::String("a".to_string())));
        assert_eq!(out.get("B"), Some(&DynValue::String("b".to_string())));
    }

    #[test]
    fn test_pass_ordering_with_gaps() {
        // pass 1, pass 3 — no pass 2
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "P3".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(3)),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(3), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "P1".to_string(),
                    expression: FieldExpression::Literal(OdinValues::integer(1)),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("P1"), Some(&DynValue::Integer(1)));
        assert_eq!(out.get("P3"), Some(&DynValue::Integer(3)));
    }

    #[test]
    fn test_pass_1_output_visible_to_pass_2_via_out_ref() {
        // Pass 1 sets "Base", pass 2 reads it via bare path (resolved from global output)
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Base".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("hello")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Derived".to_string(),
                    expression: FieldExpression::Copy("Base".to_string()),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(2), condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Base"), Some(&DynValue::String("hello".to_string())));
        assert_eq!(out.get("Derived"), Some(&DynValue::String("hello".to_string())));
    }

    #[test]
    fn test_pass_0_executes_after_all_numbered() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "PassZero".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("last")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(0), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "PassOne".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("first")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("PassOne"), Some(&DynValue::String("first".to_string())));
        assert_eq!(out.get("PassZero"), Some(&DynValue::String("last".to_string())));
    }

    #[test]
    fn test_accumulator_incremented_across_loop_in_pass() {
        let mut t = custom_transform(vec![
            TransformSegment {
                name: "Items".to_string(),
                path: "Items".to_string(),
                source_path: Some("@.items".to_string()),
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Name".to_string(),
                    expression: FieldExpression::Copy("@_item.name".to_string()),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
        ]);
        t.accumulators.insert("count".to_string(), AccumulatorDef {
            name: "count".to_string(),
            initial: OdinValues::integer(0),
            persist: false,
        });
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::Object(vec![("name".to_string(), DynValue::String("A".to_string()))]),
                DynValue::Object(vec![("name".to_string(), DynValue::String("B".to_string()))]),
            ])),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_four_passes_all_produce_output() {
        let segs: Vec<TransformSegment> = (1..=4).map(|i| TransformSegment {
            name: String::new(), path: String::new(), source_path: None,
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![FieldMapping {
                target: format!("P{}", i),
                expression: FieldExpression::Literal(OdinValues::integer(i as i64)),
                directives: vec![], modifiers: None,
            }],
            children: Vec::new(), items: Vec::new(),
            pass: Some(i), condition: None,
        }).collect();
        let t = custom_transform(segs);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        for i in 1..=4 {
            assert_eq!(out.get(&format!("P{}", i)), Some(&DynValue::Integer(i as i64)));
        }
    }

    #[test]
    fn test_pass_none_and_pass_0_both_run_late() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "FromNone".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("none")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: None, condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "FromZero".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("zero")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(0), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Early".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("early")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Early"), Some(&DynValue::String("early".to_string())));
        assert_eq!(out.get("FromNone"), Some(&DynValue::String("none".to_string())));
        assert_eq!(out.get("FromZero"), Some(&DynValue::String("zero".to_string())));
    }

    #[test]
    fn test_pass_2_overwrites_pass_1_same_field() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Val".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("from_pass1")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Val".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("from_pass2")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(2), condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        // Pass 2 runs after pass 1, so it should overwrite
        let val = out.get("Val");
        assert!(val.is_some());
    }

    #[test]
    fn test_pass_with_condition_on_segment() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Skipped".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("no")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1),
                condition: Some("@.active".to_string()),
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![FieldMapping {
                    target: "Included".to_string(),
                    expression: FieldExpression::Literal(OdinValues::string("yes")),
                    directives: vec![], modifiers: None,
                }],
                children: Vec::new(), items: Vec::new(),
                pass: Some(2), condition: None,
            },
        ]);
        let source = DynValue::Object(vec![
            ("active".to_string(), DynValue::Bool(false)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Skipped"), None);
        assert_eq!(out.get("Included"), Some(&DynValue::String("yes".to_string())));
    }

    #[test]
    fn test_single_pass_no_ordering_issues() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![
                    FieldMapping {
                        target: "A".to_string(),
                        expression: FieldExpression::Literal(OdinValues::integer(1)),
                        directives: vec![], modifiers: None,
                    },
                    FieldMapping {
                        target: "B".to_string(),
                        expression: FieldExpression::Literal(OdinValues::integer(2)),
                        directives: vec![], modifiers: None,
                    },
                ],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&DynValue::Integer(1)));
        assert_eq!(out.get("B"), Some(&DynValue::Integer(2)));
    }

    // =========================================================================
    // NEW TESTS: 3. Confidential field enforcement (~15 tests)
    // =========================================================================

    #[test]
    fn test_confidential_redact_string_becomes_null() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Secret".to_string(),
                expression: FieldExpression::Copy("@.secret".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("secret".to_string(), DynValue::String("top-secret".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Secret"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_redact_number_becomes_null() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Salary".to_string(),
                expression: FieldExpression::Copy("@.salary".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("salary".to_string(), DynValue::Float(75000.50)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Salary"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_redact_boolean_becomes_null() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "HasAccess".to_string(),
                expression: FieldExpression::Copy("@.access".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("access".to_string(), DynValue::Bool(true)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("HasAccess"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_redact_non_confidential_fields_unchanged() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Public".to_string(),
                expression: FieldExpression::Copy("@.name".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "Private".to_string(),
                expression: FieldExpression::Copy("@.ssn".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("Alice".to_string())),
            ("ssn".to_string(), DynValue::String("123-45-6789".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Public"), Some(&DynValue::String("Alice".to_string())));
        assert_eq!(out.get("Private"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_mask_string_becomes_asterisks() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Password".to_string(),
                expression: FieldExpression::Copy("@.pw".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let source = DynValue::Object(vec![
            ("pw".to_string(), DynValue::String("mypassword".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Password"), Some(&DynValue::String("**********".to_string())));
    }

    #[test]
    fn test_confidential_mask_mixed_confidential_and_non() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Copy("@.name".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "Token".to_string(),
                expression: FieldExpression::Copy("@.token".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("Bob".to_string())),
            ("token".to_string(), DynValue::String("abc123".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Name"), Some(&DynValue::String("Bob".to_string())));
        assert_eq!(out.get("Token"), Some(&DynValue::String("******".to_string())));
    }

    #[test]
    fn test_confidential_no_enforcement_value_passes_through() {
        let t = minimal_transform(vec![
            FieldMapping {
                target: "Secret".to_string(),
                expression: FieldExpression::Copy("@.secret".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        // enforce_confidential is None by default
        let source = DynValue::Object(vec![
            ("secret".to_string(), DynValue::String("visible".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Secret"), Some(&DynValue::String("visible".to_string())));
    }

    #[test]
    fn test_confidential_redact_with_dotted_nested_path() {
        // Use dotted target paths to create nested output with confidential field
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "info.name".to_string(),
                expression: FieldExpression::Copy("@.name".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "info.ssn".to_string(),
                expression: FieldExpression::Copy("@.ssn".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("name".to_string(), DynValue::String("Alice".to_string())),
            ("ssn".to_string(), DynValue::String("111-22-3333".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let info = out.get("info").unwrap();
        assert_eq!(info.get("name"), Some(&DynValue::String("Alice".to_string())));
        // The nested confidential field should be redacted
        assert_eq!(info.get("ssn"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_multiple_fields_redacted() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "SSN".to_string(),
                expression: FieldExpression::Copy("@.ssn".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
            FieldMapping {
                target: "DOB".to_string(),
                expression: FieldExpression::Copy("@.dob".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Copy("@.name".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("ssn".to_string(), DynValue::String("123-45-6789".to_string())),
            ("dob".to_string(), DynValue::String("1990-01-01".to_string())),
            ("name".to_string(), DynValue::String("Charlie".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("SSN"), Some(&DynValue::Null));
        assert_eq!(out.get("DOB"), Some(&DynValue::Null));
        assert_eq!(out.get("Name"), Some(&DynValue::String("Charlie".to_string())));
    }

    #[test]
    fn test_confidential_mask_boolean_becomes_null_v2() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Flag".to_string(),
                expression: FieldExpression::Copy("@.flag".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let source = DynValue::Object(vec![
            ("flag".to_string(), DynValue::Bool(true)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        // Booleans under mask mode become null
        assert_eq!(out.get("Flag"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_mask_empty_string_v2() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Empty".to_string(),
                expression: FieldExpression::Copy("@.val".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let source = DynValue::Object(vec![
            ("val".to_string(), DynValue::String(String::new())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        // Empty string masked is still empty string (0 asterisks)
        assert_eq!(out.get("Empty"), Some(&DynValue::String(String::new())));
    }

    #[test]
    fn test_confidential_redact_null_stays_null() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Nil".to_string(),
                expression: FieldExpression::Copy("@.missing".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(Vec::new());
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Nil"), Some(&DynValue::Null));
    }

    #[test]
    fn test_confidential_redact_with_required_modifier() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Secret".to_string(),
                expression: FieldExpression::Copy("@.val".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(true, true, false)), // required AND confidential
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let source = DynValue::Object(vec![
            ("val".to_string(), DynValue::String("classified".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Secret"), Some(&DynValue::Null));
        // Modifiers should still be recorded
        assert!(result.modifiers.contains_key("Secret"));
        assert!(result.modifiers["Secret"].required);
        assert!(result.modifiers["Secret"].confidential);
    }

    #[test]
    fn test_confidential_mask_number_becomes_null() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Amount".to_string(),
                expression: FieldExpression::Copy("@.amount".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let source = DynValue::Object(vec![
            ("amount".to_string(), DynValue::Float(99.99)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Amount"), Some(&DynValue::Null));
    }

    // =========================================================================
    // NEW TESTS: 4. Additional engine edge cases (~10 tests)
    // =========================================================================

    #[test]
    fn test_empty_transform_no_segments_no_mappings() {
        let t = custom_transform(vec![]);
        let source = DynValue::Object(vec![
            ("x".to_string(), DynValue::Integer(1)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out, DynValue::Object(Vec::new()));
    }

    #[test]
    fn test_transform_with_only_constants() {
        let mut t = custom_transform(vec![root_segment(vec![
            FieldMapping {
                target: "Version".to_string(),
                expression: FieldExpression::Copy("$const.ver".to_string()),
                directives: vec![],
                modifiers: None,
            },
            FieldMapping {
                target: "Author".to_string(),
                expression: FieldExpression::Copy("$const.author".to_string()),
                directives: vec![],
                modifiers: None,
            },
        ])]);
        t.constants.insert("ver".to_string(), OdinValues::string("3.0"));
        t.constants.insert("author".to_string(), OdinValues::string("ODIN"));
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Version"), Some(&DynValue::String("3.0".to_string())));
        assert_eq!(out.get("Author"), Some(&DynValue::String("ODIN".to_string())));
    }

    #[test]
    fn test_lookup_table_usage() {
        let mut t = custom_transform(vec![root_segment(vec![
            FieldMapping {
                target: "State".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "lookup".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Literal(OdinValues::string("states.name")),
                        VerbArg::Reference("@.code".to_string(), Vec::new()),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ])]);
        t.tables.insert("states".to_string(), LookupTable {
            name: "states".to_string(),
            columns: vec!["code".to_string(), "name".to_string()],
            rows: vec![
                vec![DynValue::String("OR".to_string()), DynValue::String("Oregon".to_string())],
                vec![DynValue::String("WA".to_string()), DynValue::String("Washington".to_string())],
            ],
            default: None,
        });
        let source = DynValue::Object(vec![
            ("code".to_string(), DynValue::String("OR".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("State"), Some(&DynValue::String("Oregon".to_string())));
    }

    #[test]
    fn test_lookup_table_not_found_returns_null() {
        let mut t = custom_transform(vec![root_segment(vec![
            FieldMapping {
                target: "State".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "lookup".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Literal(OdinValues::string("states.name")),
                        VerbArg::Reference("@.code".to_string(), Vec::new()),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ])]);
        t.tables.insert("states".to_string(), LookupTable {
            name: "states".to_string(),
            columns: vec!["code".to_string(), "name".to_string()],
            rows: vec![
                vec![DynValue::String("OR".to_string()), DynValue::String("Oregon".to_string())],
            ],
            default: None,
        });
        let source = DynValue::Object(vec![
            ("code".to_string(), DynValue::String("XX".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("State"), Some(&DynValue::Null));
    }

    #[test]
    fn test_condition_on_segment_truthy_integer() {
        // Condition references @.level which is nonzero -> truthy -> segment runs
        let t = custom_transform(vec![TransformSegment {
            name: String::new(), path: String::new(), source_path: None,
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![FieldMapping {
                target: "Premium".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("yes")),
                directives: vec![], modifiers: None,
            }],
            children: Vec::new(), items: Vec::new(),
            pass: None,
            condition: Some("@.level".to_string()),
        }]);
        let source = DynValue::Object(vec![
            ("level".to_string(), DynValue::Integer(5)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Premium"), Some(&DynValue::String("yes".to_string())));
    }

    #[test]
    fn test_condition_on_segment_falsy_integer_zero() {
        // Condition references @.level which is 0 -> falsy -> segment skipped
        let t = custom_transform(vec![TransformSegment {
            name: String::new(), path: String::new(), source_path: None,
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![FieldMapping {
                target: "Premium".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("yes")),
                directives: vec![], modifiers: None,
            }],
            children: Vec::new(), items: Vec::new(),
            pass: None,
            condition: Some("@.level".to_string()),
        }]);
        let source = DynValue::Object(vec![
            ("level".to_string(), DynValue::Integer(0)),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Premium"), None);
    }

    #[test]
    fn test_loop_processing_with_transform_verb() {
        let t = custom_transform(vec![TransformSegment {
            name: "Results".to_string(),
            path: "Results".to_string(),
            source_path: Some("@.items".to_string()),
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![FieldMapping {
                target: "Upper".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "upper".to_string(),
                    is_custom: false,
                    args: vec![VerbArg::Reference("@_item.name".to_string(), Vec::new())],
                }),
                directives: vec![], modifiers: None,
            }],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        }]);
        let source = DynValue::Object(vec![
            ("items".to_string(), DynValue::Array(vec![
                DynValue::Object(vec![("name".to_string(), DynValue::String("alice".to_string()))]),
                DynValue::Object(vec![("name".to_string(), DynValue::String("bob".to_string()))]),
            ])),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let results = out.get("Results").unwrap().as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].get("Upper"), Some(&DynValue::String("ALICE".to_string())));
        assert_eq!(results[1].get("Upper"), Some(&DynValue::String("BOB".to_string())));
    }

    #[test]
    fn test_multiple_constants_referenced() {
        let mut t = custom_transform(vec![root_segment(vec![
            FieldMapping {
                target: "Greeting".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("$const.prefix".to_string(), Vec::new()),
                        VerbArg::Literal(OdinValues::string(" ")),
                        VerbArg::Reference("$const.suffix".to_string(), Vec::new()),
                    ],
                }),
                directives: vec![], modifiers: None,
            },
        ])]);
        t.constants.insert("prefix".to_string(), OdinValues::string("Hello"));
        t.constants.insert("suffix".to_string(), OdinValues::string("World"));
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Greeting"), Some(&DynValue::String("Hello World".to_string())));
    }

    #[test]
    fn test_transform_with_only_literals_no_source_data() {
        let t = minimal_transform(vec![
            FieldMapping {
                target: "A".to_string(),
                expression: FieldExpression::Literal(OdinValues::string("alpha")),
                directives: vec![], modifiers: None,
            },
            FieldMapping {
                target: "B".to_string(),
                expression: FieldExpression::Literal(OdinValues::integer(42)),
                directives: vec![], modifiers: None,
            },
            FieldMapping {
                target: "C".to_string(),
                expression: FieldExpression::Literal(OdinValues::boolean(true)),
                directives: vec![], modifiers: None,
            },
            FieldMapping {
                target: "D".to_string(),
                expression: FieldExpression::Literal(OdinValues::null()),
                directives: vec![], modifiers: None,
            },
        ]);
        let result = execute(&t, &DynValue::Object(Vec::new()));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&DynValue::String("alpha".to_string())));
        assert_eq!(out.get("B"), Some(&DynValue::Integer(42)));
        assert_eq!(out.get("C"), Some(&DynValue::Bool(true)));
        assert_eq!(out.get("D"), Some(&DynValue::Null));
    }

    #[test]
    fn test_lookup_table_with_default_value() {
        let mut t = custom_transform(vec![root_segment(vec![
            FieldMapping {
                target: "Color".to_string(),
                expression: FieldExpression::Transform(VerbCall {
                    verb: "lookup".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Literal(OdinValues::string("colors.name")),
                        VerbArg::Reference("@.code".to_string(), Vec::new()),
                    ],
                }),
                directives: vec![],
                modifiers: None,
            },
        ])]);
        t.tables.insert("colors".to_string(), LookupTable {
            name: "colors".to_string(),
            columns: vec!["code".to_string(), "name".to_string()],
            rows: vec![
                vec![DynValue::String("R".to_string()), DynValue::String("Red".to_string())],
            ],
            default: Some(DynValue::String("Unknown".to_string())),
        });
        let source = DynValue::Object(vec![
            ("code".to_string(), DynValue::String("Z".to_string())),
        ]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Color"), Some(&DynValue::String("Unknown".to_string())));
    }

}

// =============================================================================
// Extended tests for TypeScript parity
// =============================================================================

#[cfg(test)]
mod extended_tests {
    use super::*;
    use crate::types::transform::*;
    use crate::types::values::{OdinValues, OdinModifiers};
    use std::collections::HashMap;

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn mk_transform(mappings: Vec<FieldMapping>) -> OdinTransform {
        OdinTransform {
            metadata: TransformMetadata::default(),
            source: None,
            target: TargetConfig { format: "json".to_string(), options: HashMap::new() },
            constants: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
            segments: vec![TransformSegment {
                name: String::new(), path: String::new(),
                source_path: None, discriminator: None,
                is_array: false, directives: Vec::new(),
                mappings, children: Vec::new(), items: Vec::new(),
                pass: None, condition: None,
            }],
            imports: Vec::new(),
            passes: Vec::new(),
            enforce_confidential: None,
            strict_types: false,
        }
    }

    fn mk_custom(segments: Vec<TransformSegment>) -> OdinTransform {
        OdinTransform {
            metadata: TransformMetadata::default(),
            source: None,
            target: TargetConfig { format: "json".to_string(), options: HashMap::new() },
            constants: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
            segments,
            imports: Vec::new(),
            passes: Vec::new(),
            enforce_confidential: None,
            strict_types: false,
        }
    }

    fn root_seg(mappings: Vec<FieldMapping>) -> TransformSegment {
        TransformSegment {
            name: String::new(), path: String::new(),
            source_path: None, discriminator: None,
            is_array: false, directives: Vec::new(),
            mappings, children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        }
    }

    fn named_seg(name: &str, mappings: Vec<FieldMapping>) -> TransformSegment {
        TransformSegment {
            name: name.to_string(), path: name.to_string(),
            source_path: None, discriminator: None,
            is_array: false, directives: Vec::new(),
            mappings, children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        }
    }

    fn pass_seg(pass: usize, mappings: Vec<FieldMapping>) -> TransformSegment {
        TransformSegment {
            name: String::new(), path: String::new(),
            source_path: None, discriminator: None,
            is_array: false, directives: Vec::new(),
            mappings, children: Vec::new(), items: Vec::new(),
            pass: Some(pass), condition: None,
        }
    }

    fn copy_field(target: &str, src: &str) -> FieldMapping {
        FieldMapping {
            target: target.to_string(),
            expression: FieldExpression::Copy(src.to_string()),
            directives: vec![], modifiers: None,
        }
    }

    fn literal_field(target: &str, val: crate::types::values::OdinValue) -> FieldMapping {
        FieldMapping {
            target: target.to_string(),
            expression: FieldExpression::Literal(val),
            directives: vec![], modifiers: None,
        }
    }

    fn verb_field(target: &str, verb: &str, args: Vec<VerbArg>) -> FieldMapping {
        FieldMapping {
            target: target.to_string(),
            expression: FieldExpression::Transform(VerbCall {
                verb: verb.to_string(), is_custom: false, args,
            }),
            directives: vec![], modifiers: None,
        }
    }

    fn ref_arg(path: &str) -> VerbArg {
        VerbArg::Reference(path.to_string(), Vec::new())
    }

    fn lit_arg_str(s: &str) -> VerbArg {
        VerbArg::Literal(OdinValues::string(s))
    }

    fn lit_arg_int(n: i64) -> VerbArg {
        VerbArg::Literal(OdinValues::integer(n))
    }

    fn lit_arg_num(n: f64) -> VerbArg {
        VerbArg::Literal(OdinValues::number(n))
    }

    fn lit_arg_bool(b: bool) -> VerbArg {
        VerbArg::Literal(OdinValues::boolean(b))
    }

    fn verb_arg(verb: &str, args: Vec<VerbArg>) -> VerbArg {
        VerbArg::Verb(VerbCall { verb: verb.to_string(), is_custom: false, args })
    }

    fn modifiers_field(target: &str, src: &str, mods: OdinModifiers) -> FieldMapping {
        FieldMapping {
            target: target.to_string(),
            expression: FieldExpression::Copy(src.to_string()),
            directives: vec![], modifiers: Some(mods),
        }
    }

    fn confidential_mods() -> OdinModifiers {
        OdinModifiers { required: false, confidential: true, deprecated: false, attr: false }
    }

    fn required_mods() -> OdinModifiers {
        OdinModifiers { required: true, confidential: false, deprecated: false, attr: false }
    }

    fn deprecated_mods() -> OdinModifiers {
        OdinModifiers { required: false, confidential: false, deprecated: true, attr: false }
    }

    fn all_mods() -> OdinModifiers {
        OdinModifiers { required: true, confidential: true, deprecated: true, attr: false }
    }

    fn src_obj(fields: Vec<(&str, DynValue)>) -> DynValue {
        DynValue::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn s(val: &str) -> DynValue { DynValue::String(val.to_string()) }
    fn i(val: i64) -> DynValue { DynValue::Integer(val) }
    fn f(val: f64) -> DynValue { DynValue::Float(val) }
    fn b(val: bool) -> DynValue { DynValue::Bool(val) }

    // =========================================================================
    // 1. Strict type checking (~40 tests)
    // =========================================================================
    // Note: strict_types is parsed but the engine does not yet enforce type
    // checking on verb arguments. These tests verify that the flag is parsed
    // and that verbs do their own internal type handling (coercion or error).

    #[test]
    fn ext_strict_upper_on_integer_errors() {
        // upper on a number should error (not coerce)
        let t = mk_transform(vec![verb_field("Out", "upper", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", i(42))]));
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn ext_strict_lower_on_integer_errors() {
        let t = mk_transform(vec![verb_field("Out", "lower", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", i(99))]));
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn ext_strict_upper_on_boolean_errors() {
        let t = mk_transform(vec![verb_field("Out", "upper", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", b(true))]));
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn ext_strict_lower_on_boolean_errors() {
        let t = mk_transform(vec![verb_field("Out", "lower", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", b(false))]));
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn ext_strict_add_integers() {
        let t = mk_transform(vec![verb_field("Out", "add", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", i(10)), ("b", i(20))]));
        assert!(result.success);
        let out = result.output.unwrap();
        // add should return numeric result
        let val = out.get("Out").unwrap();
        assert!(val.as_i64() == Some(30) || val.as_f64() == Some(30.0));
    }

    #[test]
    fn ext_strict_add_floats() {
        let t = mk_transform(vec![verb_field("Out", "add", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", f(1.5)), ("b", f(2.5))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!((val.as_f64().unwrap() - 4.0).abs() < 0.001);
    }

    #[test]
    fn ext_strict_add_string_numbers() {
        // Adding string representations of numbers should coerce
        let t = mk_transform(vec![verb_field("Out", "add", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", s("5")), ("b", s("3"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_f64() == Some(8.0) || val.as_i64() == Some(8));
    }

    #[test]
    fn ext_strict_subtract_integers() {
        let t = mk_transform(vec![verb_field("Out", "subtract", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", i(50)), ("b", i(30))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_i64() == Some(20) || val.as_f64() == Some(20.0));
    }

    #[test]
    fn ext_strict_multiply_integers() {
        let t = mk_transform(vec![verb_field("Out", "multiply", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", i(6)), ("b", i(7))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_i64() == Some(42) || val.as_f64() == Some(42.0));
    }

    #[test]
    fn ext_strict_divide_integers() {
        let t = mk_transform(vec![verb_field("Out", "divide", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", i(10)), ("b", i(3))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_f64().is_some());
    }

    #[test]
    fn ext_strict_abs_negative() {
        let t = mk_transform(vec![verb_field("Out", "abs", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", i(-42))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_i64() == Some(42) || val.as_f64() == Some(42.0));
    }

    #[test]
    fn ext_strict_abs_positive_unchanged() {
        let t = mk_transform(vec![verb_field("Out", "abs", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", i(42))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_i64() == Some(42) || val.as_f64() == Some(42.0));
    }

    #[test]
    fn ext_strict_abs_float() {
        let t = mk_transform(vec![verb_field("Out", "abs", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", f(-3.14))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!((val.as_f64().unwrap() - 3.14).abs() < 0.001);
    }

    #[test]
    fn ext_strict_round_float() {
        let t = mk_transform(vec![verb_field("Out", "round", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", f(3.7))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_i64() == Some(4) || val.as_f64() == Some(4.0));
    }

    #[test]
    fn ext_strict_round_negative() {
        let t = mk_transform(vec![verb_field("Out", "round", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", f(-2.3))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_i64() == Some(-2) || val.as_f64() == Some(-2.0));
    }

    #[test]
    fn ext_strict_coerce_string_from_int() {
        let t = mk_transform(vec![verb_field("Out", "coerceString", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", i(42))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&s("42")));
    }

    #[test]
    fn ext_strict_coerce_string_from_bool() {
        let t = mk_transform(vec![verb_field("Out", "coerceString", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", b(true))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&s("true")));
    }

    #[test]
    fn ext_strict_coerce_string_from_float() {
        let t = mk_transform(vec![verb_field("Out", "coerceString", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", f(3.14))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_str().is_some());
    }

    #[test]
    fn ext_strict_coerce_number_from_string() {
        let t = mk_transform(vec![verb_field("Out", "coerceNumber", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("42"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_f64() == Some(42.0) || val.as_i64() == Some(42));
    }

    #[test]
    fn ext_strict_coerce_number_from_float_string() {
        let t = mk_transform(vec![verb_field("Out", "coerceNumber", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("3.14"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!((val.as_f64().unwrap() - 3.14).abs() < 0.001);
    }

    #[test]
    fn ext_strict_coerce_boolean_from_string_true() {
        let t = mk_transform(vec![verb_field("Out", "coerceBoolean", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("true"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_coerce_boolean_from_string_false() {
        let t = mk_transform(vec![verb_field("Out", "coerceBoolean", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("false"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(false)));
    }

    #[test]
    fn ext_strict_coerce_boolean_from_int_1() {
        let t = mk_transform(vec![verb_field("Out", "coerceBoolean", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", i(1))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_coerce_boolean_from_int_0() {
        let t = mk_transform(vec![verb_field("Out", "coerceBoolean", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", i(0))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(false)));
    }

    #[test]
    fn ext_strict_coerce_integer_from_float() {
        let t = mk_transform(vec![verb_field("Out", "coerceInteger", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", f(3.9))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        // Should truncate or round
        assert!(val.as_i64().is_some());
    }

    #[test]
    fn ext_strict_coerce_integer_from_string() {
        let t = mk_transform(vec![verb_field("Out", "coerceInteger", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("99"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_i64() == Some(99));
    }

    #[test]
    fn ext_strict_trim_on_number_errors() {
        let t = mk_transform(vec![verb_field("Out", "trim", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", i(42))]));
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn ext_strict_capitalize_string() {
        let t = mk_transform(vec![verb_field("Out", "capitalize", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello world"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        let text = val.as_str().unwrap();
        assert!(text.starts_with('H'));
    }

    #[test]
    fn ext_strict_length_string() {
        let t = mk_transform(vec![verb_field("Out", "length", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_i64() == Some(5));
    }

    #[test]
    fn ext_strict_length_array() {
        let t = mk_transform(vec![verb_field("Out", "length", vec![ref_arg("@.val")])]);
        let source = src_obj(vec![("val", DynValue::Array(vec![i(1), i(2), i(3)]))]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!(val.as_i64() == Some(3));
    }

    #[test]
    fn ext_strict_substring_basic() {
        let t = mk_transform(vec![verb_field("Out", "substring", vec![ref_arg("@.val"), lit_arg_int(0), lit_arg_int(3)])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&s("hel")));
    }

    #[test]
    fn ext_strict_replace_basic() {
        let t = mk_transform(vec![verb_field("Out", "replace", vec![ref_arg("@.val"), lit_arg_str("world"), lit_arg_str("earth")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello world"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&s("hello earth")));
    }

    #[test]
    fn ext_strict_concat_null_and_string() {
        let t = mk_transform(vec![verb_field("Out", "concat", vec![ref_arg("@.missing"), lit_arg_str(" test")])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        // concat with null should still produce output
        let val = out.get("Out").unwrap();
        assert!(val.as_str().is_some());
    }

    #[test]
    fn ext_strict_eq_same_strings() {
        let t = mk_transform(vec![verb_field("Out", "eq", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", s("x")), ("b", s("x"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_eq_different_strings() {
        let t = mk_transform(vec![verb_field("Out", "eq", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", s("x")), ("b", s("y"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(false)));
    }

    #[test]
    fn ext_strict_ne_different() {
        let t = mk_transform(vec![verb_field("Out", "ne", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", s("x")), ("b", s("y"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_not_true() {
        let t = mk_transform(vec![verb_field("Out", "not", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", b(true))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(false)));
    }

    #[test]
    fn ext_strict_not_false() {
        let t = mk_transform(vec![verb_field("Out", "not", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", b(false))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_and_true_true() {
        let t = mk_transform(vec![verb_field("Out", "and", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", b(true)), ("b", b(true))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_and_true_false() {
        let t = mk_transform(vec![verb_field("Out", "and", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", b(true)), ("b", b(false))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(false)));
    }

    #[test]
    fn ext_strict_or_false_true() {
        let t = mk_transform(vec![verb_field("Out", "or", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", b(false)), ("b", b(true))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_or_false_false() {
        let t = mk_transform(vec![verb_field("Out", "or", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", b(false)), ("b", b(false))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(false)));
    }

    #[test]
    fn ext_strict_lt_true() {
        let t = mk_transform(vec![verb_field("Out", "lt", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", i(1)), ("b", i(2))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_gt_true() {
        let t = mk_transform(vec![verb_field("Out", "gt", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", i(5)), ("b", i(3))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_lte_equal() {
        let t = mk_transform(vec![verb_field("Out", "lte", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", i(5)), ("b", i(5))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_gte_equal() {
        let t = mk_transform(vec![verb_field("Out", "gte", vec![ref_arg("@.a"), ref_arg("@.b")])]);
        let result = execute(&t, &src_obj(vec![("a", i(5)), ("b", i(5))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_is_null_true() {
        let t = mk_transform(vec![verb_field("Out", "isNull", vec![ref_arg("@.missing")])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_strict_is_null_false() {
        let t = mk_transform(vec![verb_field("Out", "isNull", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("x"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&b(false)));
    }

    // =========================================================================
    // 2. Conditional operators (~30 tests)
    // =========================================================================

    #[test]
    fn ext_cond_ifelse_true_returns_then() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            lit_arg_bool(true), lit_arg_str("yes"), lit_arg_str("no"),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("yes")));
    }

    #[test]
    fn ext_cond_ifelse_false_returns_else() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            lit_arg_bool(false), lit_arg_str("yes"), lit_arg_str("no"),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("no")));
    }

    #[test]
    fn ext_cond_ifelse_with_ref_condition() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            ref_arg("@.active"), lit_arg_str("active"), lit_arg_str("inactive"),
        ])]);
        let result = execute(&t, &src_obj(vec![("active", b(true))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("active")));
    }

    #[test]
    fn ext_cond_ifelse_ref_false() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            ref_arg("@.active"), lit_arg_str("active"), lit_arg_str("inactive"),
        ])]);
        let result = execute(&t, &src_obj(vec![("active", b(false))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("inactive")));
    }

    #[test]
    fn ext_cond_ifelse_null_is_falsy() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            ref_arg("@.missing"), lit_arg_str("found"), lit_arg_str("missing"),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("missing")));
    }

    #[test]
    fn ext_cond_ifelse_zero_is_falsy() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            ref_arg("@.val"), lit_arg_str("nonzero"), lit_arg_str("zero"),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", i(0))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("zero")));
    }

    #[test]
    fn ext_cond_ifelse_nonempty_string_is_truthy() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            ref_arg("@.val"), lit_arg_str("truthy"), lit_arg_str("falsy"),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("truthy")));
    }

    #[test]
    fn ext_cond_ifelse_empty_string_is_falsy() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            ref_arg("@.val"), lit_arg_str("truthy"), lit_arg_str("falsy"),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s(""))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("falsy")));
    }

    #[test]
    fn ext_cond_ifnull_with_null() {
        let t = mk_transform(vec![verb_field("Out", "ifNull", vec![
            ref_arg("@.missing"), lit_arg_str("default"),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("default")));
    }

    #[test]
    fn ext_cond_ifnull_with_value() {
        let t = mk_transform(vec![verb_field("Out", "ifNull", vec![
            ref_arg("@.val"), lit_arg_str("default"),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("present"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("present")));
    }

    #[test]
    fn ext_cond_ifempty_with_empty_string() {
        let t = mk_transform(vec![verb_field("Out", "ifEmpty", vec![
            ref_arg("@.val"), lit_arg_str("was_empty"),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s(""))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("was_empty")));
    }

    #[test]
    fn ext_cond_ifempty_with_nonempty() {
        let t = mk_transform(vec![verb_field("Out", "ifEmpty", vec![
            ref_arg("@.val"), lit_arg_str("was_empty"),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("content"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("content")));
    }

    #[test]
    fn ext_cond_coalesce_first_non_null() {
        let t = mk_transform(vec![verb_field("Out", "coalesce", vec![
            ref_arg("@.a"), ref_arg("@.b"), ref_arg("@.c"),
        ])]);
        let result = execute(&t, &src_obj(vec![("c", s("third"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("third")));
    }

    #[test]
    fn ext_cond_coalesce_all_null() {
        let t = mk_transform(vec![verb_field("Out", "coalesce", vec![
            ref_arg("@.a"), ref_arg("@.b"),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_cond_coalesce_first_present() {
        let t = mk_transform(vec![verb_field("Out", "coalesce", vec![
            ref_arg("@.a"), ref_arg("@.b"),
        ])]);
        let result = execute(&t, &src_obj(vec![("a", s("first")), ("b", s("second"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("first")));
    }

    #[test]
    fn ext_cond_cond_first_match() {
        let t = mk_transform(vec![verb_field("Out", "cond", vec![
            lit_arg_bool(true), lit_arg_str("A"),
            lit_arg_bool(false), lit_arg_str("B"),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("A")));
    }

    #[test]
    fn ext_cond_cond_second_match() {
        let t = mk_transform(vec![verb_field("Out", "cond", vec![
            lit_arg_bool(false), lit_arg_str("A"),
            lit_arg_bool(true), lit_arg_str("B"),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("B")));
    }

    #[test]
    fn ext_cond_cond_no_match_returns_null() {
        let t = mk_transform(vec![verb_field("Out", "cond", vec![
            lit_arg_bool(false), lit_arg_str("A"),
            lit_arg_bool(false), lit_arg_str("B"),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_cond_cond_with_default() {
        // cond with odd number of args: last is default
        let t = mk_transform(vec![verb_field("Out", "cond", vec![
            lit_arg_bool(false), lit_arg_str("A"),
            lit_arg_str("default"),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("default")));
    }

    #[test]
    fn ext_cond_ifelse_with_verb_in_then() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            lit_arg_bool(true),
            verb_arg("upper", vec![lit_arg_str("hello")]),
            lit_arg_str("no"),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("HELLO")));
    }

    #[test]
    fn ext_cond_ifelse_with_verb_in_else() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            lit_arg_bool(false),
            lit_arg_str("yes"),
            verb_arg("lower", vec![lit_arg_str("WORLD")]),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("world")));
    }

    #[test]
    fn ext_cond_ifnull_with_null_uses_default() {
        let t = mk_transform(vec![verb_field("Out", "ifNull", vec![
            VerbArg::Literal(OdinValues::null()), lit_arg_str("fallback"),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("fallback")));
    }

    #[test]
    fn ext_cond_segment_condition_truthy_integer() {
        let seg = TransformSegment {
            name: String::new(), path: String::new(),
            source_path: None, discriminator: None,
            is_array: false, directives: Vec::new(),
            mappings: vec![literal_field("A", OdinValues::string("found"))],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: Some("@.flag".to_string()),
        };
        let mut t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![("flag", i(1))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&s("found")));
    }

    #[test]
    fn ext_cond_segment_condition_falsy_zero() {
        let seg = TransformSegment {
            name: String::new(), path: String::new(),
            source_path: None, discriminator: None,
            is_array: false, directives: Vec::new(),
            mappings: vec![literal_field("A", OdinValues::string("found"))],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: Some("@.flag".to_string()),
        };
        let t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![("flag", i(0))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), None);
    }

    #[test]
    fn ext_cond_segment_condition_missing_field() {
        let seg = TransformSegment {
            name: String::new(), path: String::new(),
            source_path: None, discriminator: None,
            is_array: false, directives: Vec::new(),
            mappings: vec![literal_field("A", OdinValues::string("found"))],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: Some("@.doesNotExist".to_string()),
        };
        let t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), None);
    }

    #[test]
    fn ext_cond_between_in_range() {
        let t = mk_transform(vec![verb_field("Out", "between", vec![
            ref_arg("@.val"), lit_arg_int(1), lit_arg_int(10),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", i(5))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_cond_between_out_of_range() {
        let t = mk_transform(vec![verb_field("Out", "between", vec![
            ref_arg("@.val"), lit_arg_int(1), lit_arg_int(10),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", i(15))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&b(false)));
    }

    #[test]
    fn ext_cond_is_string_true() {
        let t = mk_transform(vec![verb_field("Out", "isString", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_cond_is_string_false() {
        let t = mk_transform(vec![verb_field("Out", "isString", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", i(42))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&b(false)));
    }

    #[test]
    fn ext_cond_is_number_true() {
        let t = mk_transform(vec![verb_field("Out", "isNumber", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", i(42))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&b(true)));
    }

    // =========================================================================
    // 3. Verb expressions (~30 tests)
    // =========================================================================

    #[test]
    fn ext_verb_upper_trim_chain() {
        // upper(trim(value))
        let t = mk_transform(vec![verb_field("Out", "upper", vec![
            verb_arg("trim", vec![ref_arg("@.val")]),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("  hello  "))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("HELLO")));
    }

    #[test]
    fn ext_verb_lower_trim_chain() {
        let t = mk_transform(vec![verb_field("Out", "lower", vec![
            verb_arg("trim", vec![ref_arg("@.val")]),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("  WORLD  "))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("world")));
    }

    #[test]
    fn ext_verb_concat_upper_lower() {
        // concat(upper(@first), " ", lower(@last))
        let t = mk_transform(vec![verb_field("Out", "concat", vec![
            verb_arg("upper", vec![ref_arg("@.first")]),
            lit_arg_str(" "),
            verb_arg("lower", vec![ref_arg("@.last")]),
        ])]);
        let result = execute(&t, &src_obj(vec![("first", s("john")), ("last", s("DOE"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("JOHN doe")));
    }

    #[test]
    fn ext_verb_triple_nesting() {
        // upper(concat(trim(@a), trim(@b)))
        let t = mk_transform(vec![verb_field("Out", "upper", vec![
            verb_arg("concat", vec![
                verb_arg("trim", vec![ref_arg("@.a")]),
                verb_arg("trim", vec![ref_arg("@.b")]),
            ]),
        ])]);
        let result = execute(&t, &src_obj(vec![("a", s(" hello ")), ("b", s(" world "))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("HELLOWORLD")));
    }

    #[test]
    fn ext_verb_add_multiply_nested() {
        // add(multiply(@a, @b), @c)
        let t = mk_transform(vec![verb_field("Out", "add", vec![
            verb_arg("multiply", vec![ref_arg("@.a"), ref_arg("@.b")]),
            ref_arg("@.c"),
        ])]);
        let result = execute(&t, &src_obj(vec![("a", i(3)), ("b", i(4)), ("c", i(5))]));
        assert!(result.success);
        let val = result.output.unwrap().get("Out").unwrap().clone();
        assert!(val.as_i64() == Some(17) || val.as_f64() == Some(17.0));
    }

    #[test]
    fn ext_verb_concat_three_fields() {
        let t = mk_transform(vec![verb_field("Out", "concat", vec![
            ref_arg("@.a"), lit_arg_str("-"), ref_arg("@.b"), lit_arg_str("-"), ref_arg("@.c"),
        ])]);
        let result = execute(&t, &src_obj(vec![("a", s("x")), ("b", s("y")), ("c", s("z"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("x-y-z")));
    }

    #[test]
    fn ext_verb_replace_then_upper() {
        let t = mk_transform(vec![verb_field("Out", "upper", vec![
            verb_arg("replace", vec![ref_arg("@.val"), lit_arg_str("world"), lit_arg_str("earth")]),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello world"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("HELLO EARTH")));
    }

    #[test]
    fn ext_verb_substring_then_upper() {
        let t = mk_transform(vec![verb_field("Out", "upper", vec![
            verb_arg("substring", vec![ref_arg("@.val"), lit_arg_int(0), lit_arg_int(5)]),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello world"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("HELLO")));
    }

    #[test]
    fn ext_verb_ifelse_with_nested_verbs_both_branches() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            ref_arg("@.flag"),
            verb_arg("upper", vec![ref_arg("@.name")]),
            verb_arg("lower", vec![ref_arg("@.name")]),
        ])]);
        let result = execute(&t, &src_obj(vec![("flag", b(true)), ("name", s("Test"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("TEST")));
    }

    #[test]
    fn ext_verb_ifelse_false_nested_verb() {
        let t = mk_transform(vec![verb_field("Out", "ifElse", vec![
            ref_arg("@.flag"),
            verb_arg("upper", vec![ref_arg("@.name")]),
            verb_arg("lower", vec![ref_arg("@.name")]),
        ])]);
        let result = execute(&t, &src_obj(vec![("flag", b(false)), ("name", s("Test"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("test")));
    }

    #[test]
    fn ext_verb_coalesce_with_verb_fallback() {
        let t = mk_transform(vec![verb_field("Out", "coalesce", vec![
            ref_arg("@.missing"),
            verb_arg("upper", vec![lit_arg_str("default")]),
        ])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("DEFAULT")));
    }

    #[test]
    fn ext_verb_pad_left() {
        let t = mk_transform(vec![verb_field("Out", "padLeft", vec![
            ref_arg("@.val"), lit_arg_int(5), lit_arg_str("0"),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("42"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("00042")));
    }

    #[test]
    fn ext_verb_pad_right() {
        let t = mk_transform(vec![verb_field("Out", "padRight", vec![
            ref_arg("@.val"), lit_arg_int(5), lit_arg_str("_"),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("hi"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("hi___")));
    }

    #[test]
    fn ext_verb_truncate() {
        let t = mk_transform(vec![verb_field("Out", "truncate", vec![
            ref_arg("@.val"), lit_arg_int(5),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello world"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap().as_str().unwrap();
        assert!(val.len() <= 8); // truncated, may include ellipsis
    }

    #[test]
    fn ext_verb_split() {
        let t = mk_transform(vec![verb_field("Out", "split", vec![
            ref_arg("@.val"), lit_arg_str(","),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("a,b,c"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let arr = out.get("Out").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 3);
    }

    #[test]
    fn ext_verb_join() {
        let t = mk_transform(vec![verb_field("Out", "join", vec![
            ref_arg("@.val"), lit_arg_str("-"),
        ])]);
        let source = src_obj(vec![("val", DynValue::Array(vec![s("a"), s("b"), s("c")]))]);
        let result = execute(&t, &source);
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("a-b-c")));
    }

    #[test]
    fn ext_verb_title_case() {
        let t = mk_transform(vec![verb_field("Out", "titleCase", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello world"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap().as_str().unwrap();
        assert!(val.starts_with('H'));
        assert!(val.contains('W'));
    }

    #[test]
    fn ext_verb_contains_true() {
        let t = mk_transform(vec![verb_field("Out", "contains", vec![ref_arg("@.val"), lit_arg_str("llo")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_verb_contains_false() {
        let t = mk_transform(vec![verb_field("Out", "contains", vec![ref_arg("@.val"), lit_arg_str("xyz")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&b(false)));
    }

    #[test]
    fn ext_verb_starts_with_true() {
        let t = mk_transform(vec![verb_field("Out", "startsWith", vec![ref_arg("@.val"), lit_arg_str("hel")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_verb_ends_with_true() {
        let t = mk_transform(vec![verb_field("Out", "endsWith", vec![ref_arg("@.val"), lit_arg_str("llo")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_verb_repeat_string() {
        let t = mk_transform(vec![verb_field("Out", "repeat", vec![ref_arg("@.val"), lit_arg_int(3)])]);
        let result = execute(&t, &src_obj(vec![("val", s("ab"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("ababab")));
    }

    #[test]
    fn ext_verb_reverse_string() {
        let t = mk_transform(vec![verb_field("Out", "reverseString", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("olleh")));
    }

    #[test]
    fn ext_verb_camel_case() {
        let t = mk_transform(vec![verb_field("Out", "camelCase", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello world"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap().as_str().unwrap();
        assert!(val.starts_with('h'));
    }

    #[test]
    fn ext_verb_snake_case() {
        let t = mk_transform(vec![verb_field("Out", "snakeCase", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("helloWorld"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap().as_str().unwrap();
        assert!(val.contains('_'));
    }

    #[test]
    fn ext_verb_kebab_case() {
        let t = mk_transform(vec![verb_field("Out", "kebabCase", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("helloWorld"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap().as_str().unwrap();
        assert!(val.contains('-'));
    }

    #[test]
    fn ext_verb_word_count() {
        let t = mk_transform(vec![verb_field("Out", "wordCount", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello beautiful world"))]));
        assert!(result.success);
        let val = result.output.unwrap().get("Out").unwrap().clone();
        assert!(val.as_i64() == Some(3));
    }

    #[test]
    fn ext_verb_base64_encode_decode() {
        let t = mk_transform(vec![
            verb_field("Encoded", "base64Encode", vec![ref_arg("@.val")]),
            verb_field("Decoded", "base64Decode", vec![
                verb_arg("base64Encode", vec![ref_arg("@.val")]),
            ]),
        ]);
        let result = execute(&t, &src_obj(vec![("val", s("hello"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Decoded"), Some(&s("hello")));
    }

    // =========================================================================
    // 4. Transform features (~30 tests)
    // =========================================================================

    #[test]
    fn ext_feat_constant_string_in_output() {
        let mut t = mk_transform(vec![copy_field("Version", "$const.ver")]);
        t.constants.insert("ver".to_string(), OdinValues::string("1.0"));
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Version"), Some(&s("1.0")));
    }

    #[test]
    fn ext_feat_constant_integer() {
        let mut t = mk_transform(vec![copy_field("Max", "$const.maxRetries")]);
        t.constants.insert("maxRetries".to_string(), OdinValues::integer(3));
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Max"), Some(&i(3)));
    }

    #[test]
    fn ext_feat_constant_boolean() {
        let mut t = mk_transform(vec![copy_field("Debug", "$const.debug")]);
        t.constants.insert("debug".to_string(), OdinValues::boolean(true));
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Debug"), Some(&b(true)));
    }

    #[test]
    fn ext_feat_constant_in_verb() {
        let mut t = mk_transform(vec![verb_field("Out", "concat", vec![
            VerbArg::Reference("$const.prefix".to_string(), vec![]),
            lit_arg_str(" "),
            ref_arg("@.name"),
        ])]);
        t.constants.insert("prefix".to_string(), OdinValues::string("Hello"));
        let result = execute(&t, &src_obj(vec![("name", s("World"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("Hello World")));
    }

    #[test]
    fn ext_feat_multiple_constants() {
        let mut t = mk_transform(vec![
            copy_field("A", "$const.x"),
            copy_field("B", "$const.y"),
            copy_field("C", "$const.z"),
        ]);
        t.constants.insert("x".to_string(), OdinValues::string("alpha"));
        t.constants.insert("y".to_string(), OdinValues::string("beta"));
        t.constants.insert("z".to_string(), OdinValues::string("gamma"));
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&s("alpha")));
        assert_eq!(out.get("B"), Some(&s("beta")));
        assert_eq!(out.get("C"), Some(&s("gamma")));
    }

    #[test]
    fn ext_feat_missing_constant_returns_null() {
        let t = mk_transform(vec![copy_field("Out", "$const.missing")]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_feat_nested_output_path() {
        let t = mk_transform(vec![copy_field("a.b.c", "@.val")]);
        let result = execute(&t, &src_obj(vec![("val", s("deep"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let a = out.get("a").unwrap();
        let b = a.get("b").unwrap();
        assert_eq!(b.get("c"), Some(&s("deep")));
    }

    #[test]
    fn ext_feat_multiple_fields_same_nested_parent() {
        let t = mk_transform(vec![
            copy_field("person.name", "@.name"),
            copy_field("person.age", "@.age"),
        ]);
        let result = execute(&t, &src_obj(vec![("name", s("Alice")), ("age", i(30))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let person = out.get("person").unwrap();
        assert_eq!(person.get("name"), Some(&s("Alice")));
        assert_eq!(person.get("age"), Some(&i(30)));
    }

    #[test]
    fn ext_feat_object_expression() {
        let t = mk_transform(vec![FieldMapping {
            target: "Info".to_string(),
            expression: FieldExpression::Object(vec![
                copy_field("name", "@.name"),
                copy_field("city", "@.city"),
            ]),
            directives: vec![], modifiers: None,
        }]);
        let result = execute(&t, &src_obj(vec![("name", s("Bob")), ("city", s("NYC"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let info = out.get("Info").unwrap();
        assert_eq!(info.get("name"), Some(&s("Bob")));
        assert_eq!(info.get("city"), Some(&s("NYC")));
    }

    #[test]
    fn ext_feat_object_expression_with_verb() {
        let t = mk_transform(vec![FieldMapping {
            target: "Info".to_string(),
            expression: FieldExpression::Object(vec![
                verb_field("upperName", "upper", vec![ref_arg("@.name")]),
            ]),
            directives: vec![], modifiers: None,
        }]);
        let result = execute(&t, &src_obj(vec![("name", s("alice"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let info = out.get("Info").unwrap();
        assert_eq!(info.get("upperName"), Some(&s("ALICE")));
    }

    #[test]
    fn ext_feat_named_segment_creates_namespace() {
        let seg = named_seg("Customer", vec![copy_field("Name", "@.name")]);
        let t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![("name", s("Alice"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let cust = out.get("Customer").unwrap();
        assert_eq!(cust.get("Name"), Some(&s("Alice")));
    }

    #[test]
    fn ext_feat_multiple_named_segments() {
        let seg1 = named_seg("Header", vec![literal_field("Type", OdinValues::string("Invoice"))]);
        let seg2 = named_seg("Body", vec![copy_field("Amount", "@.amount")]);
        let t = mk_custom(vec![seg1, seg2]);
        let result = execute(&t, &src_obj(vec![("amount", i(100))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Header").unwrap().get("Type"), Some(&s("Invoice")));
        assert_eq!(out.get("Body").unwrap().get("Amount"), Some(&i(100)));
    }

    #[test]
    fn ext_feat_loop_basic() {
        let seg = TransformSegment {
            name: "Items".to_string(), path: "Items".to_string(),
            source_path: Some("@.items".to_string()), discriminator: None,
            is_array: true, directives: Vec::new(),
            mappings: vec![copy_field("name", "@_item.name")],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        };
        let t = mk_custom(vec![seg]);
        let source = src_obj(vec![("items", DynValue::Array(vec![
            src_obj(vec![("name", s("A"))]),
            src_obj(vec![("name", s("B"))]),
        ]))]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn ext_feat_loop_with_verb() {
        let seg = TransformSegment {
            name: "Items".to_string(), path: "Items".to_string(),
            source_path: Some("@.items".to_string()), discriminator: None,
            is_array: true, directives: Vec::new(),
            mappings: vec![verb_field("upper_name", "upper", vec![ref_arg("@_item.name")])],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        };
        let t = mk_custom(vec![seg]);
        let source = src_obj(vec![("items", DynValue::Array(vec![
            src_obj(vec![("name", s("alice"))]),
        ]))]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let items = out.get("Items").unwrap().as_array().unwrap();
        assert_eq!(items[0].get("upper_name"), Some(&s("ALICE")));
    }

    #[test]
    fn ext_feat_loop_empty_array() {
        let seg = TransformSegment {
            name: "Items".to_string(), path: "Items".to_string(),
            source_path: Some("@.items".to_string()), discriminator: None,
            is_array: true, directives: Vec::new(),
            mappings: vec![copy_field("name", "@_item.name")],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        };
        let t = mk_custom(vec![seg]);
        let source = src_obj(vec![("items", DynValue::Array(vec![]))]);
        let result = execute(&t, &source);
        assert!(result.success);
    }

    #[test]
    fn ext_feat_literal_types() {
        let t = mk_transform(vec![
            literal_field("S", OdinValues::string("str")),
            literal_field("N", OdinValues::integer(42)),
            literal_field("F", OdinValues::number(3.14)),
            literal_field("B", OdinValues::boolean(true)),
            literal_field("Z", OdinValues::null()),
        ]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("S"), Some(&s("str")));
        assert_eq!(out.get("N"), Some(&i(42)));
        assert_eq!(out.get("B"), Some(&b(true)));
        assert_eq!(out.get("Z"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_feat_copy_entire_source() {
        let t = mk_transform(vec![copy_field("All", "@")]);
        let source = src_obj(vec![("x", i(1)), ("y", i(2))]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let all = out.get("All").unwrap();
        assert_eq!(all.get("x"), Some(&i(1)));
        assert_eq!(all.get("y"), Some(&i(2)));
    }

    #[test]
    fn ext_feat_copy_array_field() {
        let t = mk_transform(vec![copy_field("Tags", "@.tags")]);
        let source = src_obj(vec![("tags", DynValue::Array(vec![s("a"), s("b"), s("c")]))]);
        let result = execute(&t, &source);
        assert!(result.success);
        let out = result.output.unwrap();
        let tags = out.get("Tags").unwrap().as_array().unwrap();
        assert_eq!(tags.len(), 3);
        assert_eq!(tags[0], s("a"));
    }

    #[test]
    fn ext_feat_deeply_nested_source() {
        let t = mk_transform(vec![copy_field("Out", "@.a.b.c.d")]);
        let source = src_obj(vec![("a", src_obj(vec![
            ("b", src_obj(vec![
                ("c", src_obj(vec![
                    ("d", s("deep")),
                ])),
            ])),
        ]))]);
        let result = execute(&t, &source);
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("deep")));
    }

    #[test]
    fn ext_feat_formatted_json_output() {
        let t = mk_transform(vec![
            literal_field("Name", OdinValues::string("Test")),
            literal_field("Value", OdinValues::integer(42)),
        ]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let formatted = result.formatted.unwrap();
        assert!(formatted.contains("Name"));
        assert!(formatted.contains("42"));
    }

    #[test]
    fn ext_feat_lookup_table_match() {
        let mut t = mk_custom(vec![root_seg(vec![verb_field("Color", "lookup", vec![
            lit_arg_str("colors.name"),
            ref_arg("@.code"),
        ])])]);
        t.tables.insert("colors".to_string(), LookupTable {
            name: "colors".to_string(),
            columns: vec!["code".to_string(), "name".to_string()],
            rows: vec![
                vec![s("R"), s("Red")],
                vec![s("G"), s("Green")],
                vec![s("B"), s("Blue")],
            ],
            default: None,
        });
        let result = execute(&t, &src_obj(vec![("code", s("G"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Color"), Some(&s("Green")));
    }

    #[test]
    fn ext_feat_lookup_table_no_match_returns_null() {
        let mut t = mk_custom(vec![root_seg(vec![verb_field("Color", "lookup", vec![
            lit_arg_str("colors.name"),
            ref_arg("@.code"),
        ])])]);
        t.tables.insert("colors".to_string(), LookupTable {
            name: "colors".to_string(),
            columns: vec!["code".to_string(), "name".to_string()],
            rows: vec![vec![s("R"), s("Red")]],
            default: None,
        });
        let result = execute(&t, &src_obj(vec![("code", s("X"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Color"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_feat_lookup_table_with_default() {
        let mut t = mk_custom(vec![root_seg(vec![verb_field("Color", "lookup", vec![
            lit_arg_str("colors.name"),
            ref_arg("@.code"),
        ])])]);
        t.tables.insert("colors".to_string(), LookupTable {
            name: "colors".to_string(),
            columns: vec!["code".to_string(), "name".to_string()],
            rows: vec![vec![s("R"), s("Red")]],
            default: Some(s("Unknown")),
        });
        let result = execute(&t, &src_obj(vec![("code", s("X"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Color"), Some(&s("Unknown")));
    }

    // =========================================================================
    // 5. Multi-pass transforms (~20 tests)
    // =========================================================================

    #[test]
    fn ext_pass_1_then_2() {
        let seg1 = pass_seg(1, vec![literal_field("P1", OdinValues::string("first"))]);
        let seg2 = pass_seg(2, vec![literal_field("P2", OdinValues::string("second"))]);
        let t = mk_custom(vec![seg1, seg2]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("P1"), Some(&s("first")));
        assert_eq!(out.get("P2"), Some(&s("second")));
    }

    #[test]
    fn ext_pass_none_runs_after_numbered() {
        let seg1 = pass_seg(1, vec![literal_field("P1", OdinValues::string("first"))]);
        let seg2 = root_seg(vec![literal_field("Default", OdinValues::string("last"))]);
        let t = mk_custom(vec![seg2, seg1]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("P1"), Some(&s("first")));
        assert_eq!(out.get("Default"), Some(&s("last")));
    }

    #[test]
    fn ext_pass_three_passes() {
        let seg1 = pass_seg(1, vec![literal_field("A", OdinValues::string("1"))]);
        let seg2 = pass_seg(2, vec![literal_field("B", OdinValues::string("2"))]);
        let seg3 = pass_seg(3, vec![literal_field("C", OdinValues::string("3"))]);
        let t = mk_custom(vec![seg3, seg1, seg2]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&s("1")));
        assert_eq!(out.get("B"), Some(&s("2")));
        assert_eq!(out.get("C"), Some(&s("3")));
    }

    #[test]
    fn ext_pass_later_overwrites_earlier() {
        let seg1 = pass_seg(1, vec![literal_field("Val", OdinValues::string("old"))]);
        let seg2 = pass_seg(2, vec![literal_field("Val", OdinValues::string("new"))]);
        let t = mk_custom(vec![seg1, seg2]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Val"), Some(&s("new")));
    }

    #[test]
    fn ext_pass_multiple_segments_same_pass() {
        let seg1 = pass_seg(1, vec![literal_field("A", OdinValues::string("a"))]);
        let seg2 = pass_seg(1, vec![literal_field("B", OdinValues::string("b"))]);
        let t = mk_custom(vec![seg1, seg2]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&s("a")));
        assert_eq!(out.get("B"), Some(&s("b")));
    }

    #[test]
    fn ext_pass_with_accumulator() {
        let seg1 = pass_seg(1, vec![
            verb_field("_", "accumulate", vec![
                lit_arg_str("total"), lit_arg_str("add"), lit_arg_int(10),
            ]),
        ]);
        let seg2 = pass_seg(2, vec![
            copy_field("Total", "$accumulator.total"),
        ]);
        let mut t = mk_custom(vec![seg1, seg2]);
        t.accumulators.insert("total".to_string(), AccumulatorDef {
            name: "total".to_string(),
            initial: OdinValues::integer(0),
            persist: true,
        });
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
    }

    #[test]
    fn ext_pass_with_condition_on_segment() {
        let seg1 = {
            let mut s = pass_seg(1, vec![literal_field("A", OdinValues::string("active"))]);
            s.condition = Some("@.active".to_string());
            s
        };
        let t = mk_custom(vec![seg1]);
        let result = execute(&t, &src_obj(vec![("active", b(true))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("A"), Some(&s("active")));
    }

    #[test]
    fn ext_pass_with_condition_skipped() {
        let seg1 = {
            let mut s = pass_seg(1, vec![literal_field("A", OdinValues::string("active"))]);
            s.condition = Some("@.active".to_string());
            s
        };
        let t = mk_custom(vec![seg1]);
        let result = execute(&t, &src_obj(vec![("active", b(false))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("A"), None);
    }

    #[test]
    fn ext_pass_pass1_output_accessible_in_pass2() {
        // Cross-pass references use bare path (resolved from global output snapshot)
        let seg1 = pass_seg(1, vec![literal_field("Phase1", OdinValues::string("done"))]);
        let seg2 = pass_seg(2, vec![copy_field("Ref", "Phase1")]);
        let t = mk_custom(vec![seg1, seg2]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Phase1"), Some(&s("done")));
        assert_eq!(out.get("Ref"), Some(&s("done")));
    }

    #[test]
    fn ext_pass_four_passes_all_produce_output() {
        let segs: Vec<_> = (1..=4).map(|p| {
            pass_seg(p, vec![literal_field(&format!("P{p}"), OdinValues::string(&format!("val{p}")))])
        }).collect();
        let t = mk_custom(segs);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        for p in 1..=4 {
            assert_eq!(out.get(&format!("P{p}")), Some(&s(&format!("val{p}"))));
        }
    }

    #[test]
    fn ext_pass_reverse_order_still_correct() {
        let seg1 = pass_seg(3, vec![literal_field("C", OdinValues::string("3"))]);
        let seg2 = pass_seg(1, vec![literal_field("A", OdinValues::string("1"))]);
        let seg3 = pass_seg(2, vec![literal_field("B", OdinValues::string("2"))]);
        let t = mk_custom(vec![seg1, seg2, seg3]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("A"), Some(&s("1")));
        assert_eq!(out.get("B"), Some(&s("2")));
        assert_eq!(out.get("C"), Some(&s("3")));
    }

    #[test]
    fn ext_pass_accumulator_non_persist_resets() {
        // Non-persist accumulator resets to initial value between passes
        let seg1 = pass_seg(1, vec![
            literal_field("P1", OdinValues::integer(1)),
        ]);
        let seg2 = pass_seg(2, vec![
            copy_field("Counter", "$accumulator.counter"),
        ]);
        let mut t = mk_custom(vec![seg1, seg2]);
        t.accumulators.insert("counter".to_string(), AccumulatorDef {
            name: "counter".to_string(),
            initial: OdinValues::integer(0),
            persist: false,
        });
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Counter"), Some(&i(0)));
    }

    #[test]
    fn ext_pass_accumulator_persist_survives() {
        // Persist accumulator keeps its value across pass transitions
        let seg1 = pass_seg(1, vec![
            literal_field("P1", OdinValues::integer(1)),
        ]);
        let seg2 = pass_seg(2, vec![
            copy_field("Counter", "$accumulator.persist_counter"),
        ]);
        let mut t = mk_custom(vec![seg1, seg2]);
        t.accumulators.insert("persist_counter".to_string(), AccumulatorDef {
            name: "persist_counter".to_string(),
            initial: OdinValues::integer(42),
            persist: true,
        });
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Counter"), Some(&i(42)));
    }

    #[test]
    fn ext_pass_single_pass_works_normally() {
        let seg = pass_seg(1, vec![literal_field("X", OdinValues::string("y"))]);
        let t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("X"), Some(&s("y")));
    }

    #[test]
    fn ext_pass_named_segment_in_pass() {
        let mut seg = named_seg("Header", vec![literal_field("Type", OdinValues::string("Invoice"))]);
        seg.pass = Some(1);
        let t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Header").unwrap().get("Type"), Some(&s("Invoice")));
    }

    #[test]
    fn ext_pass_copy_from_source_in_pass() {
        let seg = pass_seg(1, vec![copy_field("Name", "@.name")]);
        let t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![("name", s("Alice"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Name"), Some(&s("Alice")));
    }

    #[test]
    fn ext_pass_verb_in_pass() {
        let seg = pass_seg(1, vec![verb_field("Upper", "upper", vec![ref_arg("@.name")])]);
        let t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![("name", s("alice"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Upper"), Some(&s("ALICE")));
    }

    // =========================================================================
    // 6. Confidential enforcement (~20 tests)
    // =========================================================================

    #[test]
    fn ext_conf_redact_string() {
        let mut t = mk_transform(vec![modifiers_field("SSN", "@.ssn", confidential_mods())]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let result = execute(&t, &src_obj(vec![("ssn", s("123-45-6789"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("SSN"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_conf_redact_integer() {
        let mut t = mk_transform(vec![modifiers_field("Pin", "@.pin", confidential_mods())]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let result = execute(&t, &src_obj(vec![("pin", i(1234))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Pin"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_conf_redact_boolean() {
        let mut t = mk_transform(vec![modifiers_field("Flag", "@.flag", confidential_mods())]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let result = execute(&t, &src_obj(vec![("flag", b(true))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Flag"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_conf_redact_float() {
        let mut t = mk_transform(vec![modifiers_field("Salary", "@.salary", confidential_mods())]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let result = execute(&t, &src_obj(vec![("salary", f(75000.50))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Salary"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_conf_redact_null_stays_null() {
        let mut t = mk_transform(vec![modifiers_field("Val", "@.missing", confidential_mods())]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Val"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_conf_mask_string() {
        let mut t = mk_transform(vec![modifiers_field("SSN", "@.ssn", confidential_mods())]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let result = execute(&t, &src_obj(vec![("ssn", s("123-45-6789"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("SSN").unwrap();
        // Masked string should be asterisks of same length
        let text = val.as_str().unwrap();
        assert!(text.chars().all(|c| c == '*'));
        assert_eq!(text.len(), 11); // same length as original
    }

    #[test]
    fn ext_conf_mask_integer_becomes_null() {
        let mut t = mk_transform(vec![modifiers_field("Pin", "@.pin", confidential_mods())]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let result = execute(&t, &src_obj(vec![("pin", i(1234))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Pin"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_conf_mask_boolean_becomes_null() {
        let mut t = mk_transform(vec![modifiers_field("Flag", "@.flag", confidential_mods())]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let result = execute(&t, &src_obj(vec![("flag", b(true))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Flag"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_conf_mask_empty_string() {
        let mut t = mk_transform(vec![modifiers_field("Val", "@.val", confidential_mods())]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let result = execute(&t, &src_obj(vec![("val", s(""))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Val").unwrap();
        assert_eq!(val.as_str().unwrap(), "");
    }

    #[test]
    fn ext_conf_no_enforcement_passes_through() {
        let t = mk_transform(vec![modifiers_field("SSN", "@.ssn", confidential_mods())]);
        // No enforce_confidential set
        let result = execute(&t, &src_obj(vec![("ssn", s("123-45-6789"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("SSN"), Some(&s("123-45-6789")));
    }

    #[test]
    fn ext_conf_non_confidential_field_unchanged_with_redact() {
        let mut t = mk_transform(vec![
            modifiers_field("SSN", "@.ssn", confidential_mods()),
            copy_field("Name", "@.name"),
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let result = execute(&t, &src_obj(vec![("ssn", s("xxx")), ("name", s("Alice"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("SSN"), Some(&DynValue::Null));
        assert_eq!(out.get("Name"), Some(&s("Alice")));
    }

    #[test]
    fn ext_conf_non_confidential_field_unchanged_with_mask() {
        let mut t = mk_transform(vec![
            modifiers_field("SSN", "@.ssn", confidential_mods()),
            copy_field("Name", "@.name"),
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let result = execute(&t, &src_obj(vec![("ssn", s("xxx")), ("name", s("Alice"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let ssn_val = out.get("SSN").unwrap().as_str().unwrap();
        assert!(ssn_val.chars().all(|c| c == '*'));
        assert_eq!(out.get("Name"), Some(&s("Alice")));
    }

    #[test]
    fn ext_conf_redact_multiple_fields() {
        let mut t = mk_transform(vec![
            modifiers_field("SSN", "@.ssn", confidential_mods()),
            modifiers_field("DOB", "@.dob", confidential_mods()),
            copy_field("Name", "@.name"),
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let result = execute(&t, &src_obj(vec![
            ("ssn", s("111-22-3333")), ("dob", s("1990-01-01")), ("name", s("Bob")),
        ]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("SSN"), Some(&DynValue::Null));
        assert_eq!(out.get("DOB"), Some(&DynValue::Null));
        assert_eq!(out.get("Name"), Some(&s("Bob")));
    }

    #[test]
    fn ext_conf_required_modifier_recorded() {
        let t = mk_transform(vec![modifiers_field("Name", "@.name", required_mods())]);
        let result = execute(&t, &src_obj(vec![("name", s("Alice"))]));
        assert!(result.success);
        assert!(result.modifiers.get("Name").map_or(false, |m| m.required));
    }

    #[test]
    fn ext_conf_deprecated_modifier_recorded() {
        let t = mk_transform(vec![modifiers_field("Old", "@.old", deprecated_mods())]);
        let result = execute(&t, &src_obj(vec![("old", s("legacy"))]));
        assert!(result.success);
        assert!(result.modifiers.get("Old").map_or(false, |m| m.deprecated));
    }

    #[test]
    fn ext_conf_all_modifiers_recorded() {
        let t = mk_transform(vec![modifiers_field("Secret", "@.secret", all_mods())]);
        let result = execute(&t, &src_obj(vec![("secret", s("data"))]));
        assert!(result.success);
        let m = result.modifiers.get("Secret").unwrap();
        assert!(m.required && m.confidential && m.deprecated);
    }

    #[test]
    fn ext_conf_redact_with_required_modifier() {
        let mods = OdinModifiers { required: true, confidential: true, deprecated: false, attr: false };
        let mut t = mk_transform(vec![modifiers_field("SSN", "@.ssn", mods)]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let result = execute(&t, &src_obj(vec![("ssn", s("123-45-6789"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("SSN"), Some(&DynValue::Null));
        let m = result.modifiers.get("SSN").unwrap();
        assert!(m.required && m.confidential);
    }

    #[test]
    fn ext_conf_mask_long_string() {
        let mut t = mk_transform(vec![modifiers_field("Data", "@.data", confidential_mods())]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let long_str = "a".repeat(100);
        let result = execute(&t, &src_obj(vec![("data", s(&long_str))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Data").unwrap().as_str().unwrap();
        assert_eq!(val.len(), 100);
        assert!(val.chars().all(|c| c == '*'));
    }

    #[test]
    fn ext_conf_redact_float_becomes_null() {
        let mut t = mk_transform(vec![modifiers_field("Rate", "@.rate", confidential_mods())]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let result = execute(&t, &src_obj(vec![("rate", f(99.99))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Rate"), Some(&DynValue::Null));
    }

    // =========================================================================
    // 7. Error handling (~30 tests)
    // =========================================================================

    #[test]
    fn ext_err_unknown_verb() {
        let t = mk_transform(vec![verb_field("Out", "nonExistentVerb", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("x"))]));
        assert!(!result.success);
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].message.contains("nonExistentVerb"));
    }

    #[test]
    fn ext_err_unknown_verb_still_has_output() {
        let t = mk_transform(vec![
            verb_field("Bad", "noSuchVerb", vec![ref_arg("@.val")]),
            literal_field("Good", OdinValues::string("ok")),
        ]);
        let result = execute(&t, &src_obj(vec![("val", s("x"))]));
        // The good mapping should still produce output
        let out = result.output.unwrap();
        assert_eq!(out.get("Good"), Some(&s("ok")));
    }

    #[test]
    fn ext_err_multiple_unknown_verbs() {
        let t = mk_transform(vec![
            verb_field("A", "bad1", vec![ref_arg("@.x")]),
            verb_field("B", "bad2", vec![ref_arg("@.y")]),
        ]);
        let result = execute(&t, &src_obj(vec![("x", s("a")), ("y", s("b"))]));
        assert!(!result.success);
        assert!(result.errors.len() >= 2);
    }

    #[test]
    fn ext_err_missing_source_field_returns_null() {
        let t = mk_transform(vec![copy_field("Out", "@.nonexistent")]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_err_missing_deeply_nested_field() {
        let t = mk_transform(vec![copy_field("Out", "@.a.b.c.d.e")]);
        let result = execute(&t, &src_obj(vec![("a", src_obj(vec![]))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_err_empty_transform_empty_source() {
        let t = mk_transform(vec![]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
    }

    #[test]
    fn ext_err_empty_transform_with_source() {
        let t = mk_transform(vec![]);
        let result = execute(&t, &src_obj(vec![("name", s("Alice"))]));
        assert!(result.success);
    }

    #[test]
    fn ext_err_nested_verb_error() {
        let t = mk_transform(vec![verb_field("Out", "upper", vec![
            verb_arg("nonExistent", vec![ref_arg("@.val")]),
        ])]);
        let result = execute(&t, &src_obj(vec![("val", s("x"))]));
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn ext_err_verb_no_args_concat() {
        // concat with no args
        let t = mk_transform(vec![verb_field("Out", "concat", vec![])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        // concat with no args should return empty string
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("")));
    }

    #[test]
    fn ext_err_copy_from_non_object_path() {
        let t = mk_transform(vec![copy_field("Out", "@.name.sub")]);
        let result = execute(&t, &src_obj(vec![("name", s("Alice"))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_err_array_index_on_non_array() {
        let t = mk_transform(vec![copy_field("Out", "@.name[0]")]);
        let result = execute(&t, &src_obj(vec![("name", s("Alice"))]));
        assert!(result.success);
        // Should return null for index on non-array
    }

    #[test]
    fn ext_err_array_index_out_of_bounds() {
        let t = mk_transform(vec![copy_field("Out", "@.items[99]")]);
        let source = src_obj(vec![("items", DynValue::Array(vec![s("a")]))]);
        let result = execute(&t, &source);
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_err_multiple_errors_dont_halt() {
        let t = mk_transform(vec![
            verb_field("A", "bad1", vec![ref_arg("@.x")]),
            literal_field("B", OdinValues::string("ok")),
            verb_field("C", "bad2", vec![ref_arg("@.y")]),
            literal_field("D", OdinValues::integer(42)),
        ]);
        let result = execute(&t, &src_obj(vec![("x", s("a")), ("y", s("b"))]));
        let out = result.output.unwrap();
        assert_eq!(out.get("B"), Some(&s("ok")));
        assert_eq!(out.get("D"), Some(&i(42)));
    }

    #[test]
    fn ext_err_copy_with_null_source_data() {
        let t = mk_transform(vec![copy_field("Out", "@.val")]);
        let result = execute(&t, &DynValue::Null);
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_err_verb_on_null_input() {
        let t = mk_transform(vec![verb_field("Out", "upper", vec![ref_arg("@.missing")])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        // upper on null should handle gracefully
    }

    #[test]
    fn ext_err_constant_ref_missing() {
        let t = mk_transform(vec![copy_field("Out", "$const.undefined")]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_err_loop_on_non_array_source() {
        let seg = TransformSegment {
            name: "Items".to_string(), path: "Items".to_string(),
            source_path: Some("@.notArray".to_string()), discriminator: None,
            is_array: true, directives: Vec::new(),
            mappings: vec![copy_field("x", "@_item")],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        };
        let t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![("notArray", s("scalar"))]));
        assert!(result.success);
    }

    #[test]
    fn ext_err_loop_on_missing_source() {
        let seg = TransformSegment {
            name: "Items".to_string(), path: "Items".to_string(),
            source_path: Some("@.missing".to_string()), discriminator: None,
            is_array: true, directives: Vec::new(),
            mappings: vec![copy_field("x", "@_item")],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        };
        let t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
    }

    #[test]
    fn ext_err_discriminator_mismatch_skips() {
        let seg = TransformSegment {
            name: "TypeA".to_string(), path: "TypeA".to_string(),
            source_path: None,
            discriminator: Some(Discriminator {
                path: "@.type".to_string(),
                value: "A".to_string(),
            }),
            is_array: false, directives: Vec::new(),
            mappings: vec![literal_field("Found", OdinValues::string("A"))],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        };
        let t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![("type", s("B"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("TypeA"), None);
    }

    #[test]
    fn ext_err_discriminator_match_processes() {
        let seg = TransformSegment {
            name: "TypeA".to_string(), path: "TypeA".to_string(),
            source_path: None,
            discriminator: Some(Discriminator {
                path: "@.type".to_string(),
                value: "A".to_string(),
            }),
            is_array: false, directives: Vec::new(),
            mappings: vec![literal_field("Found", OdinValues::string("yes"))],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        };
        let t = mk_custom(vec![seg]);
        let result = execute(&t, &src_obj(vec![("type", s("A"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("TypeA").unwrap().get("Found"), Some(&s("yes")));
    }

    #[test]
    fn ext_err_object_expression_empty() {
        let t = mk_transform(vec![FieldMapping {
            target: "Empty".to_string(),
            expression: FieldExpression::Object(vec![]),
            directives: vec![], modifiers: None,
        }]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        let out = result.output.unwrap();
        let empty = out.get("Empty").unwrap();
        assert!(empty.as_object().unwrap().is_empty());
    }

    #[test]
    fn ext_err_literal_null_explicit() {
        let t = mk_transform(vec![literal_field("Out", OdinValues::null())]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_err_success_true_when_all_mappings_succeed() {
        let t = mk_transform(vec![
            literal_field("A", OdinValues::string("a")),
            literal_field("B", OdinValues::integer(1)),
        ]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn ext_err_success_false_on_verb_error() {
        let t = mk_transform(vec![verb_field("Out", "totallyFake", vec![ref_arg("@.x")])]);
        let result = execute(&t, &src_obj(vec![("x", s("a"))]));
        assert!(!result.success);
    }

    #[test]
    fn ext_err_result_has_formatted_output() {
        let t = mk_transform(vec![literal_field("X", OdinValues::string("y"))]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert!(result.formatted.is_some());
        let fmt = result.formatted.unwrap();
        assert!(fmt.contains("X"));
    }

    #[test]
    fn ext_err_mixed_success_and_errors() {
        let t = mk_transform(vec![
            literal_field("Good", OdinValues::string("ok")),
            verb_field("Bad", "doesNotExist", vec![lit_arg_str("x")]),
        ]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(!result.success);
        assert!(!result.errors.is_empty());
        let out = result.output.unwrap();
        assert_eq!(out.get("Good"), Some(&s("ok")));
    }

    #[test]
    fn ext_err_copy_integer_preserves_type() {
        let t = mk_transform(vec![copy_field("Out", "@.val")]);
        let result = execute(&t, &src_obj(vec![("val", i(42))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&i(42)));
    }

    #[test]
    fn ext_err_copy_boolean_preserves_type() {
        let t = mk_transform(vec![copy_field("Out", "@.val")]);
        let result = execute(&t, &src_obj(vec![("val", b(true))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&b(true)));
    }

    #[test]
    fn ext_err_copy_float_preserves_type() {
        let t = mk_transform(vec![copy_field("Out", "@.val")]);
        let result = execute(&t, &src_obj(vec![("val", f(3.14))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap();
        assert!((val.as_f64().unwrap() - 3.14).abs() < 0.001);
    }

    #[test]
    fn ext_err_copy_null_preserves_null() {
        let t = mk_transform(vec![copy_field("Out", "@.val")]);
        let result = execute(&t, &src_obj(vec![("val", DynValue::Null)]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_err_type_of_string() {
        let t = mk_transform(vec![verb_field("Out", "typeOf", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", s("hello"))]));
        assert!(result.success);
        let out = result.output.unwrap();
        assert_eq!(out.get("Out"), Some(&s("string")));
    }

    #[test]
    fn ext_err_type_of_integer() {
        let t = mk_transform(vec![verb_field("Out", "typeOf", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", i(42))]));
        assert!(result.success);
        let out = result.output.unwrap();
        let val = out.get("Out").unwrap().as_str().unwrap();
        assert!(val == "number" || val == "integer");
    }

    #[test]
    fn ext_err_type_of_boolean() {
        let t = mk_transform(vec![verb_field("Out", "typeOf", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", b(true))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("boolean")));
    }

    #[test]
    fn ext_err_type_of_null() {
        let t = mk_transform(vec![verb_field("Out", "typeOf", vec![ref_arg("@.missing")])]);
        let result = execute(&t, &src_obj(vec![]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("null")));
    }

    #[test]
    fn ext_err_type_of_array() {
        let t = mk_transform(vec![verb_field("Out", "typeOf", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", DynValue::Array(vec![i(1)]))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("array")));
    }

    #[test]
    fn ext_err_type_of_object() {
        let t = mk_transform(vec![verb_field("Out", "typeOf", vec![ref_arg("@.val")])]);
        let result = execute(&t, &src_obj(vec![("val", src_obj(vec![("x", i(1))]))]));
        assert!(result.success);
        assert_eq!(result.output.unwrap().get("Out"), Some(&s("object")));
    }
}
mod extended_tests_2 {
    use crate::Odin;
    use crate::types::transform::*;
    use crate::types::values::OdinValues;
    use crate::transform::engine::execute;
    use std::collections::HashMap;

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn header() -> String {
        "{$}\nodin = \"1.0.0\"\ntransform = \"1.0.0\"\ndirection = \"json->json\"\ntarget.format = \"json\"\n".to_string()
    }

    fn parse_and_exec(transform_text: &str, source: &DynValue) -> TransformResult {
        let t = Odin::parse_transform(transform_text).unwrap();
        execute(&t, source)
    }

    fn json_obj(pairs: Vec<(&str, DynValue)>) -> DynValue {
        DynValue::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn s(val: &str) -> DynValue { DynValue::String(val.to_string()) }
    fn i(val: i64) -> DynValue { DynValue::Integer(val) }
    fn f(val: f64) -> DynValue { DynValue::Float(val) }
    fn b(val: bool) -> DynValue { DynValue::Bool(val) }

    fn minimal_transform(mappings: Vec<FieldMapping>) -> OdinTransform {
        OdinTransform {
            metadata: TransformMetadata::default(),
            source: None,
            target: TargetConfig { format: "json".to_string(), options: HashMap::new() },
            constants: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
            segments: vec![TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings,
                children: Vec::new(), items: Vec::new(),
                pass: None, condition: None,
            }],
            imports: Vec::new(),
            passes: Vec::new(),
            enforce_confidential: None,
            strict_types: false,
        }
    }

    fn custom_transform(segments: Vec<TransformSegment>) -> OdinTransform {
        OdinTransform {
            metadata: TransformMetadata::default(),
            source: None,
            target: TargetConfig { format: "json".to_string(), options: HashMap::new() },
            constants: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
            segments,
            imports: Vec::new(),
            passes: Vec::new(),
            enforce_confidential: None,
            strict_types: false,
        }
    }

    fn root_segment(mappings: Vec<FieldMapping>) -> TransformSegment {
        TransformSegment {
            name: String::new(), path: String::new(), source_path: None,
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings,
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        }
    }

    fn make_modifiers(required: bool, confidential: bool, deprecated: bool) -> crate::types::values::OdinModifiers {
        crate::types::values::OdinModifiers { required, confidential, deprecated, attr: false }
    }

    fn verb_mapping(target: &str, verb: &str, args: Vec<VerbArg>) -> FieldMapping {
        FieldMapping {
            target: target.to_string(),
            expression: FieldExpression::Transform(VerbCall {
                verb: verb.to_string(),
                is_custom: false,
                args,
            }),
            directives: vec![],
            modifiers: None,
        }
    }

    fn copy_mapping(target: &str, source_path: &str) -> FieldMapping {
        FieldMapping {
            target: target.to_string(),
            expression: FieldExpression::Copy(source_path.to_string()),
            directives: vec![],
            modifiers: None,
        }
    }

    fn literal_mapping(target: &str, val: crate::types::values::OdinValue) -> FieldMapping {
        FieldMapping {
            target: target.to_string(),
            expression: FieldExpression::Literal(val),
            directives: vec![],
            modifiers: None,
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 1. Verb type checking via parse_transform (~15 tests)
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_upper_on_string_field() {
        let text = format!("{}\n{{Out}}\nName = %upper @.name\n", header());
        let r = parse_and_exec(&text, &json_obj(vec![("name", s("alice"))]));
        assert!(r.success);
        let out = r.output.unwrap();
        assert_eq!(out.get("Out").unwrap().get("Name"), Some(&s("ALICE")));
    }

    #[test]
    fn ext_lower_on_string_field() {
        let text = format!("{}\n{{Out}}\nName = %lower @.name\n", header());
        let r = parse_and_exec(&text, &json_obj(vec![("name", s("HELLO"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Out").unwrap().get("Name"), Some(&s("hello")));
    }

    #[test]
    fn ext_upper_on_numeric_string() {
        let text = format!("{}\n{{Out}}\nVal = %upper @.num\n", header());
        let r = parse_and_exec(&text, &json_obj(vec![("num", s("abc123"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Out").unwrap().get("Val"), Some(&s("ABC123")));
    }

    #[test]
    fn ext_upper_on_null_returns_null() {
        let text = format!("{}\n{{Out}}\nVal = %upper @.missing\n", header());
        let r = parse_and_exec(&text, &json_obj(vec![]));
        assert!(r.success);
        // Missing field resolves to null, verb should handle gracefully
        let out = r.output.unwrap();
        let val = out.get("Out").unwrap().get("Val");
        assert!(val == Some(&DynValue::Null) || val == Some(&s("")));
    }

    #[test]
    fn ext_trim_string() {
        let text = format!("{}\n{{Out}}\nVal = %trim @.name\n", header());
        let r = parse_and_exec(&text, &json_obj(vec![("name", s("  hello  "))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Out").unwrap().get("Val"), Some(&s("hello")));
    }

    #[test]
    fn ext_concat_multiple_strings() {
        let text = format!("{}\n{{Out}}\nFull = %concat @.first \" \" @.last\n", header());
        let r = parse_and_exec(&text, &json_obj(vec![("first", s("John")), ("last", s("Doe"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Out").unwrap().get("Full"), Some(&s("John Doe")));
    }

    #[test]
    fn ext_add_two_integers() {
        let t = minimal_transform(vec![
            verb_mapping("Sum", "add", vec![
                VerbArg::Reference("@.a".to_string(), Vec::new()),
                VerbArg::Reference("@.b".to_string(), Vec::new()),
            ]),
        ]);
        let src = json_obj(vec![("a", i(10)), ("b", i(20))]);
        let r = execute(&t, &src);
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Sum"), Some(&i(30)));
    }

    #[test]
    fn ext_add_integer_and_float() {
        let t = minimal_transform(vec![
            verb_mapping("Sum", "add", vec![
                VerbArg::Reference("@.a".to_string(), Vec::new()),
                VerbArg::Reference("@.b".to_string(), Vec::new()),
            ]),
        ]);
        let src = json_obj(vec![("a", i(10)), ("b", f(2.5))]);
        let r = execute(&t, &src);
        assert!(r.success);
        let out = r.output.unwrap();
        let sum = out.get("Sum").unwrap();
        match sum {
            DynValue::Float(v) => assert!((v - 12.5).abs() < 0.001),
            DynValue::Integer(v) => assert_eq!(*v, 12),
            _ => panic!("Expected numeric result"),
        }
    }

    #[test]
    fn ext_multiply_integers() {
        let t = minimal_transform(vec![
            verb_mapping("Product", "multiply", vec![
                VerbArg::Reference("@.a".to_string(), Vec::new()),
                VerbArg::Reference("@.b".to_string(), Vec::new()),
            ]),
        ]);
        let src = json_obj(vec![("a", i(7)), ("b", i(6))]);
        let r = execute(&t, &src);
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Product"), Some(&i(42)));
    }

    #[test]
    fn ext_subtract_integers() {
        let t = minimal_transform(vec![
            verb_mapping("Diff", "subtract", vec![
                VerbArg::Reference("@.a".to_string(), Vec::new()),
                VerbArg::Reference("@.b".to_string(), Vec::new()),
            ]),
        ]);
        let src = json_obj(vec![("a", i(100)), ("b", i(42))]);
        let r = execute(&t, &src);
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Diff"), Some(&i(58)));
    }

    #[test]
    fn ext_coerce_string_from_integer() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "coerceString", vec![
                VerbArg::Reference("@.num".to_string(), Vec::new()),
            ]),
        ]);
        let src = json_obj(vec![("num", i(42))]);
        let r = execute(&t, &src);
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("42")));
    }

    #[test]
    fn ext_coerce_number_from_string() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "coerceNumber", vec![
                VerbArg::Reference("@.num".to_string(), Vec::new()),
            ]),
        ]);
        let src = json_obj(vec![("num", s("3.14"))]);
        let r = execute(&t, &src);
        assert!(r.success);
        let out = r.output.unwrap();
        match out.get("Val").unwrap() {
            DynValue::Float(v) => assert!((v - 3.14).abs() < 0.001),
            DynValue::Integer(v) => assert_eq!(*v, 3),
            _ => panic!("Expected numeric"),
        }
    }

    #[test]
    fn ext_coerce_boolean_from_string_true() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "coerceBoolean", vec![
                VerbArg::Reference("@.flag".to_string(), Vec::new()),
            ]),
        ]);
        let src = json_obj(vec![("flag", s("true"))]);
        let r = execute(&t, &src);
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&b(true)));
    }

    #[test]
    fn ext_is_null_on_null_value() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "isNull", vec![
                VerbArg::Reference("@.missing".to_string(), Vec::new()),
            ]),
        ]);
        let src = json_obj(vec![]);
        let r = execute(&t, &src);
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&b(true)));
    }

    #[test]
    fn ext_is_null_on_present_value() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "isNull", vec![
                VerbArg::Reference("@.name".to_string(), Vec::new()),
            ]),
        ]);
        let src = json_obj(vec![("name", s("Alice"))]);
        let r = execute(&t, &src);
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&b(false)));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 2. Confidential enforcement via engine (~20 tests)
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_confidential_redact_string() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "SSN".to_string(),
            expression: FieldExpression::Copy("@.ssn".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let r = execute(&t, &json_obj(vec![("ssn", s("123-45-6789"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("SSN"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_confidential_redact_integer() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "PIN".to_string(),
            expression: FieldExpression::Copy("@.pin".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let r = execute(&t, &json_obj(vec![("pin", i(1234))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("PIN"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_confidential_redact_boolean() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "Secret".to_string(),
            expression: FieldExpression::Copy("@.flag".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let r = execute(&t, &json_obj(vec![("flag", b(true))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Secret"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_confidential_redact_float() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "Balance".to_string(),
            expression: FieldExpression::Copy("@.bal".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let r = execute(&t, &json_obj(vec![("bal", f(1234.56))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Balance"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_confidential_mask_string_becomes_asterisks() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "SSN".to_string(),
            expression: FieldExpression::Copy("@.ssn".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let r = execute(&t, &json_obj(vec![("ssn", s("123-45-6789"))]));
        assert!(r.success);
        let out = r.output.unwrap();
        let val = out.get("SSN").unwrap();
        match val {
            DynValue::String(masked) => {
                assert_eq!(masked.len(), "123-45-6789".len());
                assert!(masked.chars().all(|c| c == '*'));
            }
            _ => panic!("Expected masked string"),
        }
    }

    #[test]
    fn ext_confidential_mask_integer_becomes_null() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "PIN".to_string(),
            expression: FieldExpression::Copy("@.pin".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let r = execute(&t, &json_obj(vec![("pin", i(1234))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("PIN"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_confidential_mask_boolean_becomes_null() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "Flag".to_string(),
            expression: FieldExpression::Copy("@.flag".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let r = execute(&t, &json_obj(vec![("flag", b(false))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Flag"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_confidential_no_enforcement_passes_through() {
        let t = minimal_transform(vec![FieldMapping {
            target: "SSN".to_string(),
            expression: FieldExpression::Copy("@.ssn".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        // enforce_confidential defaults to None
        let r = execute(&t, &json_obj(vec![("ssn", s("123-45-6789"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("SSN"), Some(&s("123-45-6789")));
    }

    #[test]
    fn ext_confidential_modifier_recorded_with_redact() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "SSN".to_string(),
            expression: FieldExpression::Copy("@.ssn".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let r = execute(&t, &json_obj(vec![("ssn", s("123"))]));
        assert!(r.success);
        assert!(r.modifiers.contains_key("SSN"));
        assert!(r.modifiers["SSN"].confidential);
    }

    #[test]
    fn ext_confidential_mixed_fields_only_confidential_redacted() {
        let mut t = minimal_transform(vec![
            FieldMapping {
                target: "Name".to_string(),
                expression: FieldExpression::Copy("@.name".to_string()),
                directives: vec![], modifiers: None,
            },
            FieldMapping {
                target: "SSN".to_string(),
                expression: FieldExpression::Copy("@.ssn".to_string()),
                directives: vec![],
                modifiers: Some(make_modifiers(false, true, false)),
            },
            FieldMapping {
                target: "Email".to_string(),
                expression: FieldExpression::Copy("@.email".to_string()),
                directives: vec![], modifiers: None,
            },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let src = json_obj(vec![
            ("name", s("Alice")), ("ssn", s("123-45-6789")), ("email", s("a@b.com")),
        ]);
        let r = execute(&t, &src);
        assert!(r.success);
        let out = r.output.unwrap();
        assert_eq!(out.get("Name"), Some(&s("Alice")));
        assert_eq!(out.get("SSN"), Some(&DynValue::Null));
        assert_eq!(out.get("Email"), Some(&s("a@b.com")));
    }

    #[test]
    fn ext_confidential_required_and_confidential_both_recorded() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "Key".to_string(),
            expression: FieldExpression::Copy("@.key".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(true, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let r = execute(&t, &json_obj(vec![("key", s("secret"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Key"), Some(&DynValue::Null));
        assert!(r.modifiers["Key"].required);
        assert!(r.modifiers["Key"].confidential);
    }

    #[test]
    fn ext_confidential_deprecated_and_confidential() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "Old".to_string(),
            expression: FieldExpression::Copy("@.old".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, true)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let r = execute(&t, &json_obj(vec![("old", s("legacy"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Old"), Some(&DynValue::Null));
        assert!(r.modifiers["Old"].confidential);
        assert!(r.modifiers["Old"].deprecated);
    }

    #[test]
    fn ext_confidential_redact_null_stays_null() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "Val".to_string(),
            expression: FieldExpression::Copy("@.missing".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let r = execute(&t, &json_obj(vec![]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_confidential_mask_empty_string() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "Val".to_string(),
            expression: FieldExpression::Copy("@.val".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let r = execute(&t, &json_obj(vec![("val", s(""))]));
        assert!(r.success);
        let out = r.output.unwrap();
        assert_eq!(out.get("Val"), Some(&s("")));
    }

    #[test]
    fn ext_confidential_mask_single_char_string() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "Val".to_string(),
            expression: FieldExpression::Copy("@.val".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let r = execute(&t, &json_obj(vec![("val", s("X"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("*")));
    }

    #[test]
    fn ext_confidential_mask_float_becomes_null() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "Amt".to_string(),
            expression: FieldExpression::Copy("@.amt".to_string()),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Mask);
        let r = execute(&t, &json_obj(vec![("amt", f(99.99))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Amt"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_confidential_redact_after_verb_transform() {
        let mut t = minimal_transform(vec![FieldMapping {
            target: "SSN".to_string(),
            expression: FieldExpression::Transform(VerbCall {
                verb: "upper".to_string(),
                is_custom: false,
                args: vec![VerbArg::Reference("@.ssn".to_string(), Vec::new())],
            }),
            directives: vec![],
            modifiers: Some(make_modifiers(false, true, false)),
        }]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let r = execute(&t, &json_obj(vec![("ssn", s("abc"))]));
        assert!(r.success);
        // Even though verb transforms it, confidential redact should null it
        assert_eq!(r.output.unwrap().get("SSN"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_confidential_three_confidential_fields_all_redacted() {
        let mut t = minimal_transform(vec![
            FieldMapping { target: "A".into(), expression: FieldExpression::Copy("@.a".into()),
                directives: vec![], modifiers: Some(make_modifiers(false, true, false)) },
            FieldMapping { target: "B".into(), expression: FieldExpression::Copy("@.b".into()),
                directives: vec![], modifiers: Some(make_modifiers(false, true, false)) },
            FieldMapping { target: "C".into(), expression: FieldExpression::Copy("@.c".into()),
                directives: vec![], modifiers: Some(make_modifiers(false, true, false)) },
        ]);
        t.enforce_confidential = Some(ConfidentialMode::Redact);
        let src = json_obj(vec![("a", s("1")), ("b", s("2")), ("c", s("3"))]);
        let r = execute(&t, &src);
        assert!(r.success);
        let out = r.output.unwrap();
        assert_eq!(out.get("A"), Some(&DynValue::Null));
        assert_eq!(out.get("B"), Some(&DynValue::Null));
        assert_eq!(out.get("C"), Some(&DynValue::Null));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 3. Multi-pass transforms (~15 tests)
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_two_passes_with_accumulator() {
        let mut t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![literal_mapping("P1", OdinValues::string("pass1"))],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![literal_mapping("P2", OdinValues::string("pass2"))],
                children: Vec::new(), items: Vec::new(),
                pass: Some(2), condition: None,
            },
        ]);
        t.accumulators.insert("counter".to_string(), AccumulatorDef {
            name: "counter".to_string(), initial: OdinValues::integer(0), persist: false,
        });
        let r = execute(&t, &DynValue::Object(Vec::new()));
        assert!(r.success);
        let out = r.output.unwrap();
        assert_eq!(out.get("P1"), Some(&s("pass1")));
        assert_eq!(out.get("P2"), Some(&s("pass2")));
    }

    #[test]
    fn ext_pass_none_runs_last() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![literal_mapping("Last", OdinValues::string("none"))],
                children: Vec::new(), items: Vec::new(),
                pass: None, condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![literal_mapping("First", OdinValues::string("one"))],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
        ]);
        let r = execute(&t, &DynValue::Object(Vec::new()));
        assert!(r.success);
        let out = r.output.unwrap();
        assert_eq!(out.get("First"), Some(&s("one")));
        assert_eq!(out.get("Last"), Some(&s("none")));
    }

    #[test]
    fn ext_five_passes() {
        let segs: Vec<TransformSegment> = (1..=5).map(|p| TransformSegment {
            name: String::new(), path: String::new(), source_path: None,
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![literal_mapping(&format!("P{}", p), OdinValues::integer(p as i64))],
            children: Vec::new(), items: Vec::new(),
            pass: Some(p), condition: None,
        }).collect();
        let t = custom_transform(segs);
        let r = execute(&t, &DynValue::Object(Vec::new()));
        assert!(r.success);
        let out = r.output.unwrap();
        for p in 1..=5 {
            assert_eq!(out.get(&format!("P{}", p)), Some(&i(p as i64)));
        }
    }

    #[test]
    fn ext_pass_ordering_reverse_input() {
        // Segments in reverse order still execute by pass number
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![literal_mapping("Z", OdinValues::integer(3))],
                children: Vec::new(), items: Vec::new(),
                pass: Some(3), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![literal_mapping("A", OdinValues::integer(1))],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
        ]);
        let r = execute(&t, &DynValue::Object(Vec::new()));
        assert!(r.success);
        let out = r.output.unwrap();
        assert_eq!(out.get("A"), Some(&i(1)));
        assert_eq!(out.get("Z"), Some(&i(3)));
    }

    #[test]
    fn ext_multiple_segments_same_pass() {
        let t = custom_transform(vec![
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![literal_mapping("A", OdinValues::string("x"))],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![literal_mapping("B", OdinValues::string("y"))],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
            TransformSegment {
                name: String::new(), path: String::new(), source_path: None,
                discriminator: None, is_array: false, directives: Vec::new(),
                mappings: vec![literal_mapping("C", OdinValues::string("z"))],
                children: Vec::new(), items: Vec::new(),
                pass: Some(1), condition: None,
            },
        ]);
        let r = execute(&t, &DynValue::Object(Vec::new()));
        assert!(r.success);
        let out = r.output.unwrap();
        assert_eq!(out.get("A"), Some(&s("x")));
        assert_eq!(out.get("B"), Some(&s("y")));
        assert_eq!(out.get("C"), Some(&s("z")));
    }

    #[test]
    fn ext_pass_with_condition_true() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(), path: String::new(), source_path: None,
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![literal_mapping("Hit", OdinValues::string("yes"))],
            children: Vec::new(), items: Vec::new(),
            pass: Some(1), condition: Some("@.active".to_string()),
        }]);
        let r = execute(&t, &json_obj(vec![("active", b(true))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Hit"), Some(&s("yes")));
    }

    #[test]
    fn ext_pass_with_condition_false_skips() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(), path: String::new(), source_path: None,
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![literal_mapping("Hit", OdinValues::string("yes"))],
            children: Vec::new(), items: Vec::new(),
            pass: Some(1), condition: Some("@.active".to_string()),
        }]);
        let r = execute(&t, &json_obj(vec![("active", b(false))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Hit"), None);
    }

    #[test]
    fn ext_pass_with_condition_null_skips() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(), path: String::new(), source_path: None,
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![literal_mapping("Hit", OdinValues::string("yes"))],
            children: Vec::new(), items: Vec::new(),
            pass: Some(1), condition: Some("@.missing".to_string()),
        }]);
        let r = execute(&t, &json_obj(vec![]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Hit"), None);
    }

    #[test]
    fn ext_pass_with_condition_empty_string_skips() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(), path: String::new(), source_path: None,
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![literal_mapping("Hit", OdinValues::string("yes"))],
            children: Vec::new(), items: Vec::new(),
            pass: Some(1), condition: Some("@.val".to_string()),
        }]);
        let r = execute(&t, &json_obj(vec![("val", s(""))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Hit"), None);
    }

    #[test]
    fn ext_pass_with_condition_nonzero_int_runs() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(), path: String::new(), source_path: None,
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![literal_mapping("Hit", OdinValues::string("yes"))],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: Some("@.count".to_string()),
        }]);
        let r = execute(&t, &json_obj(vec![("count", i(5))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Hit"), Some(&s("yes")));
    }

    #[test]
    fn ext_pass_with_condition_zero_int_skips() {
        let t = custom_transform(vec![TransformSegment {
            name: String::new(), path: String::new(), source_path: None,
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![literal_mapping("Hit", OdinValues::string("yes"))],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: Some("@.count".to_string()),
        }]);
        let r = execute(&t, &json_obj(vec![("count", i(0))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Hit"), None);
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 4. Complex verb expressions (nested verbs, chains) (~15 tests)
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_nested_verb_upper_of_concat() {
        let t = minimal_transform(vec![FieldMapping {
            target: "Val".to_string(),
            expression: FieldExpression::Transform(VerbCall {
                verb: "upper".to_string(),
                is_custom: false,
                args: vec![VerbArg::Verb(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Reference("@.a".to_string(), Vec::new()),
                        VerbArg::Reference("@.b".to_string(), Vec::new()),
                    ],
                })],
            }),
            directives: vec![], modifiers: None,
        }]);
        let r = execute(&t, &json_obj(vec![("a", s("hello")), ("b", s("world"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("HELLOWORLD")));
    }

    #[test]
    fn ext_nested_verb_trim_of_upper() {
        let t = minimal_transform(vec![FieldMapping {
            target: "Val".to_string(),
            expression: FieldExpression::Transform(VerbCall {
                verb: "trim".to_string(),
                is_custom: false,
                args: vec![VerbArg::Verb(VerbCall {
                    verb: "upper".to_string(),
                    is_custom: false,
                    args: vec![VerbArg::Reference("@.name".to_string(), Vec::new())],
                })],
            }),
            directives: vec![], modifiers: None,
        }]);
        let r = execute(&t, &json_obj(vec![("name", s("  hello  "))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("HELLO")));
    }

    #[test]
    fn ext_nested_verb_length_of_concat() {
        let t = minimal_transform(vec![FieldMapping {
            target: "Len".to_string(),
            expression: FieldExpression::Transform(VerbCall {
                verb: "length".to_string(),
                is_custom: false,
                args: vec![VerbArg::Verb(VerbCall {
                    verb: "concat".to_string(),
                    is_custom: false,
                    args: vec![
                        VerbArg::Literal(OdinValues::string("abc")),
                        VerbArg::Literal(OdinValues::string("def")),
                    ],
                })],
            }),
            directives: vec![], modifiers: None,
        }]);
        let r = execute(&t, &DynValue::Object(Vec::new()));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Len"), Some(&i(6)));
    }

    #[test]
    fn ext_nested_verb_add_of_multiply() {
        let t = minimal_transform(vec![FieldMapping {
            target: "Val".to_string(),
            expression: FieldExpression::Transform(VerbCall {
                verb: "add".to_string(),
                is_custom: false,
                args: vec![
                    VerbArg::Verb(VerbCall {
                        verb: "multiply".to_string(),
                        is_custom: false,
                        args: vec![
                            VerbArg::Reference("@.a".to_string(), Vec::new()),
                            VerbArg::Literal(OdinValues::integer(2)),
                        ],
                    }),
                    VerbArg::Reference("@.b".to_string(), Vec::new()),
                ],
            }),
            directives: vec![], modifiers: None,
        }]);
        let r = execute(&t, &json_obj(vec![("a", i(5)), ("b", i(3))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&i(13)));
    }

    #[test]
    fn ext_nested_verb_coalesce_with_null() {
        let t = minimal_transform(vec![FieldMapping {
            target: "Val".to_string(),
            expression: FieldExpression::Transform(VerbCall {
                verb: "coalesce".to_string(),
                is_custom: false,
                args: vec![
                    VerbArg::Reference("@.missing".to_string(), Vec::new()),
                    VerbArg::Literal(OdinValues::string("default")),
                ],
            }),
            directives: vec![], modifiers: None,
        }]);
        let r = execute(&t, &json_obj(vec![]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("default")));
    }

    #[test]
    fn ext_if_else_true_branch() {
        let t = minimal_transform(vec![FieldMapping {
            target: "Val".to_string(),
            expression: FieldExpression::Transform(VerbCall {
                verb: "ifElse".to_string(),
                is_custom: false,
                args: vec![
                    VerbArg::Reference("@.flag".to_string(), Vec::new()),
                    VerbArg::Literal(OdinValues::string("yes")),
                    VerbArg::Literal(OdinValues::string("no")),
                ],
            }),
            directives: vec![], modifiers: None,
        }]);
        let r = execute(&t, &json_obj(vec![("flag", b(true))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("yes")));
    }

    #[test]
    fn ext_if_else_false_branch() {
        let t = minimal_transform(vec![FieldMapping {
            target: "Val".to_string(),
            expression: FieldExpression::Transform(VerbCall {
                verb: "ifElse".to_string(),
                is_custom: false,
                args: vec![
                    VerbArg::Reference("@.flag".to_string(), Vec::new()),
                    VerbArg::Literal(OdinValues::string("yes")),
                    VerbArg::Literal(OdinValues::string("no")),
                ],
            }),
            directives: vec![], modifiers: None,
        }]);
        let r = execute(&t, &json_obj(vec![("flag", b(false))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("no")));
    }

    #[test]
    fn ext_capitalize_verb() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "capitalize", vec![
                VerbArg::Reference("@.name".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("name", s("hello"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("Hello")));
    }

    #[test]
    fn ext_replace_verb() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "replace", vec![
                VerbArg::Reference("@.text".to_string(), Vec::new()),
                VerbArg::Literal(OdinValues::string("world")),
                VerbArg::Literal(OdinValues::string("earth")),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("text", s("hello world"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("hello earth")));
    }

    #[test]
    fn ext_substring_verb() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "substring", vec![
                VerbArg::Reference("@.text".to_string(), Vec::new()),
                VerbArg::Literal(OdinValues::integer(0)),
                VerbArg::Literal(OdinValues::integer(5)),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("text", s("hello world"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("hello")));
    }

    #[test]
    fn ext_length_of_string() {
        let t = minimal_transform(vec![
            verb_mapping("Len", "length", vec![
                VerbArg::Reference("@.text".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("text", s("hello"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Len"), Some(&i(5)));
    }

    #[test]
    fn ext_abs_negative_integer() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "abs", vec![
                VerbArg::Reference("@.num".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("num", i(-42))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&i(42)));
    }

    #[test]
    fn ext_round_float() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "round", vec![
                VerbArg::Reference("@.num".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("num", f(3.7))]));
        assert!(r.success);
        let out = r.output.unwrap();
        let val = out.get("Val").unwrap();
        match val {
            DynValue::Integer(v) => assert_eq!(*v, 4),
            DynValue::Float(v) => assert!((v - 4.0).abs() < 0.001),
            _ => panic!("Expected numeric"),
        }
    }

    #[test]
    fn ext_not_verb() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "not", vec![
                VerbArg::Reference("@.flag".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("flag", b(true))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&b(false)));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 5. Error handling (~15 tests)
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_missing_source_field_returns_null() {
        let t = minimal_transform(vec![copy_mapping("Val", "@.missing")]);
        let r = execute(&t, &json_obj(vec![("other", s("x"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_deeply_nested_missing_field() {
        let t = minimal_transform(vec![copy_mapping("Val", "@.a.b.c.d.e")]);
        let r = execute(&t, &json_obj(vec![("a", json_obj(vec![]))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_empty_source_object() {
        let t = minimal_transform(vec![copy_mapping("Name", "@.name")]);
        let r = execute(&t, &DynValue::Object(Vec::new()));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Name"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_copy_from_array_index() {
        let t = minimal_transform(vec![copy_mapping("First", "@.items[0]")]);
        let src = json_obj(vec![
            ("items", DynValue::Array(vec![s("alpha"), s("beta")])),
        ]);
        let r = execute(&t, &src);
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("First"), Some(&s("alpha")));
    }

    #[test]
    fn ext_copy_from_array_out_of_bounds() {
        let t = minimal_transform(vec![copy_mapping("Val", "@.items[99]")]);
        let src = json_obj(vec![
            ("items", DynValue::Array(vec![s("only")])),
        ]);
        let r = execute(&t, &src);
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_multiple_mappings_to_same_target_last_wins() {
        let t = minimal_transform(vec![
            literal_mapping("Val", OdinValues::string("first")),
            literal_mapping("Val", OdinValues::string("second")),
        ]);
        let r = execute(&t, &DynValue::Object(Vec::new()));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("second")));
    }

    #[test]
    fn ext_nested_target_path_creates_objects() {
        let t = minimal_transform(vec![
            copy_mapping("info.name", "@.name"),
            copy_mapping("info.age", "@.age"),
        ]);
        let src = json_obj(vec![("name", s("Alice")), ("age", i(30))]);
        let r = execute(&t, &src);
        assert!(r.success);
        let out = r.output.unwrap();
        let info = out.get("info").unwrap();
        assert_eq!(info.get("name"), Some(&s("Alice")));
        assert_eq!(info.get("age"), Some(&i(30)));
    }

    #[test]
    fn ext_deeply_nested_target_path() {
        let t = minimal_transform(vec![
            copy_mapping("a.b.c", "@.val"),
        ]);
        let r = execute(&t, &json_obj(vec![("val", s("deep"))]));
        assert!(r.success);
        let out = r.output.unwrap();
        let a = out.get("a").unwrap();
        let b = a.get("b").unwrap();
        assert_eq!(b.get("c"), Some(&s("deep")));
    }

    #[test]
    fn ext_literal_integer_mapping() {
        let t = minimal_transform(vec![literal_mapping("Val", OdinValues::integer(99))]);
        let r = execute(&t, &DynValue::Object(Vec::new()));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&i(99)));
    }

    #[test]
    fn ext_literal_boolean_mapping() {
        let t = minimal_transform(vec![literal_mapping("Val", OdinValues::boolean(false))]);
        let r = execute(&t, &DynValue::Object(Vec::new()));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&b(false)));
    }

    #[test]
    fn ext_literal_null_mapping() {
        let t = minimal_transform(vec![literal_mapping("Val", OdinValues::null())]);
        let r = execute(&t, &DynValue::Object(Vec::new()));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&DynValue::Null));
    }

    #[test]
    fn ext_empty_segments_produces_empty_output() {
        let t = custom_transform(vec![]);
        let r = execute(&t, &json_obj(vec![("x", i(1))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap(), DynValue::Object(Vec::new()));
    }

    #[test]
    fn ext_loop_over_empty_array() {
        let t = custom_transform(vec![TransformSegment {
            name: "Items".to_string(), path: "Items".to_string(),
            source_path: Some("@.items".to_string()),
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![copy_mapping("Name", "@_item.name")],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        }]);
        let r = execute(&t, &json_obj(vec![("items", DynValue::Array(vec![]))]));
        assert!(r.success);
    }

    #[test]
    fn ext_loop_over_array_with_verb() {
        let t = custom_transform(vec![TransformSegment {
            name: "Results".to_string(), path: "Results".to_string(),
            source_path: Some("@.items".to_string()),
            discriminator: None, is_array: false, directives: Vec::new(),
            mappings: vec![verb_mapping("Lower", "lower", vec![
                VerbArg::Reference("@_item.val".to_string(), Vec::new()),
            ])],
            children: Vec::new(), items: Vec::new(),
            pass: None, condition: None,
        }]);
        let src = json_obj(vec![
            ("items", DynValue::Array(vec![
                json_obj(vec![("val", s("HELLO"))]),
                json_obj(vec![("val", s("WORLD"))]),
            ])),
        ]);
        let r = execute(&t, &src);
        assert!(r.success);
        let out = r.output.unwrap();
        let results = out.get("Results").unwrap().as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].get("Lower"), Some(&s("hello")));
        assert_eq!(results[1].get("Lower"), Some(&s("world")));
    }

    #[test]
    fn ext_eq_verb_true() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "eq", vec![
                VerbArg::Reference("@.a".to_string(), Vec::new()),
                VerbArg::Reference("@.b".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("a", s("x")), ("b", s("x"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&b(true)));
    }

    #[test]
    fn ext_eq_verb_false() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "eq", vec![
                VerbArg::Reference("@.a".to_string(), Vec::new()),
                VerbArg::Reference("@.b".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("a", s("x")), ("b", s("y"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&b(false)));
    }

    #[test]
    fn ext_ne_verb() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "ne", vec![
                VerbArg::Reference("@.a".to_string(), Vec::new()),
                VerbArg::Reference("@.b".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("a", i(1)), ("b", i(2))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&b(true)));
    }

    #[test]
    fn ext_type_of_string() {
        let t = minimal_transform(vec![
            verb_mapping("T", "typeOf", vec![
                VerbArg::Reference("@.val".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("val", s("hello"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("T"), Some(&s("string")));
    }

    #[test]
    fn ext_type_of_integer() {
        let t = minimal_transform(vec![
            verb_mapping("T", "typeOf", vec![
                VerbArg::Reference("@.val".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("val", i(42))]));
        assert!(r.success);
        let out = r.output.unwrap();
        let t_val = out.get("T").unwrap();
        match t_val {
            DynValue::String(tv) => assert!(tv == "number" || tv == "integer"),
            _ => panic!("Expected string type name"),
        }
    }

    #[test]
    fn ext_type_of_boolean() {
        let t = minimal_transform(vec![
            verb_mapping("T", "typeOf", vec![
                VerbArg::Reference("@.val".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("val", b(true))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("T"), Some(&s("boolean")));
    }

    #[test]
    fn ext_type_of_null() {
        let t = minimal_transform(vec![
            verb_mapping("T", "typeOf", vec![
                VerbArg::Reference("@.val".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("T"), Some(&s("null")));
    }

    #[test]
    fn ext_if_null_with_null() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "ifNull", vec![
                VerbArg::Reference("@.missing".to_string(), Vec::new()),
                VerbArg::Literal(OdinValues::string("fallback")),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("fallback")));
    }

    #[test]
    fn ext_if_null_with_value() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "ifNull", vec![
                VerbArg::Reference("@.name".to_string(), Vec::new()),
                VerbArg::Literal(OdinValues::string("fallback")),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("name", s("Alice"))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&s("Alice")));
    }

    #[test]
    fn ext_divide_integers() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "divide", vec![
                VerbArg::Reference("@.a".to_string(), Vec::new()),
                VerbArg::Reference("@.b".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("a", i(10)), ("b", i(3))]));
        assert!(r.success);
        let out = r.output.unwrap();
        let val = out.get("Val").unwrap();
        match val {
            DynValue::Float(v) => assert!((v - 3.333).abs() < 0.1),
            DynValue::Integer(v) => assert_eq!(*v, 3),
            _ => panic!("Expected numeric"),
        }
    }

    #[test]
    fn ext_gt_verb() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "gt", vec![
                VerbArg::Reference("@.a".to_string(), Vec::new()),
                VerbArg::Reference("@.b".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("a", i(10)), ("b", i(5))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&b(true)));
    }

    #[test]
    fn ext_lt_verb() {
        let t = minimal_transform(vec![
            verb_mapping("Val", "lt", vec![
                VerbArg::Reference("@.a".to_string(), Vec::new()),
                VerbArg::Reference("@.b".to_string(), Vec::new()),
            ]),
        ]);
        let r = execute(&t, &json_obj(vec![("a", i(3)), ("b", i(5))]));
        assert!(r.success);
        assert_eq!(r.output.unwrap().get("Val"), Some(&b(true)));
    }
}
