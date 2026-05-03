//! Transform document parser.
//!
//! Converts a parsed `OdinDocument` into an `OdinTransform` structure.
//! The input document has already been parsed from ODIN text by the main parser;
//! this module interprets the document's metadata and assignments as transform
//! configuration, segments, field mappings, lookup tables, and so on.

use std::collections::HashMap;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::types::document::OdinDocument;
use crate::types::ordered_map::OrderedMap;
use crate::types::transform::{OdinTransform, TransformMetadata, SourceConfig, SourceDiscriminator, TargetConfig, AccumulatorDef, LookupTable, ImportRef, ConfidentialMode, TransformSegment, SegmentDirective, Discriminator, FieldMapping, FieldExpression, VerbCall, VerbArg};
use crate::types::transform::DynValue;
use crate::types::values::{OdinArrayItem, OdinModifiers, OdinValue, OdinValues};

/// Parse an already-parsed `OdinDocument` into an `OdinTransform`.
///
/// The document is expected to follow the ODIN transform specification:
/// - `{$}` metadata keys define version, direction, source/target config, etc.
/// - Non-`$` sections define transform segments with field mappings.
pub fn parse_transform_doc(doc: OdinDocument) -> OdinTransform {
    let metadata = parse_metadata(&doc);
    let source = parse_source_config(&doc);
    let target = parse_target_config(&doc);
    let constants = parse_constants(&doc);
    let accumulators = parse_accumulators(&doc);
    let tables = parse_lookup_tables(&doc);
    let imports = parse_imports(&doc);
    let enforce_confidential = parse_enforce_confidential(&doc);
    let strict_types = parse_strict_types(&doc);

    let OdinDocument { assignments, modifiers, .. } = doc;
    let segments = parse_segments(assignments, modifiers);
    let passes = collect_passes(&segments);

    OdinTransform {
        metadata,
        source,
        target,
        constants,
        accumulators,
        tables,
        segments,
        imports,
        passes,
        enforce_confidential,
        strict_types,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Metadata
// ─────────────────────────────────────────────────────────────────────────────

/// Parse transform metadata from the `{$}` section.
///
/// Standard metadata keys: `odin`, `transform`, `direction`, `name`, `description`.
fn parse_metadata(doc: &OdinDocument) -> TransformMetadata {
    TransformMetadata {
        odin_version: get_meta_string(doc, "odin"),
        transform_version: get_meta_string(doc, "transform"),
        direction: get_meta_string(doc, "direction"),
        name: get_meta_string(doc, "name"),
        description: get_meta_string(doc, "description"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Source / Target Config
// ─────────────────────────────────────────────────────────────────────────────

/// Parse source format configuration from `$.source.*` metadata keys.
fn parse_source_config(doc: &OdinDocument) -> Option<SourceConfig> {
    // Try `source.format` in metadata first (keys without `$.` prefix).
    let format = get_meta_string(doc, "source.format")?;
    let mut options = HashMap::new();
    let mut namespaces = HashMap::new();

    for (key, value) in &doc.metadata {
        if let Some(rest) = key.strip_prefix("source.namespace.") {
            namespaces.insert(rest.to_string(), odin_value_to_string(value));
        } else if let Some(rest) = key.strip_prefix("source.") {
            if rest != "format" {
                options.insert(rest.to_string(), odin_value_to_string(value));
            }
        }
    }

    // Parse discriminator configuration
    let discriminator = parse_source_discriminator(doc);

    Some(SourceConfig { format, options, namespaces, discriminator })
}

/// Parse source discriminator configuration from metadata.
fn parse_source_discriminator(doc: &OdinDocument) -> Option<SourceDiscriminator> {
    use crate::types::transform::{SourceDiscriminator, DiscriminatorType};

    let disc_type_str = get_meta_string(doc, "source.discriminator.type")?;
    let disc_type = match disc_type_str.as_str() {
        "position" => DiscriminatorType::Position,
        "field" => DiscriminatorType::Field,
        "path" => DiscriminatorType::Path,
        _ => return None,
    };

    let pos = get_meta_string(doc, "source.discriminator.pos")
        .and_then(|s| s.parse().ok());
    let len = get_meta_string(doc, "source.discriminator.len")
        .and_then(|s| s.parse().ok());
    let field = get_meta_string(doc, "source.discriminator.field")
        .and_then(|s| s.parse().ok());
    let path = get_meta_string(doc, "source.discriminator.path");

    Some(SourceDiscriminator { disc_type, pos, len, field, path })
}

/// Parse target format configuration from `$.target.*` metadata keys.
fn parse_target_config(doc: &OdinDocument) -> TargetConfig {
    let format = get_meta_string(doc, "target.format").unwrap_or_default();
    let mut options = HashMap::new();

    for (key, value) in &doc.metadata {
        if let Some(rest) = key.strip_prefix("target.") {
            if rest != "format" {
                options.insert(rest.to_string(), odin_value_to_string(value));
            }
        }
    }

    TargetConfig { format, options }
}

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Parse named constants from `$.const.*` metadata keys.
///
/// The `$.const.` prefix is stripped to produce the constant name.
/// Indexed keys like `numbers[0]`, `numbers[1]` are consolidated into array values.
fn parse_constants(doc: &OdinDocument) -> HashMap<String, OdinValue> {
    let mut constants = HashMap::new();
    // Track array-indexed constants: base_name -> Vec<(index, value)>
    let mut array_entries: HashMap<String, Vec<(usize, OdinValue)>> = HashMap::new();

    for (key, value) in &doc.metadata {
        if let Some(name) = key.strip_prefix("const.") {
            // Check for array index syntax: name[N]
            if let Some(bracket_pos) = name.find('[') {
                if name.ends_with(']') {
                    let base = &name[..bracket_pos];
                    let idx_str = &name[bracket_pos + 1..name.len() - 1];
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        array_entries.entry(base.to_string())
                            .or_default()
                            .push((idx, value.clone()));
                        continue;
                    }
                }
            }
            constants.insert(name.to_string(), value.clone());
        }
    }

    // Build arrays from indexed entries
    for (base_name, mut entries) in array_entries {
        entries.sort_by_key(|(idx, _)| *idx);
        let max_idx = entries.last().map_or(0, |(idx, _)| *idx);
        let mut arr: Vec<OdinArrayItem> = (0..=max_idx)
            .map(|_| OdinArrayItem::Value(OdinValues::null()))
            .collect();
        for (idx, val) in entries {
            arr[idx] = OdinArrayItem::Value(val);
        }
        constants.insert(base_name, OdinValues::array(arr));
    }

    constants
}

// ─────────────────────────────────────────────────────────────────────────────
// Accumulators
// ─────────────────────────────────────────────────────────────────────────────

/// Parse accumulator definitions from `$.accumulator.*` metadata keys.
///
/// Keys ending with `._persist` are skipped (they are runtime flags, not definitions).
fn parse_accumulators(doc: &OdinDocument) -> HashMap<String, AccumulatorDef> {
    let mut accumulators = HashMap::new();

    // First pass: create accumulator definitions
    for (key, value) in &doc.metadata {
        if let Some(name) = key.strip_prefix("accumulator.") {
            // Skip persistence flags like `accumulator.total._persist`
            if name.ends_with("._persist") {
                continue;
            }
            accumulators.insert(
                name.to_string(),
                AccumulatorDef {
                    name: name.to_string(),
                    initial: value.clone(),
                    persist: false,
                },
            );
        }
    }

    // Second pass: set persist flags
    for (key, value) in &doc.metadata {
        if let Some(name) = key.strip_prefix("accumulator.") {
            if let Some(acc_name) = name.strip_suffix("._persist") {
                if let Some(def) = accumulators.get_mut(acc_name) {
                    def.persist = matches!(value, OdinValue::Boolean { value: true, .. });
                }
            }
        }
    }

    accumulators
}

// ─────────────────────────────────────────────────────────────────────────────
// Lookup Tables
// ─────────────────────────────────────────────────────────────────────────────

/// Convert an `OdinValue` to a `DynValue` for table storage.
fn odin_value_to_dyn_for_table(val: &OdinValue) -> DynValue {
    match val {
        OdinValue::Null { .. } => DynValue::Null,
        OdinValue::Boolean { value, .. } => DynValue::Bool(*value),
        OdinValue::String { value, .. }
        | OdinValue::Time { value, .. }
        | OdinValue::Duration { value, .. } => DynValue::String(value.clone()),
        OdinValue::Integer { value, .. } => DynValue::Integer(*value),
        OdinValue::Number { value, .. }
        | OdinValue::Currency { value, .. }
        | OdinValue::Percent { value, .. } => DynValue::Float(*value),
        OdinValue::Date { raw, .. }
        | OdinValue::Timestamp { raw, .. } => DynValue::String(raw.clone()),
        OdinValue::Reference { path, .. } => DynValue::String(path.clone()),
        _ => DynValue::String(odin_value_to_string(val)),
    }
}

/// Parse lookup tables from `$.table.*` metadata keys.
///
/// Format: `table.NAME[row].column = value`
///
/// Tables store all columns and full row data for multi-key lookups.
/// The lookup verb uses `TABLE_NAME.result_column` syntax and matches
/// key columns against provided arguments.
fn parse_lookup_tables(doc: &OdinDocument) -> HashMap<String, LookupTable> {
    let mut tables: HashMap<String, LookupTable> = HashMap::new();

    // Intermediate: collect rows per table.
    // table_name -> vec of (row_index, column_name, DynValue)
    let mut table_rows: HashMap<String, Vec<(usize, String, DynValue)>> = HashMap::new();
    let mut table_defaults: HashMap<String, DynValue> = HashMap::new();

    for (key, value) in &doc.metadata {
        let Some(rest) = key.strip_prefix("table.") else { continue };

        // Check for default: `table.NAME._default`
        if let Some(name_and_default) = rest.strip_suffix("._default") {
            if !name_and_default.is_empty() && !name_and_default.contains('[') {
                table_defaults.insert(
                    name_and_default.to_string(),
                    odin_value_to_dyn_for_table(value),
                );
                continue;
            }
        }

        // Parse `NAME[row].column`
        if let Some(bracket_pos) = rest.find('[') {
            let table_name = &rest[..bracket_pos];
            if let Some(close_pos) = rest[bracket_pos..].find(']') {
                let idx_str = &rest[bracket_pos + 1..bracket_pos + close_pos];
                let after_bracket = &rest[bracket_pos + close_pos + 1..];

                if let Ok(row_idx) = idx_str.parse::<usize>() {
                    let col_name = after_bracket
                        .strip_prefix('.')
                        .unwrap_or(after_bracket)
                        .to_string();

                    if !col_name.is_empty() {
                        table_rows
                            .entry(table_name.to_string())
                            .or_default()
                            .push((row_idx, col_name, odin_value_to_dyn_for_table(value)));
                    }
                }
            }
        }
    }

    // Build lookup tables from collected rows with full column/row data.
    for (table_name, rows) in &table_rows {
        // Discover column names in order of first appearance.
        let mut columns: Vec<String> = Vec::new();
        for (_, col, _) in rows {
            if !columns.contains(col) {
                columns.push(col.clone());
            }
        }

        // Find the maximum row index to build the row array.
        let max_row = rows.iter().map(|(idx, _, _)| *idx).max().unwrap_or(0);

        // Group by row index.
        let mut row_data: HashMap<usize, HashMap<String, DynValue>> = HashMap::new();
        for (row_idx, col, val) in rows {
            row_data
                .entry(*row_idx)
                .or_default()
                .insert(col.clone(), val.clone());
        }

        // Build ordered row array.
        let mut built_rows: Vec<Vec<DynValue>> = Vec::new();
        for row_idx in 0..=max_row {
            if let Some(rd) = row_data.get(&row_idx) {
                let row: Vec<DynValue> = columns.iter().map(|col| {
                    rd.get(col).cloned().unwrap_or(DynValue::Null)
                }).collect();
                built_rows.push(row);
            }
        }

        let default = table_defaults.get(table_name).cloned();

        tables.insert(
            table_name.clone(),
            LookupTable {
                name: table_name.clone(),
                columns,
                rows: built_rows,
                default,
            },
        );
    }

    tables
}

// ─────────────────────────────────────────────────────────────────────────────
// Imports
// ─────────────────────────────────────────────────────────────────────────────

/// Parse import references from the document's import directives.
fn parse_imports(doc: &OdinDocument) -> Vec<ImportRef> {
    doc.imports
        .iter()
        .map(|imp| ImportRef {
            path: imp.path.clone(),
            alias: imp.alias.clone(),
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Confidential / Strict Types
// ─────────────────────────────────────────────────────────────────────────────

/// Parse the `enforceConfidential` metadata value.
fn parse_enforce_confidential(doc: &OdinDocument) -> Option<ConfidentialMode> {
    let val = get_meta_string(doc, "enforceConfidential")?;
    match val.as_str() {
        "redact" => Some(ConfidentialMode::Redact),
        "mask" => Some(ConfidentialMode::Mask),
        _ => None,
    }
}

/// Parse the `strictTypes` metadata value.
fn parse_strict_types(doc: &OdinDocument) -> bool {
    get_meta_string(doc, "strictTypes")
        .map(|v| v == "true")
        .or_else(|| {
            doc.metadata
                .get(&"strictTypes".to_string())
                .and_then(super::super::types::values::OdinValue::as_bool)
        })
        .unwrap_or(false)
}

/// Merge directive-based modifiers (`:confidential`, `:required`, `:deprecated`)
/// into the existing `Option<OdinModifiers>` from the document's prefix modifiers.
fn merge_directive_modifiers(
    modifiers: Option<OdinModifiers>,
    directives: &[crate::types::values::OdinDirective],
) -> Option<OdinModifiers> {
    let has_conf = directives.iter().any(|d| d.name == "confidential");
    let has_req = directives.iter().any(|d| d.name == "required");
    let has_dep = directives.iter().any(|d| d.name == "deprecated");
    let has_attr = directives.iter().any(|d| d.name == "attr");
    if !has_conf && !has_req && !has_dep && !has_attr {
        return modifiers;
    }
    let mut m = modifiers.unwrap_or(OdinModifiers {
        required: false,
        deprecated: false,
        confidential: false,
        attr: false,
    });
    if has_conf { m.confidential = true; }
    if has_req { m.required = true; }
    if has_dep { m.deprecated = true; }
    if has_attr { m.attr = true; }
    Some(m)
}

// ─────────────────────────────────────────────────────────────────────────────
// Segments
// ─────────────────────────────────────────────────────────────────────────────

/// Parse transform segments from non-metadata assignments.
///
/// Assignments are grouped by their top-level section name (the part before the
/// first `.` in the key). Each group becomes a `TransformSegment`. Fields that
/// start with `_` are treated as directives (e.g., `_pass`, `_loop`, `_from`,
/// `_if`). All other fields become `FieldMapping` entries.
fn parse_segments(
    assignments: OrderedMap<String, OdinValue>,
    mut modifiers: OrderedMap<String, OdinModifiers>,
) -> Vec<TransformSegment> {
    // Group assignments by top-level section, preserving insertion order.
    // Section count is typically small (<20), so a Vec with linear scan
    // beats hashing.
    let mut sections: Vec<(String, Vec<(String, OdinValue, Option<OdinModifiers>)>)> = Vec::new();
    let has_modifiers = !modifiers.is_empty();

    for (mut key, value) in assignments.into_iter() {
        if key.starts_with("$.") {
            continue;
        }
        let mods = if has_modifiers { modifiers.remove(&key) } else { None };

        // Reuse the key allocation: truncate for section, slice for field.
        let (section, field) = if let Some(dot) = key.find('.') {
            let field = key[dot + 1..].to_string();
            key.truncate(dot);
            (key, field)
        } else {
            (String::new(), key)
        };

        if section.starts_with('$') {
            continue;
        }

        if let Some(slot) = sections.iter_mut().find(|(s, _)| s == &section) {
            slot.1.push((field, value, mods));
        } else {
            sections.push((section, vec![(field, value, mods)]));
        }
    }

    sections
        .into_iter()
        .map(|(section_name, fields)| build_segment(section_name, fields))
        .collect()
}

/// Check if a child section needs to be a proper child segment (has directives or array loops)
/// vs. being flattened into the parent as dotted-path mappings.
fn needs_child_segment(child_name: &str, fields: &[(String, OdinValue, Option<OdinModifiers>)]) -> bool {
    // Array segment names (e.g., "items[]") always need their own segment
    if child_name.contains("[]") {
        return true;
    }
    // Check if any field contains directives or array notation
    fields.iter().any(|(field, _, _)| {
        field.starts_with('_') || field.contains("[]")
    })
}

/// Split a fully-qualified assignment key into (`section_name`, `field_name`).
///
/// If the key contains a `.`, the part before the first `.` is the section name
/// and the rest is the field name. Otherwise, the section is `""` (root) and
/// the field is the whole key.
fn split_section_key(key: &str) -> (&str, &str) {
    if let Some(dot_pos) = key.find('.') {
        (&key[..dot_pos], &key[dot_pos + 1..])
    } else {
        ("", key)
    }
}

/// Build a `TransformSegment` from a section name and its field assignments.
///
/// Uses an interleaved `items` list to preserve the original order of
/// mappings and child segments from the transform document.
fn build_segment(
    name: String,
    fields: Vec<(String, OdinValue, Option<OdinModifiers>)>,
) -> TransformSegment {
    use crate::types::transform::SegmentItem;

    let mut source_path: Option<String> = None;
    let mut discriminator: Option<Discriminator> = None;
    let mut pass: Option<usize> = None;
    let mut condition: Option<String> = None;
    let mut children: Vec<TransformSegment> = Vec::new();

    let mut child_fields: FxHashMap<String, Vec<(String, OdinValue, Option<OdinModifiers>)>> =
        FxHashMap::default();

    // Track interleaved order: either a direct mapping or a child section reference
    enum ItemRef {
        Mapping(FieldMapping),
        ChildRef(String),
    }
    let mut item_order: Vec<ItemRef> = Vec::new();
    let mut seen_children: FxHashSet<String> = FxHashSet::default();

    for (field, value, modifiers) in fields {
        // Check for nested sub-section (e.g., "Items.Name" under "Customer").
        if let Some(dot_pos) = field.find('.') {
            let child_section = field[..dot_pos].to_string();
            let child_field = field[dot_pos + 1..].to_string();
            // Record the child reference at first occurrence (preserves position)
            if seen_children.insert(child_section.clone()) {
                item_order.push(ItemRef::ChildRef(child_section.clone()));
            }
            child_fields
                .entry(child_section)
                .or_default()
                .push((child_field, value, modifiers));
            continue;
        }

        if field.starts_with('_') {
            // Directive field
            match field.as_str() {
                "_loop" | "_from" => {
                    source_path = Some(odin_value_to_string(&value));
                }
                "_pass" => {
                    if let Some(n) = value.as_i64() {
                        pass = Some(n as usize);
                    } else if let Ok(n) = odin_value_to_string(&value).parse::<usize>() {
                        pass = Some(n);
                    }
                }
                "_if" | "_when" => {
                    condition = Some(odin_value_to_string(&value));
                }
                "_discriminator" => {
                    if let OdinValue::Reference { path, .. } = &value {
                        discriminator = Some(Discriminator {
                            path: path.clone(),
                            value: String::new(),
                        });
                    } else {
                        let s = odin_value_to_string(&value);
                        discriminator = Some(Discriminator {
                            path: s,
                            value: String::new(),
                        });
                    }
                }
                "_discriminatorValue" | "_value" => {
                    if let Some(ref mut disc) = discriminator {
                        disc.value = odin_value_to_string(&value);
                    } else {
                        discriminator = Some(Discriminator {
                            path: String::new(),
                            value: odin_value_to_string(&value),
                        });
                    }
                }
                _ => {
                    let m = build_field_mapping(field, value, modifiers);
                    item_order.push(ItemRef::Mapping(m));
                }
            }
        } else {
            let m = build_field_mapping(field, value, modifiers);
            item_order.push(ItemRef::Mapping(m));
        }
    }

    // Build interleaved `items` and the flat `mappings` list together so
    // each FieldMapping is cloned exactly once per output side.
    let mut items: Vec<SegmentItem> = Vec::with_capacity(item_order.len());
    let mut mappings: Vec<FieldMapping> = Vec::with_capacity(item_order.len());
    for item_ref in item_order {
        match item_ref {
            ItemRef::Mapping(m) => {
                mappings.push(m.clone());
                items.push(SegmentItem::Mapping(m));
            }
            ItemRef::ChildRef(child_name) => {
                if let Some(cf) = child_fields.remove(&child_name) {
                    if needs_child_segment(&child_name, &cf) {
                        let seg = build_segment(child_name, cf);
                        children.push(seg.clone());
                        items.push(SegmentItem::Child(seg));
                    } else {
                        // Flatten: emit as dotted-path mappings in order
                        for (child_field, value, mods) in cf {
                            let full_target = format!("{child_name}.{child_field}");
                            let m = build_field_mapping(full_target, value, mods);
                            mappings.push(m.clone());
                            items.push(SegmentItem::Mapping(m));
                        }
                    }
                }
            }
        }
    }

    // Determine if segment is an array (name ends with [])
    let is_array = name.ends_with("[]");
    let path = name.clone();

    // Build segment directives from parsed underscore-prefixed fields
    let mut directives = Vec::new();
    if let Some(ref sp) = source_path {
        directives.push(SegmentDirective {
            directive_type: "loop".to_string(),
            value: Some(sp.clone()),
        });
    }
    if let Some(p) = pass {
        directives.push(SegmentDirective {
            directive_type: "pass".to_string(),
            value: Some(p.to_string()),
        });
    }
    if let Some(ref c) = condition {
        directives.push(SegmentDirective {
            directive_type: "if".to_string(),
            value: Some(c.clone()),
        });
    }
    if let Some(ref d) = discriminator {
        directives.push(SegmentDirective {
            directive_type: "type".to_string(),
            value: Some(d.value.clone()),
        });
    }

    TransformSegment {
        name,
        path,
        source_path,
        discriminator,
        is_array,
        directives,
        mappings,
        children,
        items,
        pass,
        condition,
    }
}

/// Build a `FieldMapping` from a target name, a source value, and optional
/// modifiers. Merges trailing directives from the expression and any
/// formatting directives promoted from verb args.
fn build_field_mapping(
    target: String,
    value: OdinValue,
    modifiers: Option<OdinModifiers>,
) -> FieldMapping {
    // Fast path: bare reference like `@.path :type X` — the dominant shape in
    // transform docs. Moves `path` and `directives` out instead of cloning.
    if let OdinValue::Reference { path, directives, .. } = value {
        let merged_mods = merge_directive_modifiers(modifiers, &directives);
        return FieldMapping {
            target,
            expression: FieldExpression::Copy(path),
            directives,
            modifiers: merged_mods,
        };
    }

    let mut dirs = value.directives().to_vec();
    let (expr, trailing_dirs) = value_to_field_expression_with_directives(&value);
    for td in trailing_dirs {
        if !dirs.iter().any(|d| d.name == td.name) {
            dirs.push(td);
        }
    }
    let fmt_dirs = collect_formatting_directives(&expr);
    for fd in fmt_dirs {
        if !dirs.iter().any(|d| d.name == fd.name) {
            dirs.push(fd);
        }
    }
    let merged_mods = merge_directive_modifiers(modifiers, &dirs);
    FieldMapping {
        target,
        expression: expr,
        directives: dirs,
        modifiers: merged_mods,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Verb Arity Map
// ─────────────────────────────────────────────────────────────────────────────

/// Get the expected argument count for a verb. Returns -1 for variadic verbs.
fn get_verb_arity(verb: &str) -> i32 {
    match verb {
        // Arity 0
        "today" | "now" => 0,

        // Arity 1
        "upper" | "lower" | "trim" | "trimLeft" | "trimRight"
        | "coerceString" | "coerceNumber" | "coerceInteger" | "coerceBoolean"
        | "coerceDate" | "coerceTimestamp" | "tryCoerce"
        | "toArray" | "toObject"
        | "not" | "isNull" | "isString" | "isNumber" | "isBoolean"
        | "isArray" | "isObject" | "isDate" | "typeOf"
        | "capitalize" | "titleCase" | "length" | "reverseString"
        | "camelCase" | "snakeCase" | "kebabCase" | "pascalCase"
        | "slugify" | "normalizeSpace" | "stripAccents" | "clean"
        | "wordCount" | "soundex"
        | "abs" | "floor" | "ceil" | "negate" | "sign" | "trunc"
        | "isFinite" | "isNaN" | "ln" | "log10" | "exp" | "sqrt"
        | "formatInteger" | "formatCurrency"
        | "startOfDay" | "endOfDay" | "startOfMonth" | "endOfMonth"
        | "startOfYear" | "endOfYear" | "dayOfWeek" | "weekOfYear"
        | "quarter" | "isLeapYear" | "toUnix" | "fromUnix"
        | "base64Encode" | "base64Decode" | "urlEncode" | "urlDecode"
        | "jsonEncode" | "jsonDecode" | "hexEncode" | "hexDecode"
        | "sha256" | "md5" | "sha1" | "sha512" | "crc32"
        | "flatten" | "distinct" | "sort" | "sortDesc" | "reverse"
        | "compact" | "unique" | "cumsum" | "cumprod"
        | "sum" | "count" | "min" | "max" | "avg" | "first" | "last"
        | "std" | "stdSample" | "variance" | "varianceSample"
        | "median" | "mode" | "rowNumber"
        | "uuid" | "sequence" | "resetSequence"
        | "keys" | "values" | "entries"
        | "toRadians" | "toDegrees"
        | "nextBusinessDay" | "formatDuration" => 1,

        // Arity 2
        "ifNull" | "ifEmpty"
        | "and" | "or" | "xor" | "eq" | "ne" | "lt" | "lte" | "gt" | "gte"
        | "contains" | "startsWith" | "endsWith" | "truncate" | "join"
        | "mask" | "match" | "leftOf" | "rightOf" | "repeat"
        | "matches" | "levenshtein" | "tokenize"
        | "add" | "subtract" | "multiply" | "divide" | "mod"
        | "formatNumber" | "pow" | "log" | "formatPercent" | "parseInt"
        | "formatLocaleNumber" | "round"
        | "formatDate" | "parseDate" | "addDays" | "addMonths" | "addYears"
        | "addHours" | "addMinutes" | "addSeconds" | "formatTime"
        | "formatTimestamp" | "parseTimestamp" | "isBefore" | "isAfter"
        | "daysBetweenDates" | "ageFromDate" | "isValidDate"
        | "formatLocaleDate"
        | "accumulate" | "set"
        | "percentile" | "quantile" | "covariance" | "correlation"
        | "weightedAvg" | "npv" | "irr" | "zscore"
        | "sortBy" | "map" | "indexOf" | "at" | "includes" | "concatArrays"
        | "zip" | "groupBy" | "take" | "drop" | "chunk" | "pluck"
        | "dedupe" | "diff" | "pctChange" | "limit"
        | "nanoid"
        | "has" | "merge" | "jsonPath"
        | "assert"
        | "formatPhone" | "movingAvg" | "businessDays" => 2,

        // Arity 3
        "ifElse" | "between"
        | "substring" | "replace" | "replaceRegex" | "padLeft" | "padRight"
        | "pad" | "split" | "extract" | "wrap" | "center"
        | "clamp" | "random" | "safeDivide"
        | "dateDiff" | "isBetween"
        | "compound" | "discount" | "pmt" | "fv" | "pv" | "depreciation"
        | "slice" | "range" | "shift" | "rank" | "lag" | "lead"
        | "sample" | "fillMissing"
        | "get"
        | "reduce" | "pivot" | "unpivot" | "convertUnit" => 3,

        // Arity 4
        "rate" | "nper"
        | "filter" | "every" | "some" | "find" | "findIndex" | "partition"
        | "bearing" | "midpoint" => 4,

        // Arity 5
        "distance" | "interpolate" => 5,

        // Arity 6
        "inBoundingBox" => 6,

        // Variadic (including unknown verbs)
        _ => -1,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transform Expression Parser
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a string value as a transform expression, also collecting any trailing
/// directives (`:pos`, `:len`, `:leftPad`, etc.) that follow the expression.
fn parse_string_expression_with_directives(raw: &str) -> (FieldExpression, Vec<crate::types::values::OdinDirective>) {
    let trimmed = raw.trim();

    if trimmed.starts_with('%') {
        let (expr, consumed) = parse_verb_expression(trimmed);
        // Parse remaining directives after the verb expression
        let remaining = &trimmed[consumed..];
        let dirs = parse_remaining_directives(remaining);
        (expr, dirs)
    } else if let Some(after_at) = trimmed.strip_prefix('@') {
        // Copy expression: @path — also collect trailing directives
        let path = extract_path_token(after_at);
        let path_end = 1 + path.len();
        let remaining = &trimmed[path_end..];
        let dirs = parse_remaining_directives(remaining);
        (FieldExpression::Copy(path), dirs)
    } else {
        // Literal string — check for trailing directives after the literal
        let dirs = Vec::new();
        (FieldExpression::Literal(OdinValues::string(raw)), dirs)
    }
}

/// Parse remaining directives from a string (e.g., `:pos 31 :len 3 :leftPad "0"`).
fn parse_remaining_directives(s: &str) -> Vec<crate::types::values::OdinDirective> {
    let mut dirs = Vec::new();
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return dirs;
    }
    let mut pos = 0;
    let bytes = trimmed.as_bytes();
    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() || bytes[pos] != b':' {
            break;
        }
        let (dir, consumed) = parse_extraction_directive(&trimmed[pos..]);
        if let Some(d) = dir {
            dirs.push(d);
            pos += consumed;
        } else {
            break;
        }
    }
    dirs
}

/// Parse a verb expression: `%verbName arg1 arg2 ...`
///
/// Returns the expression and how many bytes were consumed.
fn parse_verb_expression(raw: &str) -> (FieldExpression, usize) {
    let is_custom = raw.starts_with("%&");
    let start = if is_custom { 2 } else { 1 };

    // Find verb name (ends at whitespace or end of string)
    let verb_end = raw[start..]
        .find(char::is_whitespace)
        .map_or(raw.len(), |p| p + start);
    let verb = &raw[start..verb_end];

    if verb.is_empty() {
        return (FieldExpression::Literal(OdinValues::string(raw)), raw.len());
    }

    let arity = get_verb_arity(verb);

    // Parse arguments
    let args_str = if verb_end < raw.len() {
        &raw[verb_end..]
    } else {
        ""
    };
    let (args, args_consumed) = parse_expression_args(args_str, arity);

    let verb_call = VerbCall {
        verb: verb.to_string(),
        is_custom,
        args,
    };

    (FieldExpression::Transform(verb_call), verb_end + args_consumed)
}

/// Parse a verb expression as a `VerbArg` (for recursive use).
fn parse_verb_arg_expression(raw: &str) -> (VerbArg, usize) {
    let is_custom = raw.starts_with("%&");
    let start = if is_custom { 2 } else { 1 };

    let verb_end = raw[start..]
        .find(char::is_whitespace)
        .map_or(raw.len(), |p| p + start);
    let verb = &raw[start..verb_end];

    if verb.is_empty() {
        return (VerbArg::Literal(OdinValues::string(raw)), raw.len());
    }

    let arity = get_verb_arity(verb);

    let args_str = if verb_end < raw.len() {
        &raw[verb_end..]
    } else {
        ""
    };
    let (args, args_consumed) = parse_expression_args(args_str, arity);

    let verb_call = VerbCall {
        verb: verb.to_string(),
        is_custom,
        args,
    };

    (VerbArg::Verb(verb_call), verb_end + args_consumed)
}

/// Parse arguments from a string, respecting verb arity.
///
/// `limit` is the max number of args to consume (-1 = variadic/unlimited).
fn parse_expression_args(args_str: &str, limit: i32) -> (Vec<VerbArg>, usize) {
    let mut args = Vec::new();
    let mut pos = 0;
    let bytes = args_str.as_bytes();

    // Skip leading whitespace
    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
        pos += 1;
    }

    while pos < bytes.len() {
        // Stop if we've reached the argument limit
        if limit >= 0 && args.len() >= limit as usize {
            break;
        }

        // Stop at modifiers (: prefix)
        if bytes[pos] == b':' {
            break;
        }

        if bytes[pos] == b'%' {
            // Nested verb expression
            let (arg, consumed) = parse_verb_arg_expression(&args_str[pos..]);
            args.push(arg);
            pos += consumed;
        } else if bytes[pos] == b'@' {
            // Reference: @path until whitespace
            let path_start = pos + 1;
            let path_end = find_token_end(&args_str[path_start..]) + path_start;
            let path = &args_str[path_start..path_end];
            pos = path_end;

            // Skip whitespace before potential directives
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }

            // Collect extraction directives (:pos, :len, :field, :trim) that follow the reference
            let mut ref_directives = Vec::new();
            while pos < bytes.len() && bytes[pos] == b':' {
                let (dir, consumed) = parse_extraction_directive(&args_str[pos..]);
                if let Some(d) = dir {
                    ref_directives.push(d);
                    pos += consumed;
                    // Skip whitespace after directive
                    while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                        pos += 1;
                    }
                } else {
                    break; // Not a recognized extraction directive — stop
                }
            }

            args.push(VerbArg::Reference(path.to_string(), ref_directives));
        } else if bytes[pos] == b'"' {
            // Quoted string literal
            let (s, consumed) = parse_quoted_string_arg(&args_str[pos..]);
            args.push(VerbArg::Literal(OdinValues::string(&s)));
            pos += consumed;
        } else if pos + 1 < bytes.len() && bytes[pos] == b'#' && bytes[pos + 1] == b'$' {
            // Currency: #$99.99
            let num_start = pos + 2;
            let num_end = find_number_end(&args_str[num_start..]) + num_start;
            if let Ok(v) = args_str[num_start..num_end].parse::<f64>() {
                let dp = args_str[num_start..num_end]
                    .find('.')
                    .map_or(2, |d| (num_end - num_start - d - 1) as u8);
                args.push(VerbArg::Literal(OdinValue::Currency {
                    value: v,
                    decimal_places: dp,
                    currency_code: None,
                    raw: Some(args_str[num_start..num_end].to_string()),
                    modifiers: None,
                    directives: Vec::new(),
                }));
            }
            pos = num_end;
        } else if pos + 1 < bytes.len() && bytes[pos] == b'#' && bytes[pos + 1] == b'#' {
            // Integer: ##42
            let num_start = pos + 2;
            let num_end = find_number_end(&args_str[num_start..]) + num_start;
            let raw = &args_str[num_start..num_end];
            let val = raw.parse::<i64>().unwrap_or(0);
            args.push(VerbArg::Literal(OdinValues::integer(val)));
            pos = num_end;
        } else if bytes[pos] == b'#' {
            // Number: #3.14
            let num_start = pos + 1;
            let num_end = find_number_end(&args_str[num_start..]) + num_start;
            let raw = &args_str[num_start..num_end];
            if let Ok(v) = raw.parse::<f64>() {
                args.push(VerbArg::Literal(OdinValue::Number {
                    value: v,
                    decimal_places: raw.find('.').map(|d| (raw.len() - d - 1) as u8),
                    raw: Some(raw.to_string()),
                    modifiers: None,
                    directives: Vec::new(),
                }));
            }
            pos = num_end;
        } else if bytes[pos] == b'~' {
            // Null
            args.push(VerbArg::Literal(OdinValues::null()));
            pos += 1;
        } else if args_str[pos..].starts_with("true") && (pos + 4 >= bytes.len() || bytes[pos + 4].is_ascii_whitespace()) {
            args.push(VerbArg::Literal(OdinValues::boolean(true)));
            pos += 4;
        } else if args_str[pos..].starts_with("false") && (pos + 5 >= bytes.len() || bytes[pos + 5].is_ascii_whitespace()) {
            args.push(VerbArg::Literal(OdinValues::boolean(false)));
            pos += 5;
        } else {
            // Unquoted string (table name, field name, etc.)
            let end = find_token_end(&args_str[pos..]) + pos;
            let val = &args_str[pos..end];
            args.push(VerbArg::Literal(OdinValues::string(val)));
            pos = end;
        }

        // Skip whitespace between arguments
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
    }

    (args, pos)
}

/// Extract a path token (until whitespace).
fn extract_path_token(s: &str) -> String {
    let end = find_token_end(s);
    s[..end].to_string()
}

/// Parse an extraction directive (`:pos N`, `:len N`, `:field N`, `:trim`, `:type X`, `:date`, etc.)
/// from the start of `s`. Returns the directive and how many bytes were consumed, or `None`
/// if this doesn't look like a recognized directive.
fn parse_extraction_directive(s: &str) -> (Option<crate::types::values::OdinDirective>, usize) {
    use crate::types::values::{OdinDirective, DirectiveValue};

    if !s.starts_with(':') {
        return (None, 0);
    }

    // Get directive name (after colon, until whitespace or end)
    let name_start = 1;
    let name_end = s[name_start..].find(char::is_whitespace).map_or(s.len(), |p| p + name_start);
    let name = &s[name_start..name_end];

    // Only consume directives that are recognized extraction/type/formatting directives
    let recognized = matches!(name, "pos" | "len" | "field" | "trim" | "type"
        | "date" | "time" | "duration" | "timestamp" | "boolean" | "integer" | "number"
        | "currency" | "reference" | "binary" | "percent" | "decimals" | "currencyCode"
        | "leftPad" | "rightPad" | "truncate" | "default" | "upper" | "lower");

    if !recognized {
        return (None, 0);
    }

    let mut consumed = name_end;

    // Check for a value after the directive name
    let needs_value = matches!(name, "pos" | "len" | "field" | "type" | "decimals" | "currencyCode"
        | "leftPad" | "rightPad" | "default");

    let value = if needs_value {
        // Skip whitespace
        while consumed < s.len() && s.as_bytes()[consumed].is_ascii_whitespace() {
            consumed += 1;
        }
        if consumed < s.len() {
            // Handle quoted string values (e.g., :leftPad "0")
            if s.as_bytes()[consumed] == b'"' {
                let (qstr, qconsumed) = parse_quoted_string_arg(&s[consumed..]);
                consumed += qconsumed;
                Some(DirectiveValue::String(qstr))
            } else {
                let val_end = s[consumed..].find(char::is_whitespace).map_or(s.len(), |p| p + consumed);
                let val_str = &s[consumed..val_end];
                consumed = val_end;
                // Try to parse as number first
                if let Ok(n) = val_str.parse::<f64>() {
                    Some(DirectiveValue::Number(n))
                } else {
                    Some(DirectiveValue::String(val_str.to_string()))
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    (Some(OdinDirective { name: name.to_string(), value }), consumed)
}

/// Find the end of a token (first whitespace or end of string).
fn find_token_end(s: &str) -> usize {
    s.find(char::is_whitespace).unwrap_or(s.len())
}

/// Find the end of a numeric token (digits, '.', '-', 'e', 'E', '+').
fn find_number_end(s: &str) -> usize {
    let mut i = 0;
    let bytes = s.as_bytes();
    // Allow leading minus
    if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
    }
    while i < bytes.len() {
        match bytes[i] {
            b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-' if i > 0 => i += 1,
            b'0'..=b'9' | b'.' => i += 1,
            _ => break,
        }
    }
    if i == 0 { s.len().min(1) } else { i }
}

/// Parse a quoted string argument, handling escape sequences.
fn parse_quoted_string_arg(s: &str) -> (String, usize) {
    if !s.starts_with('"') {
        return (String::new(), 0);
    }
    let mut result = String::new();
    let mut i = 1; // skip opening quote
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'"' => { result.push('"'); i += 2; }
                b'\\' => { result.push('\\'); i += 2; }
                b'n' => { result.push('\n'); i += 2; }
                b't' => { result.push('\t'); i += 2; }
                b'r' => { result.push('\r'); i += 2; }
                _ => { result.push(bytes[i] as char); i += 1; }
            }
        } else if bytes[i] == b'"' {
            i += 1; // skip closing quote
            break;
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    (result, i)
}

// ─────────────────────────────────────────────────────────────────────────────
// Field Expression Conversion
// ─────────────────────────────────────────────────────────────────────────────

/// Convert an `OdinValue` to a `FieldExpression`.
fn value_to_field_expression(value: &OdinValue) -> FieldExpression {
    value_to_field_expression_with_directives(value).0
}

/// Convert an `OdinValue` to a `FieldExpression` and collect any trailing directives
/// from verb/copy expressions (`:pos`, `:len`, `:leftPad`, `:rightPad`, etc.).
fn value_to_field_expression_with_directives(value: &OdinValue) -> (FieldExpression, Vec<crate::types::values::OdinDirective>) {
    match value {
        OdinValue::Reference { path, .. } => (FieldExpression::Copy(path.clone()), Vec::new()),

        OdinValue::Verb {
            verb,
            is_custom,
            args,
            ..
        } => {
            if args.is_empty() && verb.starts_with('%') {
                // Bare verb expression from the parser — the full expression string
                // (e.g., "%ifElse %eq @actual \"A\" ...") is stored in `verb`.
                // Re-parse it to extract the verb call and args.
                parse_string_expression_with_directives(verb)
            } else {
                // Pre-parsed verb with structured args (from nested verb parsing).
                let verb_call = VerbCall {
                    verb: verb.clone(),
                    is_custom: *is_custom,
                    args: args.iter().map(odin_value_to_verb_arg).collect(),
                };
                (FieldExpression::Transform(verb_call), Vec::new())
            }
        }

        OdinValue::Object { value: fields, .. } => {
            let field_mappings = fields
                .iter()
                .map(|(key, val)| {
                    let modifiers = val.modifiers().cloned();
                    FieldMapping {
                        target: key.clone(),
                        expression: value_to_field_expression(val),
                        directives: val.directives().to_vec(),
                        modifiers,
                    }
                })
                .collect();
            (FieldExpression::Object(field_mappings), Vec::new())
        }

        // Quoted strings: @-references are copy expressions, everything else is a literal.
        // Verb expressions (%verb ...) must be bare (unquoted) to be treated as verbs.
        OdinValue::String { value: s, .. } => {
            let trimmed = s.trim();
            if trimmed.starts_with('@') {
                parse_string_expression_with_directives(trimmed)
            } else {
                (FieldExpression::Literal(value.clone()), Vec::new())
            }
        }

        other => (FieldExpression::Literal(other.clone()), Vec::new()),
    }
}

/// Formatting directive names that should be promoted from verb args to `FieldMapping` level.
/// NOTE: :pos and :len are NOT included here because they serve dual purpose:
/// - For import transforms: extraction directives on verb arg references (applied at arg level)
/// - For export transforms: output positioning (the formatter reads them directly from verb args)
///
/// Promoting them would cause double-extraction for imports.
const FORMATTING_DIRECTIVE_NAMES: &[&str] = &[
    "leftPad", "rightPad", "truncate", "default", "upper", "lower",
];

/// Collect formatting directives from a `FieldExpression`'s verb args.
///
/// Mirrors the TypeScript `collectFormattingDirectives()` — extracts directives like
/// `:pos`, `:len`, `:leftPad`, `:rightPad` from verb arg references and returns them.
/// The directives remain on the verb args (copy, not move) since the engine may
/// conditionally apply them as extraction directives for raw-text source formats.
fn collect_formatting_directives(expr: &FieldExpression) -> Vec<crate::types::values::OdinDirective> {
    let mut collected = Vec::new();
    if let FieldExpression::Transform(ref verb_call) = expr {
        collect_from_verb_args(&verb_call.args, &mut collected);
    }
    collected
}

fn collect_from_verb_args(args: &[VerbArg], collected: &mut Vec<crate::types::values::OdinDirective>) {
    for arg in args {
        match arg {
            VerbArg::Reference(_, directives) => {
                for dir in directives {
                    if FORMATTING_DIRECTIVE_NAMES.contains(&dir.name.as_str()) {
                        // Only add if not already collected (avoid duplicates)
                        if !collected.iter().any(|d| d.name == dir.name) {
                            collected.push(dir.clone());
                        }
                    }
                }
            }
            VerbArg::Verb(nested) => {
                collect_from_verb_args(&nested.args, collected);
            }
            _ => {}
        }
    }
}

/// Convert an `OdinValue` (verb argument) to a `VerbArg`.
///
/// - `OdinValue::Reference` → `VerbArg::Reference(path)`
/// - `OdinValue::Verb` → `VerbArg::Verb(VerbCall)` (recursive)
/// - Everything else → `VerbArg::Literal(value)`
fn odin_value_to_verb_arg(value: &OdinValue) -> VerbArg {
    match value {
        OdinValue::Reference { path, directives, .. } => VerbArg::Reference(path.clone(), directives.clone()),

        OdinValue::Verb {
            verb,
            is_custom,
            args,
            ..
        } => {
            let verb_call = VerbCall {
                verb: verb.clone(),
                is_custom: *is_custom,
                args: args.iter().map(odin_value_to_verb_arg).collect(),
            };
            VerbArg::Verb(verb_call)
        }

        other => VerbArg::Literal(other.clone()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pass Collection
// ─────────────────────────────────────────────────────────────────────────────

/// Collect distinct pass numbers from all segments, sorted in ascending order.
fn collect_passes(segments: &[TransformSegment]) -> Vec<usize> {
    let mut passes: Vec<usize> = Vec::new();
    collect_passes_recursive(segments, &mut passes);
    passes.sort_unstable();
    passes.dedup();
    passes
}

fn collect_passes_recursive(segments: &[TransformSegment], passes: &mut Vec<usize>) {
    for seg in segments {
        if let Some(p) = seg.pass {
            passes.push(p);
        }
        collect_passes_recursive(&seg.children, passes);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Get a metadata value as a `String`, handling both `OdinValue::String` and
/// other types by converting through `Display`.
fn get_meta_string(doc: &OdinDocument, key: &str) -> Option<String> {
    let value = doc.metadata.get(key)?;
    Some(odin_value_to_string(value))
}

/// Convert any `OdinValue` to a plain string representation.
fn odin_value_to_string(value: &OdinValue) -> String {
    match value {
        OdinValue::String { value, .. }
        | OdinValue::Time { value, .. }
        | OdinValue::Duration { value, .. } => value.clone(),
        OdinValue::Boolean { value, .. } => value.to_string(),
        OdinValue::Integer { value, raw, .. } => {
            raw.as_deref().unwrap_or(&value.to_string()).to_string()
        }
        OdinValue::Number { value, raw, .. }
        | OdinValue::Currency { value, raw, .. }
        | OdinValue::Percent { value, raw, .. } => {
            raw.as_deref().unwrap_or(&value.to_string()).to_string()
        }
        OdinValue::Null { .. } => "~".to_string(),
        OdinValue::Reference { path, .. } => format!("@{path}"),
        OdinValue::Date { raw, .. }
        | OdinValue::Timestamp { raw, .. } => raw.clone(),
        OdinValue::Binary { .. } => "<binary>".to_string(),
        OdinValue::Verb { verb, .. } => format!("%{verb}"),
        OdinValue::Array { items, .. } => format!("[{} items]", items.len()),
        OdinValue::Object { value, .. } => format!("{{{} fields}}", value.len()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::document::OdinDocumentBuilder;
    use crate::types::values::OdinValues;

    #[test]
    fn test_parse_metadata() {
        let doc = OdinDocumentBuilder::new()
            .metadata("odin", OdinValues::string("1.0.0"))
            .metadata("transform", OdinValues::string("1.0.0"))
            .metadata("direction", OdinValues::string("json->odin"))
            .metadata("name", OdinValues::string("my-transform"))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.metadata.odin_version.as_deref(), Some("1.0.0"));
        assert_eq!(transform.metadata.transform_version.as_deref(), Some("1.0.0"));
        assert_eq!(transform.metadata.direction.as_deref(), Some("json->odin"));
        assert_eq!(transform.metadata.name.as_deref(), Some("my-transform"));
    }

    #[test]
    fn test_parse_source_target_config() {
        let doc = OdinDocumentBuilder::new()
            .metadata("source.format", OdinValues::string("json"))
            .metadata("target.format", OdinValues::string("odin"))
            .metadata("target.root", OdinValues::string("Policy"))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);

        let source = transform.source.as_ref().unwrap();
        assert_eq!(source.format, "json");

        assert_eq!(transform.target.format, "odin");
        assert_eq!(transform.target.options.get("root").unwrap(), "Policy");
    }

    #[test]
    fn test_parse_constants() {
        let doc = OdinDocumentBuilder::new()
            .metadata("const.version", OdinValues::string("2.0"))
            .metadata("const.maxRetries", OdinValues::integer(3))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.constants.len(), 2);
        assert_eq!(
            transform.constants.get("version").unwrap().as_str(),
            Some("2.0")
        );
        assert_eq!(
            transform.constants.get("maxRetries").unwrap().as_i64(),
            Some(3)
        );
    }

    #[test]
    fn test_parse_accumulators() {
        let doc = OdinDocumentBuilder::new()
            .metadata("accumulator.total", OdinValues::integer(0))
            .metadata("accumulator.count", OdinValues::integer(0))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.accumulators.len(), 2);
        assert_eq!(
            transform.accumulators.get("total").unwrap().initial.as_i64(),
            Some(0)
        );
    }

    #[test]
    fn test_accumulator_persist_skipped() {
        let doc = OdinDocumentBuilder::new()
            .metadata("accumulator.total", OdinValues::integer(0))
            .metadata("accumulator.total._persist", OdinValues::boolean(true))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.accumulators.len(), 1);
        assert!(transform.accumulators.contains_key("total"));
    }

    #[test]
    fn test_parse_enforce_confidential() {
        let doc = OdinDocumentBuilder::new()
            .metadata("enforceConfidential", OdinValues::string("redact"))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.enforce_confidential, Some(ConfidentialMode::Redact));
    }

    #[test]
    fn test_parse_enforce_confidential_mask() {
        let doc = OdinDocumentBuilder::new()
            .metadata("enforceConfidential", OdinValues::string("mask"))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.enforce_confidential, Some(ConfidentialMode::Mask));
    }

    #[test]
    fn test_parse_strict_types() {
        let doc = OdinDocumentBuilder::new()
            .metadata("strictTypes", OdinValues::string("true"))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert!(transform.strict_types);
    }

    #[test]
    fn test_parse_strict_types_boolean() {
        let doc = OdinDocumentBuilder::new()
            .metadata("strictTypes", OdinValues::boolean(true))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert!(transform.strict_types);
    }

    #[test]
    fn test_parse_segments_with_reference() {
        let doc = OdinDocumentBuilder::new()
            .metadata("direction", OdinValues::string("json->odin"))
            .set("Customer.Name", OdinValues::reference(".name"))
            .set("Customer.Age", OdinValues::reference(".age"))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.segments.len(), 1);
        assert_eq!(transform.segments[0].name, "Customer");
        assert_eq!(transform.segments[0].mappings.len(), 2);

        let name_mapping = &transform.segments[0].mappings[0];
        assert_eq!(name_mapping.target, "Name");
        match &name_mapping.expression {
            FieldExpression::Copy(path) => assert_eq!(path, ".name"),
            other => panic!("Expected Copy, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_segments_with_literal() {
        let doc = OdinDocumentBuilder::new()
            .set("Output.Status", OdinValues::string("active"))
            .set("Output.Count", OdinValues::integer(42))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.segments.len(), 1);

        let status_mapping = &transform.segments[0].mappings[0];
        assert_eq!(status_mapping.target, "Status");
        match &status_mapping.expression {
            FieldExpression::Literal(OdinValue::String { value, .. }) => {
                assert_eq!(value, "active");
            }
            other => panic!("Expected Literal string, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_segments_with_verb() {
        let doc = OdinDocumentBuilder::new()
            .set(
                "Output.FullName",
                OdinValues::verb(
                    "concat",
                    vec![
                        OdinValues::reference(".first"),
                        OdinValues::string(" "),
                        OdinValues::reference(".last"),
                    ],
                ),
            )
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.segments.len(), 1);

        let mapping = &transform.segments[0].mappings[0];
        assert_eq!(mapping.target, "FullName");
        match &mapping.expression {
            FieldExpression::Transform(vc) => {
                assert_eq!(vc.verb, "concat");
                assert_eq!(vc.args.len(), 3);
                match &vc.args[0] {
                    VerbArg::Reference(p, _) => assert_eq!(p, ".first"),
                    other => panic!("Expected Reference, got {:?}", other),
                }
                match &vc.args[1] {
                    VerbArg::Literal(OdinValue::String { value, .. }) => {
                        assert_eq!(value, " ");
                    }
                    other => panic!("Expected Literal string, got {:?}", other),
                }
            }
            other => panic!("Expected Transform, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_segment_directives() {
        let doc = OdinDocumentBuilder::new()
            .set("Items._loop", OdinValues::reference(".items"))
            .set("Items._pass", OdinValues::integer(2))
            .set("Items._if", OdinValues::string("@.active == true"))
            .set("Items.Name", OdinValues::reference(".name"))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.segments.len(), 1);

        let seg = &transform.segments[0];
        assert_eq!(seg.name, "Items");
        assert_eq!(seg.source_path.as_deref(), Some("@.items"));
        assert_eq!(seg.pass, Some(2));
        assert_eq!(seg.condition.as_deref(), Some("@.active == true"));
        assert_eq!(seg.mappings.len(), 1);
        assert_eq!(seg.mappings[0].target, "Name");
    }

    #[test]
    fn test_passes_collected() {
        let doc = OdinDocumentBuilder::new()
            .set("A._pass", OdinValues::integer(1))
            .set("A.X", OdinValues::reference(".x"))
            .set("B._pass", OdinValues::integer(2))
            .set("B.Y", OdinValues::reference(".y"))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.passes, vec![1, 2]);
    }

    #[test]
    fn test_no_source_config_when_absent() {
        let doc = OdinDocumentBuilder::new()
            .metadata("direction", OdinValues::string("json->odin"))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert!(transform.source.is_none());
    }

    #[test]
    fn test_empty_document_produces_empty_transform() {
        let doc = OdinDocumentBuilder::new().build().unwrap();

        let transform = parse_transform_doc(doc);
        assert!(transform.metadata.odin_version.is_none());
        assert!(transform.source.is_none());
        assert_eq!(transform.target.format, "");
        assert!(transform.constants.is_empty());
        assert!(transform.accumulators.is_empty());
        assert!(transform.tables.is_empty());
        assert!(transform.segments.is_empty());
        assert!(transform.imports.is_empty());
        assert!(transform.passes.is_empty());
        assert!(transform.enforce_confidential.is_none());
        assert!(!transform.strict_types);
    }

    #[test]
    fn test_multiple_segments_preserve_order() {
        let doc = OdinDocumentBuilder::new()
            .set("Alpha.A", OdinValues::reference(".a"))
            .set("Beta.B", OdinValues::reference(".b"))
            .set("Gamma.C", OdinValues::reference(".c"))
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        assert_eq!(transform.segments.len(), 3);
        assert_eq!(transform.segments[0].name, "Alpha");
        assert_eq!(transform.segments[1].name, "Beta");
        assert_eq!(transform.segments[2].name, "Gamma");
    }

    #[test]
    fn test_nested_verb_args() {
        let inner = OdinValues::verb("upper", vec![OdinValues::reference(".name")]);
        let outer = OdinValues::verb("concat", vec![inner, OdinValues::string("!")]);

        let doc = OdinDocumentBuilder::new()
            .set("Out.Result", outer)
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        let mapping = &transform.segments[0].mappings[0];

        match &mapping.expression {
            FieldExpression::Transform(vc) => {
                assert_eq!(vc.verb, "concat");
                assert_eq!(vc.args.len(), 2);
                match &vc.args[0] {
                    VerbArg::Verb(inner_vc) => {
                        assert_eq!(inner_vc.verb, "upper");
                        assert_eq!(inner_vc.args.len(), 1);
                    }
                    other => panic!("Expected nested Verb, got {:?}", other),
                }
            }
            other => panic!("Expected Transform, got {:?}", other),
        }
    }

    #[test]
    fn test_field_modifiers_flow_through() {
        let mods = OdinModifiers {
            required: true,
            confidential: true,
            deprecated: false,
            attr: false,
        };
        let doc = OdinDocumentBuilder::new()
            .set_with_modifiers(
                "Sec.SSN",
                OdinValues::reference(".ssn"),
                mods.clone(),
            )
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        let mapping = &transform.segments[0].mappings[0];
        assert_eq!(mapping.target, "SSN");
        let m = mapping.modifiers.as_ref().unwrap();
        assert!(m.required);
        assert!(m.confidential);
        assert!(!m.deprecated);
    }

    #[test]
    fn test_custom_verb() {
        let doc = OdinDocumentBuilder::new()
            .set(
                "Out.Value",
                OdinValues::custom_verb("myns.transform", vec![OdinValues::reference(".x")]),
            )
            .build()
            .unwrap();

        let transform = parse_transform_doc(doc);
        let mapping = &transform.segments[0].mappings[0];
        match &mapping.expression {
            FieldExpression::Transform(vc) => {
                assert_eq!(vc.verb, "myns.transform");
                assert!(vc.is_custom);
            }
            other => panic!("Expected Transform, got {:?}", other),
        }
    }
}
