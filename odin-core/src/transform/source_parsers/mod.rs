//! Source parsers for the transform engine.
//!
//! Parse input data from various formats into `DynValue`:
//! - JSON
//! - XML
//! - CSV
//! - Fixed-width
//! - Flat (key=value)
//! - YAML

use crate::types::transform::DynValue;

/// Parse JSON text into a `DynValue`.
pub fn parse_json(input: &str) -> Result<DynValue, String> {
    let v: serde_json::Value = serde_json::from_str(input).map_err(|e| e.to_string())?;
    Ok(DynValue::from_json_value(v))
}

// ---------------------------------------------------------------------------
// CSV Parser (RFC 4180)
// ---------------------------------------------------------------------------

/// Parse CSV text into a `DynValue`.
///
/// If `has_header` is true, the first row provides field names and each
/// subsequent row becomes a `DynValue::Object`. Otherwise every row is a
/// `DynValue::Array` of values. Fields are auto-typed.
pub fn parse_csv(input: &str, delimiter: char, has_header: bool) -> Result<DynValue, String> {
    let rows = csv_split_rows(input, delimiter)?;
    if rows.is_empty() {
        return Ok(DynValue::Array(Vec::new()));
    }

    if has_header {
        if rows.is_empty() {
            return Ok(DynValue::Array(Vec::new()));
        }
        let headers: Vec<String> = rows[0].to_vec();
        let mut result = Vec::new();
        for row in &rows[1..] {
            let mut entries = Vec::new();
            for (i, header) in headers.iter().enumerate() {
                let val = if i < row.len() { &row[i] } else { "" };
                entries.push((header.clone(), infer_type(val)));
            }
            result.push(DynValue::Object(entries));
        }
        Ok(DynValue::Array(result))
    } else {
        let mut result = Vec::new();
        for row in &rows {
            let items: Vec<DynValue> = row.iter().map(|s| infer_type(s)).collect();
            result.push(DynValue::Array(items));
        }
        Ok(DynValue::Array(result))
    }
}

/// Split CSV input into rows of fields, respecting quoted fields (RFC 4180).
fn csv_split_rows(input: &str, delimiter: char) -> Result<Vec<Vec<String>>, String> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut current_field = String::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                // Peek for escaped quote
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current_field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                current_field.push(ch);
            }
        } else if ch == '"' {
            in_quotes = true;
        } else if ch == delimiter {
            current_row.push(current_field.clone());
            current_field.clear();
        } else if ch == '\r' {
            // CR – if followed by LF consume it
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            current_row.push(current_field.clone());
            current_field.clear();
            if !current_row.is_empty() {
                rows.push(current_row.clone());
            }
            current_row.clear();
        } else if ch == '\n' {
            current_row.push(current_field.clone());
            current_field.clear();
            if !current_row.is_empty() {
                rows.push(current_row.clone());
            }
            current_row.clear();
        } else {
            current_field.push(ch);
        }
    }

    if in_quotes {
        return Err("unterminated quoted field in CSV".to_string());
    }

    // Flush last field / row
    if !current_field.is_empty() || !current_row.is_empty() {
        current_row.push(current_field);
        rows.push(current_row);
    }

    Ok(rows)
}

/// Infer a `DynValue` type from a string value.
fn infer_type(s: &str) -> DynValue {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return DynValue::String(String::new());
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return DynValue::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return DynValue::Bool(false);
    }
    if trimmed.eq_ignore_ascii_case("null") {
        return DynValue::Null;
    }
    if let Ok(i) = trimmed.parse::<i64>() {
        return DynValue::Integer(i);
    }
    if let Ok(f) = trimmed.parse::<f64>() {
        // Ensure it actually contains a dot or 'e'/'E' so "42" doesn't match
        if trimmed.contains('.') || trimmed.contains('e') || trimmed.contains('E') {
            return DynValue::Float(f);
        }
    }
    DynValue::String(trimmed.to_string())
}

// ---------------------------------------------------------------------------
// XML Parser (quick-xml)
// ---------------------------------------------------------------------------

/// Parse well-formed XML into a `DynValue`.
///
/// - Elements become `Object` entries.
/// - Attributes become keys prefixed with `@`.
/// - Text content becomes `_text` if mixed, or a direct value for leaf elements.
/// - Repeated sibling elements with the same name become an `Array`.
/// - CDATA sections are treated as text.
/// - Standard XML entities are decoded.
/// - Security: nesting depth is limited to 100.
pub fn parse_xml(input: &str) -> Result<DynValue, String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(input);

    // Skip prolog (processing instructions, comments) until we hit the first element
    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let tag_name = std::str::from_utf8(e.name().as_ref())
                    .map_err(|e| e.to_string())?
                    .to_string();
                let attrs = qxml_read_attrs(e)?;
                let value = qxml_parse_element_body(&mut reader, &tag_name, attrs, 0)?;
                return Ok(DynValue::Object(vec![(tag_name, value)]));
            }
            Ok(Event::Empty(ref e)) => {
                let tag_name = std::str::from_utf8(e.name().as_ref())
                    .map_err(|e| e.to_string())?
                    .to_string();
                let attrs = qxml_read_attrs(e)?;
                let value = qxml_build_empty(attrs);
                return Ok(DynValue::Object(vec![(tag_name, value)]));
            }
            Ok(Event::Eof) => return Err("empty XML document".to_string()),
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {} // skip prolog, comments, whitespace
        }
    }
}

/// Read attributes from a quick-xml element, filtering xmlns and handling xsi:nil.
fn qxml_read_attrs(e: &quick_xml::events::BytesStart<'_>) -> Result<Vec<(String, String)>, String> {
    let mut attrs = Vec::new();
    for attr_result in e.attributes() {
        let attr = attr_result.map_err(|e| format!("XML attribute error: {e}"))?;
        let key = std::str::from_utf8(attr.key.as_ref())
            .map_err(|e| e.to_string())?
            .to_string();
        let value = attr.unescape_value()
            .map_err(|e| format!("XML attribute value error: {e}"))?
            .to_string();
        attrs.push((key, value));
    }
    Ok(attrs)
}

/// Build a DynValue for an empty/self-closing element.
fn qxml_build_empty(attrs: Vec<(String, String)>) -> DynValue {
    let is_nil = attrs.iter().any(|(k, v)| {
        (k == "xsi:nil" || k == "nil" || k == "nillable") && (v == "true" || v == "1")
    });
    if is_nil {
        return DynValue::Null;
    }

    let filtered: Vec<(String, String)> = attrs.into_iter()
        .filter(|(k, _)| !k.starts_with("xmlns"))
        .collect();

    if filtered.is_empty() {
        DynValue::Null
    } else {
        DynValue::Object(
            filtered.into_iter()
                .map(|(k, v)| (format!("@{k}"), DynValue::String(v)))
                .collect()
        )
    }
}

/// Parse the body of an XML element (after Start event), returning the DynValue.
fn qxml_parse_element_body(
    reader: &mut quick_xml::Reader<&[u8]>,
    _tag_name: &str,
    attrs: Vec<(String, String)>,
    depth: usize,
) -> Result<DynValue, String> {
    use quick_xml::events::Event;

    if depth > 100 {
        return Err("XML nesting depth limit (100) exceeded".to_string());
    }

    let is_nil = attrs.iter().any(|(k, v)| {
        (k == "xsi:nil" || k == "nil" || k == "nillable") && (v == "true" || v == "1")
    });

    let filtered_attrs: Vec<(String, String)> = attrs.into_iter()
        .filter(|(k, _)| !k.starts_with("xmlns"))
        .collect();

    let mut child_entries: Vec<(String, DynValue)> = Vec::new();
    let mut text_buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let child_tag = std::str::from_utf8(e.name().as_ref())
                    .map_err(|e| e.to_string())?
                    .to_string();
                let child_attrs = qxml_read_attrs(e)?;
                let child_val = qxml_parse_element_body(reader, &child_tag, child_attrs, depth + 1)?;
                child_entries.push((child_tag, child_val));
            }
            Ok(Event::Empty(ref e)) => {
                let child_tag = std::str::from_utf8(e.name().as_ref())
                    .map_err(|e| e.to_string())?
                    .to_string();
                let child_attrs = qxml_read_attrs(e)?;
                let child_val = qxml_build_empty(child_attrs);
                child_entries.push((child_tag, child_val));
            }
            Ok(Event::Text(ref e)) => {
                let text = e.unescape()
                    .map_err(|e| format!("XML text error: {e}"))?;
                text_buf.push_str(&text);
            }
            Ok(Event::CData(ref e)) => {
                let text = std::str::from_utf8(e.as_ref())
                    .map_err(|e| e.to_string())?;
                text_buf.push_str(text);
            }
            Ok(Event::End(_)) => break,
            Ok(Event::Eof) => return Err("unexpected end of XML".to_string()),
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }

    if is_nil {
        return Ok(DynValue::Null);
    }

    let has_attrs = !filtered_attrs.is_empty();
    let has_children = !child_entries.is_empty();
    let has_text = !text_buf.trim().is_empty();

    // Leaf text-only element with no attributes
    if !has_attrs && !has_children {
        if has_text {
            return Ok(DynValue::String(text_buf.trim().to_string()));
        }
        return Ok(DynValue::String(String::new()));
    }

    // Build object
    let mut entries: Vec<(String, DynValue)> = Vec::new();

    // Attributes first
    for (k, v) in filtered_attrs {
        entries.push((format!("@{k}"), DynValue::String(v)));
    }

    // Text
    if has_text {
        entries.push(("_text".to_string(), DynValue::String(text_buf.trim().to_string())));
    }

    // Children – group repeated names into arrays
    let mut seen: Vec<String> = Vec::new();
    let mut child_map: Vec<(String, Vec<DynValue>)> = Vec::new();
    for (name, val) in child_entries {
        if let Some(idx) = seen.iter().position(|n| n == &name) {
            child_map[idx].1.push(val);
        } else {
            seen.push(name.clone());
            child_map.push((name, vec![val]));
        }
    }
    for (name, vals) in child_map {
        // Always wrap <item> elements as arrays (common collection pattern),
        // matching TS reference behavior for consistent 1-vs-N handling.
        if name == "item" {
            entries.push((name, DynValue::Array(vals)));
        } else if vals.len() == 1 {
            // Safe: we just confirmed len() == 1, so into_iter().next() is always Some.
            if let Some(val) = vals.into_iter().next() {
                entries.push((name, val));
            }
        } else {
            entries.push((name, DynValue::Array(vals)));
        }
    }

    if entries.is_empty() {
        return Ok(DynValue::Null);
    }

    Ok(DynValue::Object(entries))
}

// ---------------------------------------------------------------------------
// Fixed-Width Parser
// ---------------------------------------------------------------------------

/// Parse fixed-width text into a `DynValue`.
///
/// `fields` is a slice of `(field_name, start_position, length)` tuples.
/// Each non-empty line produces one record `Object`. Multiple lines produce an
/// `Array` of records; a single line produces a single record `Object`.
pub fn parse_fixed_width(input: &str, fields: &[(String, usize, usize)]) -> Result<DynValue, String> {
    let lines: Vec<&str> = input.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return Ok(DynValue::Array(Vec::new()));
    }

    let mut records: Vec<DynValue> = Vec::new();
    for line in &lines {
        let chars: Vec<char> = line.chars().collect();
        let mut entries: Vec<(String, DynValue)> = Vec::new();
        for (name, start, length) in fields {
            let end = (*start + *length).min(chars.len());
            let val: String = if *start < chars.len() {
                chars[*start..end].iter().collect()
            } else {
                String::new()
            };
            let trimmed = val.trim_end().to_string();
            entries.push((name.clone(), DynValue::String(trimmed)));
        }
        records.push(DynValue::Object(entries));
    }

    if records.len() == 1 {
        // Safe: we just confirmed len() == 1.
        Ok(records.into_iter().next().unwrap_or(DynValue::Null))
    } else {
        Ok(DynValue::Array(records))
    }
}

// ---------------------------------------------------------------------------
// Flat (key=value / properties) Parser
// ---------------------------------------------------------------------------

/// Parse key=value (properties-style) text into a `DynValue`.
///
/// Supports:
/// - Comment lines starting with `#` or `;`
/// - Dot notation for nesting (`a.b.c = val`)
/// - Bracket notation for arrays (`items[0].name = foo`)
/// - Value type inference (booleans, integers, floats)
pub fn parse_flat(input: &str) -> Result<DynValue, String> {
    let mut root: Vec<(String, DynValue)> = Vec::new();

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let Some(eq_pos) = trimmed.find('=') else { continue }; // skip lines without =
        let key = trimmed[..eq_pos].trim();
        let raw_value = trimmed[eq_pos + 1..].trim();
        // Flat KVP conventions:
        // - Empty value → null
        // - Tilde (~) → null
        // - Quoted values ("...") → strip outer quotes, unescape inner
        let dyn_val = if raw_value.is_empty() || raw_value == "~" {
            DynValue::Null
        } else if raw_value.starts_with('"') && raw_value.ends_with('"') && raw_value.len() >= 2 {
            // Flat KVP uses quotes to protect values containing = or other specials.
            // The content inside quotes is literal — no backslash escaping.
            let inner = &raw_value[1..raw_value.len()-1];
            DynValue::String(inner.to_string())
        } else {
            infer_type(raw_value)
        };
        flat_set_path(&mut root, key, dyn_val)?;
    }

    Ok(DynValue::Object(root))
}

/// Set a value at a dotted/bracketed path within a nested `DynValue::Object` tree.
fn flat_set_path(root: &mut Vec<(String, DynValue)>, path: &str, value: DynValue) -> Result<(), String> {
    let segments = flat_parse_path(path);
    if segments.is_empty() {
        return Err("empty key path".to_string());
    }
    flat_set_segments(root, &segments, value);
    Ok(())
}

#[derive(Debug)]
enum PathSegment {
    Key(String),
    Index(usize),
}

fn flat_parse_path(path: &str) -> Vec<PathSegment> {
    let mut segments = Vec::new();
    let mut current = String::new();

    let mut chars = path.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '.' {
            if !current.is_empty() {
                segments.push(PathSegment::Key(current.clone()));
                current.clear();
            }
        } else if ch == '[' {
            if !current.is_empty() {
                segments.push(PathSegment::Key(current.clone()));
                current.clear();
            }
            let mut idx_str = String::new();
            while let Some(&c) = chars.peek() {
                if c == ']' {
                    chars.next();
                    break;
                }
                idx_str.push(c);
                chars.next();
            }
            if let Ok(idx) = idx_str.parse::<usize>() {
                segments.push(PathSegment::Index(idx));
            } else {
                segments.push(PathSegment::Key(idx_str));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        segments.push(PathSegment::Key(current));
    }
    segments
}

fn flat_set_segments(entries: &mut Vec<(String, DynValue)>, segments: &[PathSegment], value: DynValue) {
    if segments.is_empty() {
        return;
    }

    match &segments[0] {
        PathSegment::Key(key) => {
            if segments.len() == 1 {
                if let Some(entry) = entries.iter_mut().find(|(k, _)| k == key) {
                    entry.1 = value;
                } else {
                    entries.push((key.clone(), value));
                }
                return;
            }
            // Find-or-insert by index, then mutate via index — avoids the
            // borrow-checker conflict between find() and a follow-up mut access.
            let idx = match entries.iter().position(|(k, _)| k == key) {
                Some(i) => i,
                None => {
                    let placeholder = match &segments[1] {
                        PathSegment::Index(_) => DynValue::Array(Vec::new()),
                        PathSegment::Key(_) => DynValue::Object(Vec::new()),
                    };
                    entries.push((key.clone(), placeholder));
                    entries.len() - 1
                }
            };
            match &segments[1] {
                PathSegment::Index(_) => {
                    if !matches!(&entries[idx].1, DynValue::Array(_)) {
                        entries[idx].1 = DynValue::Array(Vec::new());
                    }
                    if let DynValue::Array(ref mut arr) = entries[idx].1 {
                        flat_set_in_array(arr, &segments[1..], value);
                    }
                }
                PathSegment::Key(_) => {
                    if !matches!(&entries[idx].1, DynValue::Object(_)) {
                        entries[idx].1 = DynValue::Object(Vec::new());
                    }
                    if let DynValue::Object(ref mut obj) = entries[idx].1 {
                        flat_set_segments(obj, &segments[1..], value);
                    }
                }
            }
        }
        PathSegment::Index(_) => {
            // Shouldn't happen at root level in an Object context — ignore.
        }
    }
}

fn flat_set_in_array(arr: &mut Vec<DynValue>, segments: &[PathSegment], value: DynValue) {
    if segments.is_empty() {
        return;
    }
    if let PathSegment::Index(idx) = &segments[0] {
        // Extend array if needed
        while arr.len() <= *idx {
            arr.push(DynValue::Null);
        }
        if segments.len() == 1 {
            arr[*idx] = value;
        } else {
            // Need to descend further
            match &segments[1] {
                PathSegment::Key(_) => {
                    if !matches!(&arr[*idx], DynValue::Object(_)) {
                        arr[*idx] = DynValue::Object(Vec::new());
                    }
                    if let DynValue::Object(ref mut obj) = arr[*idx] {
                        flat_set_segments(obj, &segments[1..], value);
                    }
                }
                PathSegment::Index(_) => {
                    if !matches!(&arr[*idx], DynValue::Array(_)) {
                        arr[*idx] = DynValue::Array(Vec::new());
                    }
                    if let DynValue::Array(ref mut inner) = arr[*idx] {
                        flat_set_in_array(inner, &segments[1..], value);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// YAML Parser (simple indentation-based)
// ---------------------------------------------------------------------------

/// Parse simple YAML text into a `DynValue`.
///
/// Supports:
/// - `key: value` mappings
/// - `-` array items
/// - Indentation-based nesting (2-space or consistent)
/// - `#` comments
/// - Quoted strings (single and double)
/// - Value type inference
pub fn parse_yaml(input: &str) -> Result<DynValue, String> {
    let lines = yaml_preprocess(input);
    if lines.is_empty() {
        return Ok(DynValue::Object(Vec::new()));
    }
    let mut pos = 0;
    yaml_parse_block(&lines, &mut pos, 0)
}

#[derive(Debug)]
struct YamlLine {
    indent: usize,
    content: String,
}

/// Preprocess YAML: strip comments, skip blank lines, calculate indents.
fn yaml_preprocess(input: &str) -> Vec<YamlLine> {
    let mut lines = Vec::new();
    for raw_line in input.lines() {
        // Strip inline comments (but not inside quotes)
        let content = yaml_strip_comment(raw_line);
        let trimmed = content.trim_end();
        if trimmed.trim_start().is_empty() {
            continue;
        }
        let indent = trimmed.len() - trimmed.trim_start().len();
        lines.push(YamlLine {
            indent,
            content: trimmed.trim_start().to_string(),
        });
    }
    lines
}

fn yaml_strip_comment(line: &str) -> String {
    let mut result = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let chars = line.chars().peekable();
    for ch in chars {
        if ch == '\'' && !in_double {
            in_single = !in_single;
            result.push(ch);
        } else if ch == '"' && !in_single {
            in_double = !in_double;
            result.push(ch);
        } else if ch == '#' && !in_single && !in_double {
            // Check if preceded by whitespace or at start of remaining
            // The # must be preceded by a space to be a comment (or at start of line)
            if result.is_empty() || result.ends_with(' ') || result.ends_with('\t') {
                break;
            }
            result.push(ch);
        } else {
            result.push(ch);
        }
    }
    result
}

/// Parse a block of YAML lines at a given indentation level.
fn yaml_parse_block(lines: &[YamlLine], pos: &mut usize, base_indent: usize) -> Result<DynValue, String> {
    if *pos >= lines.len() {
        return Ok(DynValue::Object(Vec::new()));
    }

    // Determine if this block is an array or a mapping
    let first = &lines[*pos];
    if first.content.starts_with("- ") || first.content == "-" {
        yaml_parse_array(lines, pos, base_indent)
    } else {
        yaml_parse_mapping(lines, pos, base_indent)
    }
}

fn yaml_parse_mapping(lines: &[YamlLine], pos: &mut usize, base_indent: usize) -> Result<DynValue, String> {
    let mut entries: Vec<(String, DynValue)> = Vec::new();

    while *pos < lines.len() {
        let line = &lines[*pos];
        if line.indent < base_indent {
            break;
        }
        if line.indent > base_indent {
            // This shouldn't happen at the mapping level; skip it.
            break;
        }

        // Parse key: value
        let content = &line.content;
        if content.starts_with("- ") || content == "-" {
            // Switched to array mode within the mapping – shouldn't happen at same indent.
            break;
        }

        if let Some(colon_pos) = yaml_find_colon(content) {
            let key = content[..colon_pos].trim().to_string();
            let after_colon = content[colon_pos + 1..].trim();

            if after_colon.is_empty() {
                // Value is a nested block on subsequent lines
                *pos += 1;
                if *pos < lines.len() && lines[*pos].indent > base_indent {
                    let child_indent = lines[*pos].indent;
                    let child = yaml_parse_block(lines, pos, child_indent)?;
                    entries.push((key, child));
                } else {
                    entries.push((key, DynValue::Null));
                }
            } else {
                // Inline value
                entries.push((key, yaml_parse_scalar(after_colon)));
                *pos += 1;
            }
        } else {
            // No colon found – skip this line
            *pos += 1;
        }
    }

    Ok(DynValue::Object(entries))
}

fn yaml_parse_array(lines: &[YamlLine], pos: &mut usize, base_indent: usize) -> Result<DynValue, String> {
    let mut items: Vec<DynValue> = Vec::new();

    while *pos < lines.len() {
        let line = &lines[*pos];
        if line.indent < base_indent {
            break;
        }
        if line.indent > base_indent {
            break;
        }

        let content = &line.content;
        if !content.starts_with("- ") && content != "-" {
            break;
        }

        let after_dash = if content == "-" {
            ""
        } else {
            content[2..].trim()
        };

        if after_dash.is_empty() {
            // Nested block after bare `-`
            *pos += 1;
            if *pos < lines.len() && lines[*pos].indent > base_indent {
                let child_indent = lines[*pos].indent;
                let child = yaml_parse_block(lines, pos, child_indent)?;
                items.push(child);
            } else {
                items.push(DynValue::Null);
            }
        } else if let Some(colon_pos) = yaml_find_colon(after_dash) {
            // Inline mapping item, e.g. `- key: value`
            // Treat the rest as a single-entry mapping, then check subsequent
            // indented lines for more keys in this mapping.
            let key = after_dash[..colon_pos].trim().to_string();
            let val_str = after_dash[colon_pos + 1..].trim();

            let mut obj_entries: Vec<(String, DynValue)> = Vec::new();

            if val_str.is_empty() {
                *pos += 1;
                if *pos < lines.len() && lines[*pos].indent > base_indent {
                    let child_indent = lines[*pos].indent;
                    let child = yaml_parse_block(lines, pos, child_indent)?;
                    obj_entries.push((key, child));
                } else {
                    obj_entries.push((key, DynValue::Null));
                }
            } else {
                obj_entries.push((key, yaml_parse_scalar(val_str)));
                *pos += 1;
            }

            // Consume any continuation lines at greater indent that form more
            // key-value pairs for this same mapping item.
            // The continuation indent is base_indent + 2 (typical for `- key:` style).
            let continuation_indent = base_indent + 2;
            while *pos < lines.len() && lines[*pos].indent >= continuation_indent {
                let cont = &lines[*pos];
                if cont.indent > continuation_indent {
                    // belongs to a deeper nesting – let the recursive call handle it
                    break;
                }
                if let Some(cp) = yaml_find_colon(&cont.content) {
                    let ck = cont.content[..cp].trim().to_string();
                    let cv = cont.content[cp + 1..].trim();
                    if cv.is_empty() {
                        *pos += 1;
                        if *pos < lines.len() && lines[*pos].indent > continuation_indent {
                            let ci = lines[*pos].indent;
                            let child = yaml_parse_block(lines, pos, ci)?;
                            obj_entries.push((ck, child));
                        } else {
                            obj_entries.push((ck, DynValue::Null));
                        }
                    } else {
                        obj_entries.push((ck, yaml_parse_scalar(cv)));
                        *pos += 1;
                    }
                } else {
                    *pos += 1;
                }
            }

            items.push(DynValue::Object(obj_entries));
        } else {
            // Simple scalar array item
            items.push(yaml_parse_scalar(after_dash));
            *pos += 1;
        }
    }

    Ok(DynValue::Array(items))
}

/// Find the first `:` that acts as a key-value separator (followed by space or end).
fn yaml_find_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\'' && !in_double {
            in_single = !in_single;
        } else if b == b'"' && !in_single {
            in_double = !in_double;
        } else if b == b':' && !in_single && !in_double {
            // Must be followed by space, end of string, or nothing
            if i + 1 >= bytes.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\t' {
                return Some(i);
            }
        }
    }
    None
}

/// Parse a YAML scalar value (handles quoting and type inference).
fn yaml_parse_scalar(s: &str) -> DynValue {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return DynValue::Null;
    }

    // Quoted strings
    if ((trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
        && trimmed.len() >= 2 {
            return DynValue::String(trimmed[1..trimmed.len() - 1].to_string());
        }

    // Null
    if trimmed == "null" || trimmed == "~" {
        return DynValue::Null;
    }

    // Booleans
    if trimmed.eq_ignore_ascii_case("true") || trimmed == "yes" || trimmed == "on" {
        return DynValue::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") || trimmed == "no" || trimmed == "off" {
        return DynValue::Bool(false);
    }

    // Integer
    if let Ok(i) = trimmed.parse::<i64>() {
        return DynValue::Integer(i);
    }

    // Float
    if let Ok(f) = trimmed.parse::<f64>() {
        if trimmed.contains('.') || trimmed.contains('e') || trimmed.contains('E') {
            return DynValue::Float(f);
        }
    }

    DynValue::String(trimmed.to_string())
}

// ---------------------------------------------------------------------------
// Format Dispatcher
// ---------------------------------------------------------------------------

/// Dispatch to the appropriate parser by format name.
///
/// Recognized formats: `"json"`, `"xml"`, `"csv"`, `"fixed-width"`, `"flat"`,
/// `"properties"`, `"yaml"`.
///
/// CSV defaults to comma delimiter with headers.
/// Fixed-width requires field definitions and must be called directly via
/// `parse_fixed_width`; calling through this dispatcher returns an error
/// because field definitions cannot be inferred from the format string alone.
pub fn parse_source(input: &str, format: &str) -> Result<DynValue, String> {
    match format {
        "json" => parse_json(input),
        "xml" => parse_xml(input),
        "csv" => parse_csv(input, ',', true),
        "flat" | "properties" => parse_flat(input),
        "yaml" => parse_yaml(input),
        "fixed-width" => Err("fixed-width format requires field definitions; use parse_fixed_width directly".to_string()),
        _ => Err(format!("unknown source format: {format}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_primitives() {
        assert_eq!(parse_json("null").unwrap(), DynValue::Null);
        assert_eq!(parse_json("true").unwrap(), DynValue::Bool(true));
        assert_eq!(parse_json("42").unwrap(), DynValue::Integer(42));
        assert_eq!(parse_json("3.14").unwrap(), DynValue::Float(3.14));
        assert_eq!(parse_json("\"hello\"").unwrap(), DynValue::String("hello".to_string()));
    }

    #[test]
    fn test_parse_json_object() {
        let result = parse_json(r#"{"name": "Alice", "age": 30}"#).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].0, "name");
            assert_eq!(entries[1].0, "age");
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_parse_json_array() {
        let result = parse_json("[1, 2, 3]").unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 3);
        } else {
            panic!("expected array");
        }
    }

    // -----------------------------------------------------------------------
    // CSV tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_csv_with_header() {
        let input = "name,age,active\nAlice,30,true\nBob,25,false\n";
        let result = parse_csv(input, ',', true).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 2);
            if let DynValue::Object(entries) = &rows[0] {
                assert_eq!(entries[0], ("name".to_string(), DynValue::String("Alice".to_string())));
                assert_eq!(entries[1], ("age".to_string(), DynValue::Integer(30)));
                assert_eq!(entries[2], ("active".to_string(), DynValue::Bool(true)));
            } else {
                panic!("expected object row");
            }
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_csv_without_header() {
        let input = "Alice,30\nBob,25\n";
        let result = parse_csv(input, ',', false).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 2);
            if let DynValue::Array(cols) = &rows[0] {
                assert_eq!(cols[0], DynValue::String("Alice".to_string()));
                assert_eq!(cols[1], DynValue::Integer(30));
            } else {
                panic!("expected array row");
            }
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_csv_quoted_fields() {
        let input = "name,bio\nAlice,\"She said \"\"hello\"\"\"\n";
        let result = parse_csv(input, ',', true).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 1);
            if let DynValue::Object(entries) = &rows[0] {
                assert_eq!(entries[1].1, DynValue::String("She said \"hello\"".to_string()));
            } else {
                panic!("expected object");
            }
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_csv_newline_in_quoted_field() {
        let input = "name,note\nAlice,\"line1\nline2\"\n";
        let result = parse_csv(input, ',', true).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 1);
            if let DynValue::Object(entries) = &rows[0] {
                assert_eq!(entries[1].1, DynValue::String("line1\nline2".to_string()));
            } else {
                panic!("expected object");
            }
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_csv_tab_delimiter() {
        let input = "a\tb\n1\t2\n";
        let result = parse_csv(input, '\t', true).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 1);
            if let DynValue::Object(entries) = &rows[0] {
                assert_eq!(entries[0], ("a".to_string(), DynValue::Integer(1)));
            } else {
                panic!("expected object");
            }
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_csv_empty() {
        let result = parse_csv("", ',', true).unwrap();
        assert_eq!(result, DynValue::Array(Vec::new()));
    }

    #[test]
    fn test_csv_type_inference() {
        let input = "val\n42\n3.14\ntrue\nfalse\nhello\n";
        let result = parse_csv(input, ',', true).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 5);
            // 42 -> Integer
            if let DynValue::Object(e) = &rows[0] {
                assert_eq!(e[0].1, DynValue::Integer(42));
            }
            // 3.14 -> Float
            if let DynValue::Object(e) = &rows[1] {
                assert_eq!(e[0].1, DynValue::Float(3.14));
            }
            // true -> Bool
            if let DynValue::Object(e) = &rows[2] {
                assert_eq!(e[0].1, DynValue::Bool(true));
            }
            // false -> Bool
            if let DynValue::Object(e) = &rows[3] {
                assert_eq!(e[0].1, DynValue::Bool(false));
            }
            // hello -> String
            if let DynValue::Object(e) = &rows[4] {
                assert_eq!(e[0].1, DynValue::String("hello".to_string()));
            }
        } else {
            panic!("expected array");
        }
    }

    // -----------------------------------------------------------------------
    // XML tests
    // -----------------------------------------------------------------------

    // Helper to unwrap the root element from parse_xml result.
    // parse_xml wraps the root element: `<root>...</root>` → Object([("root", ...)])
    fn unwrap_root(val: DynValue) -> DynValue {
        if let DynValue::Object(entries) = val {
            if entries.len() == 1 {
                return entries.into_iter().next().unwrap().1;
            }
            DynValue::Object(entries)
        } else {
            val
        }
    }

    #[test]
    fn test_xml_simple_element() {
        let input = "<name>Alice</name>";
        let result = unwrap_root(parse_xml(input).unwrap());
        assert_eq!(result, DynValue::String("Alice".to_string()));
    }

    #[test]
    fn test_xml_nested_elements() {
        let input = "<person><name>Alice</name><age>30</age></person>";
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0], ("name".to_string(), DynValue::String("Alice".to_string())));
            // XML text is always parsed as String; type coercion happens via transform directives
            assert_eq!(entries[1], ("age".to_string(), DynValue::String("30".to_string())));
        } else {
            panic!("expected object, got {:?}", result);
        }
    }

    #[test]
    fn test_xml_attributes() {
        let input = r#"<person id="1"><name>Alice</name></person>"#;
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            assert!(entries.iter().any(|(k, v)| k == "@id" && *v == DynValue::String("1".to_string())));
            assert!(entries.iter().any(|(k, v)| k == "name" && *v == DynValue::String("Alice".to_string())));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_xml_repeated_elements() {
        let input = "<list><item>a</item><item>b</item><item>c</item></list>";
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].0, "item");
            if let DynValue::Array(items) = &entries[0].1 {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], DynValue::String("a".to_string()));
            } else {
                panic!("expected array for repeated elements");
            }
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_xml_self_closing() {
        let input = r#"<root><empty/></root>"#;
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("empty".to_string(), DynValue::Null));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_xml_self_closing_with_attrs() {
        let input = r#"<root><img src="a.png"/></root>"#;
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            if let DynValue::Object(img) = &entries[0].1 {
                assert_eq!(img[0], ("@src".to_string(), DynValue::String("a.png".to_string())));
            } else {
                panic!("expected object for img");
            }
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_xml_cdata() {
        let input = "<data><![CDATA[Hello <world> & friends]]></data>";
        let result = unwrap_root(parse_xml(input).unwrap());
        assert_eq!(result, DynValue::String("Hello <world> & friends".to_string()));
    }

    #[test]
    fn test_xml_entities() {
        let input = "<msg>a &amp; b &lt; c &gt; d &quot;e&quot; &apos;f&apos;</msg>";
        let result = unwrap_root(parse_xml(input).unwrap());
        assert_eq!(result, DynValue::String("a & b < c > d \"e\" 'f'".to_string()));
    }

    #[test]
    fn test_xml_declaration_skipped() {
        let input = r#"<?xml version="1.0" encoding="UTF-8"?><root><x>1</x></root>"#;
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("x".to_string(), DynValue::String("1".to_string())));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_xml_comment_skipped() {
        let input = "<root><!-- comment --><x>1</x></root>";
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("x".to_string(), DynValue::String("1".to_string())));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_xml_mixed_content() {
        let input = "<p>Hello <b>world</b></p>";
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            assert!(entries.iter().any(|(k, _)| k == "_text"));
            assert!(entries.iter().any(|(k, _)| k == "b"));
        } else {
            panic!("expected object for mixed content");
        }
    }

    // -----------------------------------------------------------------------
    // Fixed-width tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_fixed_width_single_line() {
        let input = "Alice     30  ";
        let fields = vec![
            ("name".to_string(), 0, 10),
            ("age".to_string(), 10, 4),
        ];
        let result = parse_fixed_width(input, &fields).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("name".to_string(), DynValue::String("Alice".to_string())));
            assert_eq!(entries[1], ("age".to_string(), DynValue::String("30".to_string())));
        } else {
            panic!("expected object for single line");
        }
    }

    #[test]
    fn test_fixed_width_multiple_lines() {
        let input = "Alice     30\nBob       25\n";
        let fields = vec![
            ("name".to_string(), 0, 10),
            ("age".to_string(), 10, 2),
        ];
        let result = parse_fixed_width(input, &fields).unwrap();
        if let DynValue::Array(records) = result {
            assert_eq!(records.len(), 2);
            if let DynValue::Object(e) = &records[0] {
                assert_eq!(e[0].1, DynValue::String("Alice".to_string()));
            }
            if let DynValue::Object(e) = &records[1] {
                assert_eq!(e[0].1, DynValue::String("Bob".to_string()));
            }
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_fixed_width_empty() {
        let result = parse_fixed_width("", &[]).unwrap();
        assert_eq!(result, DynValue::Array(Vec::new()));
    }

    #[test]
    fn test_fixed_width_short_line() {
        // Line is shorter than field spec
        let input = "Hi";
        let fields = vec![
            ("short".to_string(), 0, 2),
            ("missing".to_string(), 10, 5),
        ];
        let result = parse_fixed_width(input, &fields).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].1, DynValue::String("Hi".to_string()));
            assert_eq!(entries[1].1, DynValue::String("".to_string()));
        } else {
            panic!("expected object");
        }
    }

    // -----------------------------------------------------------------------
    // Flat (key=value) tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_flat_simple() {
        let input = "name = Alice\nage = 30\nactive = true\n";
        let result = parse_flat(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("name".to_string(), DynValue::String("Alice".to_string())));
            assert_eq!(entries[1], ("age".to_string(), DynValue::Integer(30)));
            assert_eq!(entries[2], ("active".to_string(), DynValue::Bool(true)));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_flat_comments_and_blanks() {
        let input = "# This is a comment\n; Another comment\n\nname = Alice\n";
        let result = parse_flat(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].0, "name");
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_flat_dot_notation() {
        let input = "a.b.c = hello\na.b.d = 42\na.e = true\n";
        let result = parse_flat(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].0, "a");
            if let DynValue::Object(a) = &entries[0].1 {
                assert_eq!(a[0].0, "b");
                if let DynValue::Object(b) = &a[0].1 {
                    assert_eq!(b[0], ("c".to_string(), DynValue::String("hello".to_string())));
                    assert_eq!(b[1], ("d".to_string(), DynValue::Integer(42)));
                } else {
                    panic!("expected nested object for b");
                }
                assert_eq!(a[1], ("e".to_string(), DynValue::Bool(true)));
            } else {
                panic!("expected nested object for a");
            }
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_flat_array_notation() {
        let input = "items[0].name = foo\nitems[0].value = 1\nitems[1].name = bar\nitems[1].value = 2\n";
        let result = parse_flat(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].0, "items");
            if let DynValue::Array(arr) = &entries[0].1 {
                assert_eq!(arr.len(), 2);
                if let DynValue::Object(item0) = &arr[0] {
                    assert_eq!(item0[0], ("name".to_string(), DynValue::String("foo".to_string())));
                    assert_eq!(item0[1], ("value".to_string(), DynValue::Integer(1)));
                } else {
                    panic!("expected object for item[0]");
                }
            } else {
                panic!("expected array for items");
            }
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_flat_type_inference() {
        let input = "a = true\nb = false\nc = 42\nd = 3.14\ne = hello\n";
        let result = parse_flat(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].1, DynValue::Bool(true));
            assert_eq!(entries[1].1, DynValue::Bool(false));
            assert_eq!(entries[2].1, DynValue::Integer(42));
            assert_eq!(entries[3].1, DynValue::Float(3.14));
            assert_eq!(entries[4].1, DynValue::String("hello".to_string()));
        } else {
            panic!("expected object");
        }
    }

    // -----------------------------------------------------------------------
    // YAML tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_yaml_simple_mapping() {
        let input = "name: Alice\nage: 30\nactive: true\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("name".to_string(), DynValue::String("Alice".to_string())));
            assert_eq!(entries[1], ("age".to_string(), DynValue::Integer(30)));
            assert_eq!(entries[2], ("active".to_string(), DynValue::Bool(true)));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_yaml_nested_mapping() {
        let input = "person:\n  name: Alice\n  age: 30\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].0, "person");
            if let DynValue::Object(person) = &entries[0].1 {
                assert_eq!(person[0], ("name".to_string(), DynValue::String("Alice".to_string())));
                assert_eq!(person[1], ("age".to_string(), DynValue::Integer(30)));
            } else {
                panic!("expected nested object");
            }
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_yaml_simple_array() {
        let input = "items:\n  - apple\n  - banana\n  - cherry\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].0, "items");
            if let DynValue::Array(items) = &entries[0].1 {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], DynValue::String("apple".to_string()));
                assert_eq!(items[1], DynValue::String("banana".to_string()));
                assert_eq!(items[2], DynValue::String("cherry".to_string()));
            } else {
                panic!("expected array");
            }
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_yaml_array_of_objects() {
        let input = "people:\n  - name: Alice\n    age: 30\n  - name: Bob\n    age: 25\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(entries) = result {
            if let DynValue::Array(people) = &entries[0].1 {
                assert_eq!(people.len(), 2);
                if let DynValue::Object(p0) = &people[0] {
                    assert_eq!(p0[0], ("name".to_string(), DynValue::String("Alice".to_string())));
                    assert_eq!(p0[1], ("age".to_string(), DynValue::Integer(30)));
                } else {
                    panic!("expected object for person 0");
                }
            } else {
                panic!("expected array for people");
            }
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_yaml_comments() {
        let input = "# This is a comment\nname: Alice # inline comment\nage: 30\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("name".to_string(), DynValue::String("Alice".to_string())));
            assert_eq!(entries[1], ("age".to_string(), DynValue::Integer(30)));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_yaml_quoted_strings() {
        let input = "single: 'hello world'\ndouble: \"goodbye world\"\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("single".to_string(), DynValue::String("hello world".to_string())));
            assert_eq!(entries[1], ("double".to_string(), DynValue::String("goodbye world".to_string())));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_yaml_null_and_tilde() {
        let input = "a: null\nb: ~\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].1, DynValue::Null);
            assert_eq!(entries[1].1, DynValue::Null);
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_yaml_type_inference() {
        let input = "i: 42\nf: 3.14\ntrue_val: true\nfalse_val: false\nstr: hello\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].1, DynValue::Integer(42));
            assert_eq!(entries[1].1, DynValue::Float(3.14));
            assert_eq!(entries[2].1, DynValue::Bool(true));
            assert_eq!(entries[3].1, DynValue::Bool(false));
            assert_eq!(entries[4].1, DynValue::String("hello".to_string()));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_yaml_empty() {
        let result = parse_yaml("").unwrap();
        assert_eq!(result, DynValue::Object(Vec::new()));
    }

    // -----------------------------------------------------------------------
    // parse_source dispatcher tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_source_json() {
        let result = parse_source(r#"{"a": 1}"#, "json").unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("a".to_string(), DynValue::Integer(1)));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_parse_source_csv() {
        let input = "x\n42\n";
        let result = parse_source(input, "csv").unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 1);
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn test_parse_source_xml() {
        let result = parse_source("<root><a>1</a></root>", "xml").unwrap();
        // parse_xml wraps in root object: Object([("root", Object([("a", String("1"))]))])
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].0, "root");
            if let DynValue::Object(inner) = &entries[0].1 {
                assert_eq!(inner[0], ("a".to_string(), DynValue::String("1".to_string())));
            } else {
                panic!("expected inner object");
            }
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_parse_source_flat() {
        let result = parse_source("key = value\n", "flat").unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("key".to_string(), DynValue::String("value".to_string())));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_parse_source_properties() {
        let result = parse_source("key = value\n", "properties").unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("key".to_string(), DynValue::String("value".to_string())));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_parse_source_yaml() {
        let result = parse_source("x: 1\n", "yaml").unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0], ("x".to_string(), DynValue::Integer(1)));
        } else {
            panic!("expected object");
        }
    }

    #[test]
    fn test_parse_source_fixed_width_error() {
        let result = parse_source("data", "fixed-width");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("field definitions"));
    }

    #[test]
    fn test_parse_source_unknown() {
        let result = parse_source("data", "protobuf");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown source format"));
    }
}

#[cfg(test)]
mod extended_tests {
    use super::*;

    // ===================================================================
    // JSON parsing extended tests
    // ===================================================================

    #[test]
    fn json_nested_object() {
        let input = r#"{"a": {"b": {"c": 42}}}"#;
        let result = parse_json(input).unwrap();
        if let DynValue::Object(a) = result {
            if let DynValue::Object(b) = &a[0].1 {
                if let DynValue::Object(c) = &b[0].1 {
                    assert_eq!(c[0].1, DynValue::Integer(42));
                } else { panic!("expected nested object c"); }
            } else { panic!("expected nested object b"); }
        } else { panic!("expected object"); }
    }

    #[test]
    fn json_array_of_objects() {
        let input = r#"[{"id": 1}, {"id": 2}, {"id": 3}]"#;
        let result = parse_json(input).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 3);
            for (i, item) in items.iter().enumerate() {
                if let DynValue::Object(e) = item {
                    assert_eq!(e[0].1, DynValue::Integer(i as i64 + 1));
                } else { panic!("expected object at index {i}"); }
            }
        } else { panic!("expected array"); }
    }

    #[test]
    fn json_null_values_in_object() {
        let result = parse_json(r#"{"a": null, "b": null}"#).unwrap();
        if let DynValue::Object(e) = result {
            assert_eq!(e[0].1, DynValue::Null);
            assert_eq!(e[1].1, DynValue::Null);
        } else { panic!("expected object"); }
    }

    #[test]
    fn json_boolean_values() {
        let result = parse_json(r#"{"t": true, "f": false}"#).unwrap();
        if let DynValue::Object(e) = result {
            assert_eq!(e[0].1, DynValue::Bool(true));
            assert_eq!(e[1].1, DynValue::Bool(false));
        } else { panic!("expected object"); }
    }

    #[test]
    fn json_negative_numbers() {
        let result = parse_json(r#"{"neg": -42, "negf": -3.14}"#).unwrap();
        if let DynValue::Object(e) = result {
            assert_eq!(e[0].1, DynValue::Integer(-42));
            assert_eq!(e[1].1, DynValue::Float(-3.14));
        } else { panic!("expected object"); }
    }

    #[test]
    fn json_string_with_escapes() {
        let input = r#"{"msg": "line1\nline2\ttab \"quoted\""}"#;
        let result = parse_json(input).unwrap();
        if let DynValue::Object(e) = result {
            assert_eq!(e[0].1, DynValue::String("line1\nline2\ttab \"quoted\"".to_string()));
        } else { panic!("expected object"); }
    }

    #[test]
    fn json_unicode_string() {
        let input = r#"{"emoji": "\u0048\u0065\u006C\u006C\u006F"}"#;
        let result = parse_json(input).unwrap();
        if let DynValue::Object(e) = result {
            assert_eq!(e[0].1, DynValue::String("Hello".to_string()));
        } else { panic!("expected object"); }
    }

    #[test]
    fn json_deeply_nested() {
        let input = r#"{"l1": {"l2": {"l3": {"l4": {"l5": "deep"}}}}}"#;
        let result = parse_json(input).unwrap();
        // Navigate 5 levels deep
        let mut current = result;
        for _ in 0..5 {
            match current {
                DynValue::Object(e) => current = e.into_iter().next().unwrap().1,
                _ => panic!("expected object at each level"),
            }
        }
        assert_eq!(current, DynValue::String("deep".to_string()));
    }

    #[test]
    fn json_empty_object() {
        let result = parse_json("{}").unwrap();
        assert_eq!(result, DynValue::Object(Vec::new()));
    }

    #[test]
    fn json_empty_array() {
        let result = parse_json("[]").unwrap();
        assert_eq!(result, DynValue::Array(Vec::new()));
    }

    #[test]
    fn json_mixed_array() {
        let input = r#"[1, "two", true, null, 3.14]"#;
        let result = parse_json(input).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 5);
            assert_eq!(items[0], DynValue::Integer(1));
            assert_eq!(items[1], DynValue::String("two".to_string()));
            assert_eq!(items[2], DynValue::Bool(true));
            assert_eq!(items[3], DynValue::Null);
            assert_eq!(items[4], DynValue::Float(3.14));
        } else { panic!("expected array"); }
    }

    #[test]
    fn json_large_integer() {
        let result = parse_json("9007199254740992").unwrap();
        assert_eq!(result, DynValue::Integer(9007199254740992));
    }

    #[test]
    fn json_zero_values() {
        let result = parse_json(r#"{"i": 0, "f": 0.0}"#).unwrap();
        if let DynValue::Object(e) = result {
            assert_eq!(e[0].1, DynValue::Integer(0));
            // 0.0 may come through as Float(0.0) or Integer(0) depending on serde
            match &e[1].1 {
                DynValue::Float(f) => assert_eq!(*f, 0.0),
                DynValue::Integer(i) => assert_eq!(*i, 0),
                other => panic!("unexpected: {:?}", other),
            }
        } else { panic!("expected object"); }
    }

    #[test]
    fn json_malformed_error() {
        assert!(parse_json("{invalid}").is_err());
        assert!(parse_json("").is_err());
        assert!(parse_json("{\"a\": }").is_err());
        assert!(parse_json("[1, 2,]").is_err());
    }

    #[test]
    fn json_nested_arrays() {
        let input = "[[1, 2], [3, 4]]";
        let result = parse_json(input).unwrap();
        if let DynValue::Array(outer) = result {
            assert_eq!(outer.len(), 2);
            if let DynValue::Array(inner) = &outer[0] {
                assert_eq!(inner[0], DynValue::Integer(1));
                assert_eq!(inner[1], DynValue::Integer(2));
            } else { panic!("expected inner array"); }
        } else { panic!("expected array"); }
    }

    // ===================================================================
    // CSV parsing extended tests
    // ===================================================================

    #[test]
    fn csv_single_column() {
        let input = "name\nAlice\nBob\n";
        let result = parse_csv(input, ',', true).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 2);
            if let DynValue::Object(e) = &rows[0] {
                assert_eq!(e[0], ("name".to_string(), DynValue::String("Alice".to_string())));
            } else { panic!("expected object"); }
        } else { panic!("expected array"); }
    }

    #[test]
    fn csv_many_rows() {
        let mut input = String::from("id,val\n");
        for i in 0..100 {
            input.push_str(&format!("{i},data{i}\n"));
        }
        let result = parse_csv(&input, ',', true).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 100);
        } else { panic!("expected array"); }
    }

    #[test]
    fn csv_empty_fields() {
        let input = "a,b,c\n1,,3\n";
        let result = parse_csv(input, ',', true).unwrap();
        if let DynValue::Array(rows) = result {
            if let DynValue::Object(e) = &rows[0] {
                assert_eq!(e[0].1, DynValue::Integer(1));
                assert_eq!(e[1].1, DynValue::String(String::new()));
                assert_eq!(e[2].1, DynValue::Integer(3));
            } else { panic!("expected object"); }
        } else { panic!("expected array"); }
    }

    #[test]
    fn csv_semicolon_delimiter() {
        let input = "a;b\n1;2\n";
        let result = parse_csv(input, ';', true).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 1);
            if let DynValue::Object(e) = &rows[0] {
                assert_eq!(e[0], ("a".to_string(), DynValue::Integer(1)));
                assert_eq!(e[1], ("b".to_string(), DynValue::Integer(2)));
            } else { panic!("expected object"); }
        } else { panic!("expected array"); }
    }

    #[test]
    fn csv_pipe_delimiter() {
        let input = "x|y\nhello|world\n";
        let result = parse_csv(input, '|', true).unwrap();
        if let DynValue::Array(rows) = result {
            if let DynValue::Object(e) = &rows[0] {
                assert_eq!(e[0].1, DynValue::String("hello".to_string()));
                assert_eq!(e[1].1, DynValue::String("world".to_string()));
            } else { panic!("expected object"); }
        } else { panic!("expected array"); }
    }

    #[test]
    fn csv_no_header_multiple_columns() {
        let input = "a,1,true\nb,2,false\n";
        let result = parse_csv(input, ',', false).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 2);
            if let DynValue::Array(cols) = &rows[0] {
                assert_eq!(cols[0], DynValue::String("a".to_string()));
                assert_eq!(cols[1], DynValue::Integer(1));
                assert_eq!(cols[2], DynValue::Bool(true));
            } else { panic!("expected array row"); }
        } else { panic!("expected array"); }
    }

    #[test]
    fn csv_header_only_no_data() {
        let input = "name,age\n";
        let result = parse_csv(input, ',', true).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 0);
        } else { panic!("expected array"); }
    }

    #[test]
    fn csv_null_inference() {
        let input = "val\nnull\nNULL\n";
        let result = parse_csv(input, ',', true).unwrap();
        if let DynValue::Array(rows) = result {
            if let DynValue::Object(e) = &rows[0] {
                assert_eq!(e[0].1, DynValue::Null);
            } else { panic!("expected object"); }
            if let DynValue::Object(e) = &rows[1] {
                assert_eq!(e[0].1, DynValue::Null);
            } else { panic!("expected object"); }
        } else { panic!("expected array"); }
    }

    #[test]
    fn csv_crlf_line_endings() {
        let input = "a,b\r\n1,2\r\n3,4\r\n";
        let result = parse_csv(input, ',', true).unwrap();
        if let DynValue::Array(rows) = result {
            assert_eq!(rows.len(), 2);
        } else { panic!("expected array"); }
    }

    #[test]
    fn csv_unterminated_quote_error() {
        let input = "a\n\"unterminated\n";
        let result = parse_csv(input, ',', true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unterminated"));
    }

    // ===================================================================
    // XML parsing extended tests
    // ===================================================================

    fn unwrap_root(val: DynValue) -> DynValue {
        if let DynValue::Object(entries) = val {
            if entries.len() == 1 {
                return entries.into_iter().next().unwrap().1;
            }
            DynValue::Object(entries)
        } else {
            val
        }
    }

    #[test]
    fn xml_multiple_attributes() {
        let input = r#"<item id="1" type="product" active="true"><name>Widget</name></item>"#;
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            assert!(entries.iter().any(|(k, _)| k == "@id"));
            assert!(entries.iter().any(|(k, _)| k == "@type"));
            assert!(entries.iter().any(|(k, _)| k == "@active"));
            assert!(entries.iter().any(|(k, _)| k == "name"));
        } else { panic!("expected object"); }
    }

    #[test]
    fn xml_deeply_nested_elements() {
        let input = "<a><b><c><d><e>deep</e></d></c></b></a>";
        let result = unwrap_root(parse_xml(input).unwrap());
        // Navigate through layers
        let mut current = result;
        for _ in 0..4 {
            if let DynValue::Object(e) = current {
                current = e.into_iter().next().unwrap().1;
            } else { panic!("expected nested object"); }
        }
        assert_eq!(current, DynValue::String("deep".to_string()));
    }

    #[test]
    fn xml_empty_root() {
        let input = "<root></root>";
        let result = unwrap_root(parse_xml(input).unwrap());
        assert_eq!(result, DynValue::String(String::new()));
    }

    #[test]
    fn xml_namespace_prefix_filtered() {
        let input = r#"<root xmlns:ns="http://example.com"><ns:item>val</ns:item></root>"#;
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            // Namespace attributes should be filtered, but ns:item stays as element name
            assert!(entries.iter().any(|(k, _)| k == "ns:item"));
        } else { panic!("expected object"); }
    }

    #[test]
    fn xml_xsi_nil_element() {
        let input = r#"<root><value xsi:nil="true"/></root>"#;
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].1, DynValue::Null);
        } else { panic!("expected object"); }
    }

    #[test]
    fn xml_multiple_repeated_elements() {
        let input = "<root><a>1</a><a>2</a><b>x</b><b>y</b></root>";
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            let a_entry = entries.iter().find(|(k, _)| k == "a").unwrap();
            if let DynValue::Array(items) = &a_entry.1 {
                assert_eq!(items.len(), 2);
            } else { panic!("expected array for repeated a"); }
            let b_entry = entries.iter().find(|(k, _)| k == "b").unwrap();
            if let DynValue::Array(items) = &b_entry.1 {
                assert_eq!(items.len(), 2);
            } else { panic!("expected array for repeated b"); }
        } else { panic!("expected object"); }
    }

    #[test]
    fn xml_mixed_content_with_text() {
        let input = "<p>Start <em>middle</em> end</p>";
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            assert!(entries.iter().any(|(k, _)| k == "_text"));
            assert!(entries.iter().any(|(k, _)| k == "em"));
        } else { panic!("expected object with mixed content"); }
    }

    #[test]
    fn xml_self_closing_multiple() {
        let input = r#"<root><br/><hr/></root>"#;
        let result = unwrap_root(parse_xml(input).unwrap());
        if let DynValue::Object(entries) = result {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].1, DynValue::Null);
            assert_eq!(entries[1].1, DynValue::Null);
        } else { panic!("expected object"); }
    }

    #[test]
    fn xml_empty_document_error() {
        assert!(parse_xml("").is_err());
    }

    #[test]
    fn xml_malformed_error() {
        assert!(parse_xml("<root><unclosed>").is_err());
    }

    #[test]
    fn xml_special_chars_in_text() {
        let input = "<msg>Price: 5 &lt; 10 &amp; 3 &gt; 1</msg>";
        let result = unwrap_root(parse_xml(input).unwrap());
        assert_eq!(result, DynValue::String("Price: 5 < 10 & 3 > 1".to_string()));
    }

    // ===================================================================
    // Fixed-width parsing extended tests
    // ===================================================================

    #[test]
    fn fixed_width_various_widths() {
        let input = "AB1234CDEF56";
        let fields = vec![
            ("f2".to_string(), 0, 2),
            ("f4".to_string(), 2, 4),
            ("f4b".to_string(), 6, 4),
            ("f2b".to_string(), 10, 2),
        ];
        let result = parse_fixed_width(input, &fields).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].1, DynValue::String("AB".to_string()));
            assert_eq!(entries[1].1, DynValue::String("1234".to_string()));
            assert_eq!(entries[2].1, DynValue::String("CDEF".to_string()));
            assert_eq!(entries[3].1, DynValue::String("56".to_string()));
        } else { panic!("expected object"); }
    }

    #[test]
    fn fixed_width_trimming() {
        let input = "Hello     World     ";
        let fields = vec![
            ("a".to_string(), 0, 10),
            ("b".to_string(), 10, 10),
        ];
        let result = parse_fixed_width(input, &fields).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].1, DynValue::String("Hello".to_string()));
            assert_eq!(entries[1].1, DynValue::String("World".to_string()));
        } else { panic!("expected object"); }
    }

    #[test]
    fn fixed_width_field_beyond_line() {
        let input = "AB";
        let fields = vec![
            ("present".to_string(), 0, 2),
            ("absent".to_string(), 50, 10),
        ];
        let result = parse_fixed_width(input, &fields).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries[0].1, DynValue::String("AB".to_string()));
            assert_eq!(entries[1].1, DynValue::String(String::new()));
        } else { panic!("expected object"); }
    }

    #[test]
    fn fixed_width_blank_lines_skipped() {
        let input = "Alice     30\n\n  \nBob       25\n";
        let fields = vec![
            ("name".to_string(), 0, 10),
            ("age".to_string(), 10, 2),
        ];
        let result = parse_fixed_width(input, &fields).unwrap();
        if let DynValue::Array(records) = result {
            assert_eq!(records.len(), 2);
        } else { panic!("expected array"); }
    }

    // ===================================================================
    // YAML parsing extended tests
    // ===================================================================

    #[test]
    fn yaml_deeply_nested() {
        let input = "a:\n  b:\n    c:\n      d: deep\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(a) = result {
            if let DynValue::Object(b) = &a[0].1 {
                if let DynValue::Object(c) = &b[0].1 {
                    if let DynValue::Object(d) = &c[0].1 {
                        assert_eq!(d[0].1, DynValue::String("deep".to_string()));
                    } else { panic!("expected d"); }
                } else { panic!("expected c"); }
            } else { panic!("expected b"); }
        } else { panic!("expected object"); }
    }

    #[test]
    fn yaml_boolean_variants() {
        let input = "a: yes\nb: no\nc: on\nd: off\ne: true\nf: false\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(e) = result {
            assert_eq!(e[0].1, DynValue::Bool(true));
            assert_eq!(e[1].1, DynValue::Bool(false));
            assert_eq!(e[2].1, DynValue::Bool(true));
            assert_eq!(e[3].1, DynValue::Bool(false));
            assert_eq!(e[4].1, DynValue::Bool(true));
            assert_eq!(e[5].1, DynValue::Bool(false));
        } else { panic!("expected object"); }
    }

    #[test]
    fn yaml_array_of_scalars() {
        let input = "- 1\n- 2\n- 3\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], DynValue::Integer(1));
            assert_eq!(items[1], DynValue::Integer(2));
            assert_eq!(items[2], DynValue::Integer(3));
        } else { panic!("expected array"); }
    }

    #[test]
    fn yaml_mixed_types() {
        let input = "str: hello\nint: 42\nfloat: 3.14\nbool: true\nnull_val: null\ntilde: ~\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(e) = result {
            assert_eq!(e[0].1, DynValue::String("hello".to_string()));
            assert_eq!(e[1].1, DynValue::Integer(42));
            assert_eq!(e[2].1, DynValue::Float(3.14));
            assert_eq!(e[3].1, DynValue::Bool(true));
            assert_eq!(e[4].1, DynValue::Null);
            assert_eq!(e[5].1, DynValue::Null);
        } else { panic!("expected object"); }
    }

    #[test]
    fn yaml_comments_everywhere() {
        let input = "# top comment\nname: Alice # inline\n# middle\nage: 30\n";
        let result = parse_yaml(input).unwrap();
        if let DynValue::Object(e) = result {
            assert_eq!(e.len(), 2);
            assert_eq!(e[0].1, DynValue::String("Alice".to_string()));
            assert_eq!(e[1].1, DynValue::Integer(30));
        } else { panic!("expected object"); }
    }

    // ===================================================================
    // Flat (key=value) parsing extended tests
    // ===================================================================

    #[test]
    fn flat_null_values() {
        let input = "a = \nb = ~\n";
        let result = parse_flat(input).unwrap();
        if let DynValue::Object(e) = result {
            assert_eq!(e[0].1, DynValue::Null);
            assert_eq!(e[1].1, DynValue::Null);
        } else { panic!("expected object"); }
    }

    #[test]
    fn flat_quoted_values() {
        let input = "msg = \"hello = world\"\n";
        let result = parse_flat(input).unwrap();
        if let DynValue::Object(e) = result {
            assert_eq!(e[0].1, DynValue::String("hello = world".to_string()));
        } else { panic!("expected object"); }
    }

    #[test]
    fn flat_nested_arrays() {
        let input = "data[0][0] = a\ndata[0][1] = b\ndata[1][0] = c\n";
        let result = parse_flat(input).unwrap();
        if let DynValue::Object(top) = result {
            if let DynValue::Array(outer) = &top[0].1 {
                if let DynValue::Array(inner0) = &outer[0] {
                    assert_eq!(inner0[0], DynValue::String("a".to_string()));
                    assert_eq!(inner0[1], DynValue::String("b".to_string()));
                } else { panic!("expected inner array"); }
            } else { panic!("expected outer array"); }
        } else { panic!("expected object"); }
    }

    #[test]
    fn flat_empty_input() {
        let result = parse_flat("").unwrap();
        assert_eq!(result, DynValue::Object(Vec::new()));
    }

    #[test]
    fn flat_only_comments() {
        let result = parse_flat("# comment\n; another\n\n").unwrap();
        assert_eq!(result, DynValue::Object(Vec::new()));
    }

    // ===================================================================
    // Error/edge case tests
    // ===================================================================

    #[test]
    fn parse_source_dispatches_correctly() {
        // Verify each format is correctly dispatched
        assert!(parse_source("{}", "json").is_ok());
        assert!(parse_source("<r/>", "xml").is_ok());
        assert!(parse_source("a\n1\n", "csv").is_ok());
        assert!(parse_source("key = val\n", "flat").is_ok());
        assert!(parse_source("key = val\n", "properties").is_ok());
        assert!(parse_source("key: val\n", "yaml").is_ok());
    }
}
