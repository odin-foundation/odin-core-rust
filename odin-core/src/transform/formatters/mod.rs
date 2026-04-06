//! Output formatters for the transform engine.
//!
//! Converts transform output to target format strings:
//! - JSON
//! - XML
//! - CSV
//! - Fixed-width
//! - Flat (key=value)
//! - ODIN

use std::fmt::Write;

use crate::types::transform::DynValue;

/// Format transform output as JSON with format options.
///
/// Supported options:
/// - `indent`: indentation spaces (0 = compact/minified, default 2)
/// - `nulls`: "omit" to recursively remove null-valued keys
/// - `emptyArrays`: "omit" to recursively remove empty array properties
pub fn format_json_with_opts(value: &DynValue, options: &std::collections::HashMap<String, String>) -> String {
    let indent: usize = options.get("indent").and_then(|v| v.parse().ok()).unwrap_or(2);
    let omit_nulls = options.get("nulls").is_some_and(|v| v == "omit");
    let omit_empty_arrays = options.get("emptyArrays").is_some_and(|v| v == "omit");

    let value = if omit_nulls || omit_empty_arrays {
        strip_json_values(value, omit_nulls, omit_empty_arrays)
    } else {
        value.clone()
    };

    let mut output = String::new();
    if indent == 0 {
        write_json_value(&mut output, &value, usize::MAX, 0);
    } else {
        write_json_value(&mut output, &value, indent, 0);
    }
    output
}

/// Recursively strip null values and/or empty arrays from a DynValue tree.
fn strip_json_values(value: &DynValue, omit_nulls: bool, omit_empty_arrays: bool) -> DynValue {
    match value {
        DynValue::Object(entries) => {
            let filtered: Vec<(String, DynValue)> = entries.iter()
                .filter(|(_, v)| {
                    if omit_nulls && v.is_null() { return false; }
                    if omit_empty_arrays {
                        if let DynValue::Array(arr) = v {
                            if arr.is_empty() { return false; }
                        }
                    }
                    true
                })
                .map(|(k, v)| (k.clone(), strip_json_values(v, omit_nulls, omit_empty_arrays)))
                .collect();
            DynValue::Object(filtered)
        }
        DynValue::Array(items) => {
            DynValue::Array(items.iter().map(|v| strip_json_values(v, omit_nulls, omit_empty_arrays)).collect())
        }
        _ => value.clone(),
    }
}

/// Format transform output as JSON.
pub fn format_json(value: &DynValue, pretty: bool) -> String {
    let mut output = String::new();
    write_json_value(&mut output, value, if pretty { 2 } else { usize::MAX }, 0);
    output
}

fn write_json_value(output: &mut String, value: &DynValue, indent: usize, depth: usize) {
    let pretty = indent != usize::MAX;
    match value {
        DynValue::Null => output.push_str("null"),
        DynValue::Bool(b) => output.push_str(if *b { "true" } else { "false" }),
        DynValue::Integer(n) => {
            let mut buf = itoa::Buffer::new();
            output.push_str(buf.format(*n));
        }
        DynValue::Float(n) | DynValue::Currency(n, _, _) | DynValue::Percent(n) => {
            if n.is_finite() {
                output.push_str(&format_float(*n));
            } else {
                output.push_str("null");
            }
        }
        DynValue::FloatRaw(s) | DynValue::CurrencyRaw(s, _, _) => {
            output.push_str(s);
        }
        DynValue::String(s) | DynValue::Reference(s) | DynValue::Binary(s)
        | DynValue::Date(s) | DynValue::Timestamp(s) | DynValue::Time(s)
        | DynValue::Duration(s) => {
            output.push('"');
            for ch in s.chars() {
                match ch {
                    '"' => output.push_str("\\\""),
                    '\\' => output.push_str("\\\\"),
                    '\n' => output.push_str("\\n"),
                    '\r' => output.push_str("\\r"),
                    '\t' => output.push_str("\\t"),
                    c if c.is_control() => { let _ = write!(output, "\\u{:04x}", c as u32); }
                    c => output.push(c),
                }
            }
            output.push('"');
        }
        DynValue::Array(items) => {
            output.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    output.push(',');
                }
                if pretty {
                    output.push('\n');
                    for _ in 0..(depth + 1) * indent {
                        output.push(' ');
                    }
                }
                write_json_value(output, item, indent, depth + 1);
            }
            if pretty && !items.is_empty() {
                output.push('\n');
                for _ in 0..depth * indent {
                    output.push(' ');
                }
            }
            output.push(']');
        }
        DynValue::Object(entries) => {
            output.push('{');
            for (i, (key, val)) in entries.iter().enumerate() {
                if i > 0 {
                    output.push(',');
                }
                if pretty {
                    output.push('\n');
                    for _ in 0..(depth + 1) * indent {
                        output.push(' ');
                    }
                }
                output.push('"');
                output.push_str(key);
                output.push_str("\":");
                if pretty {
                    output.push(' ');
                }
                write_json_value(output, val, indent, depth + 1);
            }
            if pretty && !entries.is_empty() {
                output.push('\n');
                for _ in 0..depth * indent {
                    output.push(' ');
                }
            }
            output.push('}');
        }
    }
}

fn format_float(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        // Whole number: render without decimal point
        let mut buf = itoa::Buffer::new();
        buf.format(n as i64).to_string()
    } else {
        format_float_raw(n)
    }
}

/// Format an f64 preserving its natural representation.
/// Uses scientific notation for very large or very small values.
fn format_float_raw(n: f64) -> String {
    if n.abs() >= 1e15 || (n != 0.0 && n.abs() < 1e-4) {
        // Use scientific notation matching JS Number.toString() behavior
        format_scientific(n)
    } else {
        let mut buf = ryu::Buffer::new();
        buf.format(n).to_string()
    }
}

/// Format a number in scientific notation like JS: `6.022e+23`.
fn format_scientific(n: f64) -> String {
    if n == 0.0 {
        return "0".to_string();
    }
    // Use Rust's built-in {:e} formatting and normalize to match JS output
    let s = format!("{n:e}");
    // Rust formats as "6.022e23", JS wants "6.022e+23"
    if let Some(e_pos) = s.find('e') {
        let coeff = &s[..e_pos];
        let exp_str = &s[e_pos + 1..];
        // Clean up coefficient: remove trailing zeros
        let coeff = coeff.trim_end_matches('0').trim_end_matches('.');
        // Add explicit + sign for positive exponents
        if exp_str.starts_with('-') {
            format!("{coeff}e{exp_str}")
        } else {
            format!("{coeff}e+{exp_str}")
        }
    } else {
        s
    }
}

/// Format transform output as CSV with format options.
///
/// Supported options:
/// - `delimiter`: field separator (default `,`)
/// - `header`: "false" to suppress the header row (default true)
pub fn format_csv_with_opts(value: &DynValue, options: &std::collections::HashMap<String, String>) -> String {
    let delimiter = options.get("delimiter").map_or(",", String::as_str);
    let include_header = options.get("header").map_or(true, |v| v != "false");

    let mut output = String::new();
    // Unwrap single-key objects containing arrays (e.g., {"products": [...]})
    let value = match value {
        DynValue::Object(entries) if entries.len() == 1 => {
            if matches!(&entries[0].1, DynValue::Array(_)) {
                &entries[0].1
            } else {
                value
            }
        }
        _ => value,
    };
    if let DynValue::Array(rows) = value {
        if let Some(DynValue::Object(first)) = rows.first() {
            if include_header {
                let headers: Vec<&str> = first.iter().map(|(k, _)| k.as_str()).collect();
                output.push_str(&headers.join(delimiter));
                output.push('\n');
            }

            for row in rows {
                if let DynValue::Object(fields) = row {
                    let values: Vec<String> = fields.iter().map(|(_, v)| csv_value_with_delim(v, delimiter)).collect();
                    output.push_str(&values.join(delimiter));
                    output.push('\n');
                }
            }
        }
    }
    output
}

/// Format transform output as CSV.
pub fn format_csv(value: &DynValue) -> String {
    let opts = std::collections::HashMap::new();
    format_csv_with_opts(value, &opts)
}

fn csv_value(v: &DynValue) -> String {
    csv_value_with_delim(v, ",")
}

fn csv_value_with_delim(v: &DynValue, delimiter: &str) -> String {
    match v {
        DynValue::Bool(b) => b.to_string(),
        DynValue::Integer(n) => { let mut buf = itoa::Buffer::new(); buf.format(*n).to_string() }
        DynValue::Float(n) | DynValue::Percent(n) => format_float(*n),
        DynValue::Currency(n, dp, _) => format!("{:.prec$}", n, prec = *dp as usize),
        DynValue::FloatRaw(s) | DynValue::CurrencyRaw(s, _, _) => s.clone(),
        DynValue::String(s) | DynValue::Reference(s) | DynValue::Binary(s)
        | DynValue::Date(s) | DynValue::Timestamp(s) | DynValue::Time(s)
        | DynValue::Duration(s) => {
            // When using a non-comma delimiter, only quote if it contains the delimiter,
            // a double-quote, or a newline. With comma delimiter, quote if contains comma.
            if s.contains(delimiter) || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.clone()
            }
        }
        DynValue::Null | DynValue::Array(_) | DynValue::Object(_) => String::new(),
    }
}

// ---------------------------------------------------------------------------
// XML Formatter
// ---------------------------------------------------------------------------

/// Format transform output as XML.
///
/// - Wraps output in an XML declaration
/// - If the top-level value is an object with a single key, that key becomes
///   the root element name; otherwise `<root>` is used.
/// - Arrays produce repeated sibling elements
/// - Strings are XML-escaped
pub fn format_xml_with_options(
    value: &DynValue,
    indent: usize,
    _options: &std::collections::HashMap<String, String>,
) -> String {
    format_xml(value, indent)
}

/// Full XML formatter with odin:type attributes, :attr support, and xmlns:odin namespace.
/// Used by the transform engine when target format is XML.
///
/// Supported options:
/// - `declaration`: "false" to skip the `<?xml ...?>` declaration (default true)
/// - `indent`: indentation spaces (default 2)
pub fn format_xml_full(
    value: &DynValue,
    options: &std::collections::HashMap<String, String>,
    modifiers: &std::collections::HashMap<String, crate::types::values::OdinModifiers>,
) -> String {
    let include_declaration = options.get("declaration").map_or(true, |v| v != "false");
    let indent: usize = options.get("indent").and_then(|v| v.parse().ok()).unwrap_or(2);

    let mut output = String::new();
    if include_declaration {
        output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    }
    let needs_ns = xml_needs_odin_namespace(value);

    if let DynValue::Object(entries) = value {
        for (key, val) in entries {
            match val {
                DynValue::Array(items) => {
                    // Array sections: each item is a repeating element (no xmlns:odin)
                    for item in items {
                        xml_write_element_full(&mut output, key, item, indent, 0, false, modifiers, key);
                    }
                }
                _ => {
                    // Object sections: root-level elements get xmlns:odin
                    xml_write_element_full(&mut output, key, val, indent, 0, needs_ns, modifiers, key);
                }
            }
        }
    }

    output
}

/// Format a `DynValue` as XML with the given indentation level.
pub fn format_xml(value: &DynValue, indent: usize) -> String {
    let mut output = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");

    match value {
        DynValue::Object(entries) if entries.len() == 1 => {
            let (key, val) = &entries[0];
            write_xml_element(&mut output, key, val, indent, 0);
        }
        _ => {
            write_xml_element(&mut output, "root", value, indent, 0);
        }
    }

    output
}

/// Check if any value in the tree is non-string (needs odin:type attribute).
fn xml_needs_odin_namespace(value: &DynValue) -> bool {
    match value {
        DynValue::Null | DynValue::Bool(_) | DynValue::Integer(_) | DynValue::Float(_)
        | DynValue::Currency(_, _, _) | DynValue::CurrencyRaw(_, _, _)
        | DynValue::Percent(_) | DynValue::FloatRaw(_) => true,
        DynValue::Object(entries) => entries.iter().any(|(_, v)| xml_needs_odin_namespace(v)),
        DynValue::Array(items) => items.iter().any(xml_needs_odin_namespace),
        _ => false,
    }
}

/// Get the odin:type string for a `DynValue`, or None for string types.
/// Numeric values that are whole numbers are typed as "integer".
fn xml_odin_type(value: &DynValue) -> Option<&'static str> {
    match value {
        DynValue::Bool(_) => Some("boolean"),
        DynValue::Integer(_) => Some("integer"),
        DynValue::Float(f) | DynValue::Currency(f, _, _) | DynValue::Percent(f) => {
            if f.fract() == 0.0 && f.is_finite() { Some("integer") } else { Some("number") }
        }
        DynValue::FloatRaw(s) | DynValue::CurrencyRaw(s, _, _) => {
            if let Ok(f) = s.parse::<f64>() {
                if f.fract() == 0.0 && f.is_finite() { Some("integer") } else { Some("number") }
            } else {
                Some("number")
            }
        }
        DynValue::Date(_) => Some("date"),
        DynValue::Timestamp(_) => Some("timestamp"),
        DynValue::Time(_) => Some("time"),
        DynValue::Duration(_) => Some("duration"),
        DynValue::Reference(_) => Some("reference"),
        DynValue::Binary(_) => Some("binary"),
        _ => None,
    }
}

/// Convert a `DynValue` to its XML text content string.
fn xml_value_text(value: &DynValue) -> String {
    match value {
        DynValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        DynValue::Integer(n) => { let mut buf = itoa::Buffer::new(); buf.format(*n).to_string() }
        DynValue::Float(n) | DynValue::Currency(n, _, _) | DynValue::Percent(n) => format_float(*n),
        DynValue::FloatRaw(s) | DynValue::CurrencyRaw(s, _, _) => s.clone(),
        DynValue::String(s) | DynValue::Reference(s) | DynValue::Binary(s)
        | DynValue::Date(s) | DynValue::Timestamp(s) | DynValue::Time(s)
        | DynValue::Duration(s) => xml_escape(s),
        DynValue::Null | DynValue::Array(_) | DynValue::Object(_) => String::new(),
    }
}

/// Write an XML element with odin:type attributes and :attr support.
/// `section_key` is the top-level section name used for modifier path lookups.
#[allow(clippy::too_many_arguments)]
fn xml_write_element_full(
    output: &mut String,
    tag: &str,
    value: &DynValue,
    indent: usize,
    depth: usize,
    include_ns: bool,
    modifiers: &std::collections::HashMap<String, crate::types::values::OdinModifiers>,
    section_key: &str,
) {
    match value {
        DynValue::Null => {
            xml_indent(output, indent, depth);
            output.push('<');
            output.push_str(tag);
            output.push_str(" odin:type=\"null\"></");
            output.push_str(tag);
            output.push_str(">\n");
        }
        DynValue::Object(entries) => {
            xml_indent(output, indent, depth);
            output.push('<');
            output.push_str(tag);

            if include_ns {
                output.push_str(" xmlns:odin=\"https://odin.foundation/ns\"");
            }

            // Collect :attr fields as XML attributes
            let mut attr_keys = std::collections::HashSet::new();
            for (child_key, child_val) in entries {
                let mod_path = format!("{section_key}.{child_key}");
                if let Some(mods) = modifiers.get(&mod_path) {
                    if mods.attr {
                        attr_keys.insert(child_key.clone());
                        output.push(' ');
                        output.push_str(child_key);
                        output.push_str("=\"");
                        output.push_str(&xml_escape(&xml_value_text(child_val)));
                        output.push('"');
                    }
                }
            }

            output.push_str(">\n");

            // Write non-attr children
            for (child_key, child_val) in entries {
                if attr_keys.contains(child_key) {
                    continue;
                }
                let child_section = format!("{section_key}.{child_key}");
                xml_write_element_full(output, child_key, child_val, indent, depth + 1, false, modifiers, &child_section);
            }

            xml_indent(output, indent, depth);
            output.push_str("</");
            output.push_str(tag);
            output.push_str(">\n");
        }
        DynValue::Array(items) => {
            for item in items {
                xml_write_element_full(output, tag, item, indent, depth, false, modifiers, section_key);
            }
        }
        _ => {
            // Scalar value
            xml_indent(output, indent, depth);
            output.push('<');
            output.push_str(tag);
            if let Some(odin_type) = xml_odin_type(value) {
                output.push_str(" odin:type=\"");
                output.push_str(odin_type);
                output.push('"');
            }
            output.push('>');
            output.push_str(&xml_value_text(value));
            output.push_str("</");
            output.push_str(tag);
            output.push_str(">\n");
        }
    }
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

fn xml_indent(output: &mut String, indent: usize, depth: usize) {
    for _ in 0..indent * depth {
        output.push(' ');
    }
}

fn write_xml_element(
    output: &mut String,
    tag: &str,
    value: &DynValue,
    indent: usize,
    depth: usize,
) {
    match value {
        DynValue::Null => {
            xml_indent(output, indent, depth);
            output.push('<');
            output.push_str(tag);
            output.push_str(" odin:type=\"null\"></");
            output.push_str(tag);
            output.push_str(">\n");
        }
        DynValue::Bool(b) => {
            xml_indent(output, indent, depth);
            output.push('<');
            output.push_str(tag);
            output.push('>');
            output.push_str(if *b { "true" } else { "false" });
            output.push_str("</");
            output.push_str(tag);
            output.push_str(">\n");
        }
        DynValue::Integer(n) => {
            xml_indent(output, indent, depth);
            output.push('<');
            output.push_str(tag);
            output.push('>');
            let mut buf = itoa::Buffer::new();
            output.push_str(buf.format(*n));
            output.push_str("</");
            output.push_str(tag);
            output.push_str(">\n");
        }
        DynValue::Float(n) | DynValue::Currency(n, _, _) | DynValue::Percent(n) => {
            xml_indent(output, indent, depth);
            output.push('<');
            output.push_str(tag);
            output.push('>');
            output.push_str(&format_float(*n));
            output.push_str("</");
            output.push_str(tag);
            output.push_str(">\n");
        }
        DynValue::FloatRaw(s) | DynValue::CurrencyRaw(s, _, _) => {
            xml_indent(output, indent, depth);
            output.push('<');
            output.push_str(tag);
            output.push('>');
            output.push_str(s);
            output.push_str("</");
            output.push_str(tag);
            output.push_str(">\n");
        }
        DynValue::String(s) | DynValue::Reference(s) | DynValue::Binary(s)
        | DynValue::Date(s) | DynValue::Timestamp(s) | DynValue::Time(s)
        | DynValue::Duration(s) => {
            xml_indent(output, indent, depth);
            output.push('<');
            output.push_str(tag);
            output.push('>');
            output.push_str(&xml_escape(s));
            output.push_str("</");
            output.push_str(tag);
            output.push_str(">\n");
        }
        DynValue::Array(items) => {
            for item in items {
                write_xml_element(output, tag, item, indent, depth);
            }
        }
        DynValue::Object(entries) => {
            xml_indent(output, indent, depth);
            output.push('<');
            output.push_str(tag);
            output.push_str(">\n");
            for (key, val) in entries {
                write_xml_element(output, key, val, indent, depth + 1);
            }
            xml_indent(output, indent, depth);
            output.push_str("</");
            output.push_str(tag);
            output.push_str(">\n");
        }
    }
}

// ---------------------------------------------------------------------------
// Fixed-Width Formatter
// ---------------------------------------------------------------------------

/// Format transform output as fixed-width text.
///
/// `field_widths` is a slice of `(field_name, width)` pairs that define the
/// column layout. Each record (object in an array) produces one line.
/// Strings are left-aligned and numbers are right-aligned within their column.
/// Values are padded with spaces or truncated to fit the specified width.
pub fn format_fixed_width(value: &DynValue, field_widths: &[(String, usize)]) -> String {
    let mut output = String::new();

    // Unwrap single-key objects containing arrays
    let value = match value {
        DynValue::Object(entries) if entries.len() == 1 => {
            if matches!(&entries[0].1, DynValue::Array(_)) {
                &entries[0].1
            } else {
                value
            }
        }
        _ => value,
    };

    let records: &[DynValue] = match value {
        DynValue::Array(arr) => arr,
        DynValue::Object(_) => std::slice::from_ref(value),
        _ => return output,
    };

    for record in records {
        if let DynValue::Object(fields) = record {
            for (field_name, width) in field_widths {
                let val = fields
                    .iter()
                    .find(|(k, _)| k == field_name)
                    .map(|(_, v)| v);
                let text = fixed_width_value(val);
                let is_numeric = matches!(
                    val,
                    Some(DynValue::Integer(_) | DynValue::Float(_) | DynValue::Currency(_, _, _) |
DynValue::Percent(_))
                );
                if is_numeric {
                    // Right-align numbers
                    if text.len() >= *width {
                        output.push_str(&text[..(*width)]);
                    } else {
                        for _ in 0..(*width - text.len()) {
                            output.push(' ');
                        }
                        output.push_str(&text);
                    }
                } else {
                    // Left-align strings
                    if text.len() >= *width {
                        output.push_str(&text[..(*width)]);
                    } else {
                        output.push_str(&text);
                        for _ in 0..(*width - text.len()) {
                            output.push(' ');
                        }
                    }
                }
            }
            output.push('\n');
        }
    }

    output
}

fn fixed_width_value(v: Option<&DynValue>) -> String {
    match v {
        Some(DynValue::Bool(b)) => b.to_string(),
        Some(DynValue::Integer(n)) => { let mut buf = itoa::Buffer::new(); buf.format(*n).to_string() }
        Some(DynValue::Float(n) | DynValue::Currency(n, _, _) | DynValue::Percent(n)) => format_float(*n),
        Some(DynValue::FloatRaw(s) | DynValue::CurrencyRaw(s, _, _))
        | Some(DynValue::String(s) | DynValue::Reference(s) | DynValue::Binary(s)
        | DynValue::Date(s) | DynValue::Timestamp(s) | DynValue::Time(s)
        | DynValue::Duration(s)) => s.clone(),
        None | Some(DynValue::Null) | Some(DynValue::Array(_) | DynValue::Object(_)) => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Fixed-Width Segment-Aware Formatter
// ---------------------------------------------------------------------------

/// Collect all formatting directives from a `FieldMapping`, searching both
/// FieldMapping.directives AND verb arg references in the expression.
fn collect_all_fwf_directives(mapping: &crate::types::transform::FieldMapping) -> Vec<crate::types::values::OdinDirective> {
    let mut dirs = mapping.directives.clone();
    // Also collect from verb arg references (where :pos/:len may live)
    collect_expr_directives(&mapping.expression, &mut dirs);
    dirs
}

fn collect_expr_directives(expr: &crate::types::transform::FieldExpression, dirs: &mut Vec<crate::types::values::OdinDirective>) {
    use crate::types::transform::{FieldExpression, VerbArg};
    if let FieldExpression::Transform(verb_call) = expr {
        for arg in &verb_call.args {
            match arg {
                VerbArg::Reference(_, ref_dirs) => {
                    for d in ref_dirs {
                        if !dirs.iter().any(|existing| existing.name == d.name) {
                            dirs.push(d.clone());
                        }
                    }
                }
                VerbArg::Verb(nested) => {
                    for arg2 in &nested.args {
                        if let VerbArg::Reference(_, ref_dirs) = arg2 {
                            for d in ref_dirs {
                                if !dirs.iter().any(|existing| existing.name == d.name) {
                                    dirs.push(d.clone());
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Format transform output as fixed-width text using segment mapping directives
/// for per-field positioning (`:pos`, `:len`, `:leftPad`, `:rightPad`).
///
/// This is the export-mode formatter that builds positional output from transform
/// segment definitions, matching the TypeScript `formatFixedWidthLine()`.
pub fn format_fixed_width_from_segments(
    output: &DynValue,
    segments: &[crate::types::transform::TransformSegment],
    options: &std::collections::HashMap<String, String>,
) -> String {
    let line_ending = options.get("lineEnding").map_or("\n", std::string::String::as_str);
    let default_pad = options.get("padChar").map_or(" ", std::string::String::as_str);
    let mut lines: Vec<String> = Vec::new();

    for segment in segments {
        let seg_name = segment.name.strip_suffix("[]").unwrap_or(&segment.name);

        // Check if any mapping has :pos/:len directives (in mapping.directives or verb arg refs)
        let has_positional = segment.mappings.iter().any(|m| {
            let dirs = collect_all_fwf_directives(m);
            dirs.iter().any(|d| d.name == "pos" || d.name == "len")
        });

        if !has_positional {
            // No positional directives — skip or use generic formatting
            continue;
        }

        // Resolve the segment's data from the output
        let data = resolve_segment_data(output, seg_name);

        match data {
            Some(DynValue::Array(items)) => {
                for item in items {
                    if let DynValue::Object(fields) = &item {
                        lines.push(format_fwf_line(&segment.mappings, fields, default_pad));
                    }
                }
            }
            Some(DynValue::Object(fields)) => {
                lines.push(format_fwf_line(&segment.mappings, fields, default_pad));
            }
            _ => {
                // Try to get data from output directly (segment name is the top-level key)
                if let DynValue::Object(top) = output {
                    if let Some((_, val)) = top.iter().find(|(k, _)| k == seg_name) {
                        match val {
                            DynValue::Array(items) => {
                                for item in items {
                                    if let DynValue::Object(fields) = item {
                                        lines.push(format_fwf_line(&segment.mappings, fields, default_pad));
                                    }
                                }
                            }
                            DynValue::Object(fields) => {
                                lines.push(format_fwf_line(&segment.mappings, fields, default_pad));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    lines.join(line_ending)
}

/// Resolve segment data from the output object.
fn resolve_segment_data<'a>(output: &'a DynValue, seg_name: &str) -> Option<&'a DynValue> {
    if let DynValue::Object(entries) = output {
        // Direct match
        if let Some((_, val)) = entries.iter().find(|(k, _)| k == seg_name) {
            return Some(val);
        }
        // If output is a single top-level object, look inside
        if entries.len() == 1 {
            if let DynValue::Object(inner) = &entries[0].1 {
                if let Some((_, val)) = inner.iter().find(|(k, _)| k == seg_name) {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// Format a single fixed-width line from field mappings and record data.
fn format_fwf_line(
    mappings: &[crate::types::transform::FieldMapping],
    fields: &[(String, DynValue)],
    default_pad: &str,
) -> String {
    use crate::types::values::DirectiveValue;

    // Collect fields with their position info, sorted by position
    #[allow(clippy::type_complexity)]
    let mut positioned: Vec<(usize, usize, String, Option<String>, Option<String>)> = Vec::new(); // (pos, len, value, leftPad, rightPad)

    for mapping in mappings {
        // Skip internal mappings
        if mapping.target.starts_with('_') {
            continue;
        }

        let mut pos: Option<usize> = None;
        let mut len: Option<usize> = None;
        let mut left_pad: Option<String> = None;
        let mut right_pad: Option<String> = None;

        // Collect directives from both FieldMapping.directives and verb arg references
        let all_dirs = collect_all_fwf_directives(mapping);
        for dir in &all_dirs {
            match dir.name.as_str() {
                "pos" => {
                    if let Some(DirectiveValue::Number(n)) = &dir.value {
                        pos = Some(*n as usize);
                    }
                }
                "len" => {
                    if let Some(DirectiveValue::Number(n)) = &dir.value {
                        len = Some(*n as usize);
                    }
                }
                "leftPad" => {
                    if let Some(DirectiveValue::String(s)) = &dir.value {
                        left_pad = Some(s.clone());
                    }
                }
                "rightPad" => {
                    if let Some(DirectiveValue::String(s)) = &dir.value {
                        right_pad = Some(s.clone());
                    }
                }
                _ => {}
            }
        }

        if let (Some(p), Some(l)) = (pos, len) {
            // Get the field value from the record
            let field_name = mapping.target.split('.').next_back().unwrap_or(&mapping.target);
            let val = fields.iter()
                .find(|(k, _)| k == field_name || k == &mapping.target)
                .map(|(_, v)| v);
            let text = fixed_width_value(val);
            positioned.push((p, l, text, left_pad, right_pad));
        }
    }

    // Sort by position
    positioned.sort_by_key(|(p, _, _, _, _)| *p);

    // Build the line
    let mut line = String::new();

    for (pos, len, text, left_pad, right_pad) in &positioned {
        // Fill gap to position
        while line.len() < *pos {
            line.push_str(default_pad);
        }

        let mut field_text = text.clone();

        // Truncate if too long
        if field_text.len() > *len {
            field_text = field_text[..*len].to_string();
        }

        // Apply padding
        if field_text.len() < *len {
            let pad_char = if let Some(lp) = left_pad {
                lp.chars().next().unwrap_or(' ')
            } else if let Some(rp) = right_pad {
                rp.chars().next().unwrap_or(' ')
            } else {
                ' '
            };

            if left_pad.is_some() {
                // Left pad (right-align)
                let padding = *len - field_text.len();
                let mut padded = String::new();
                for _ in 0..padding {
                    padded.push(pad_char);
                }
                padded.push_str(&field_text);
                field_text = padded;
            } else {
                // Right pad (left-align) — default
                let padding = *len - field_text.len();
                for _ in 0..padding {
                    field_text.push(pad_char);
                }
            }
        }

        // Place at position (overwrite if needed)
        if line.len() <= *pos {
            line.push_str(&field_text);
        } else {
            // Overwrite existing content at this position
            let before = &line[..*pos];
            let after_pos = pos + field_text.len();
            let after = if after_pos < line.len() { &line[after_pos..] } else { "" };
            line = format!("{before}{field_text}{after}");
        }
    }

    line
}

// ---------------------------------------------------------------------------
// Flat (key=value) Formatter
// ---------------------------------------------------------------------------

/// Format transform output as flat key-value lines.
///
/// - Nested objects use dot notation: `a.b.c = value`
/// - Arrays use bracket notation: `items[0] = value`
/// - Null values are skipped
pub fn format_flat(value: &DynValue) -> String {
    format_flat_kvp(value)
}

/// Format as flat key=value pairs, sorted alphabetically.
pub fn format_flat_kvp(value: &DynValue) -> String {
    let mut pairs: Vec<(String, String)> = Vec::new();
    collect_flat_pairs(&mut pairs, value, "");
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut output = String::new();
    for (path, val) in &pairs {
        output.push_str(path);
        output.push('=');
        output.push_str(val);
        output.push('\n');
    }
    output
}

fn collect_flat_pairs(pairs: &mut Vec<(String, String)>, value: &DynValue, prefix: &str) {
    match value {
        DynValue::Null => {} // Skip nulls
        DynValue::Bool(b) => {
            pairs.push((prefix.to_string(), b.to_string()));
        }
        DynValue::Integer(n) => {
            let mut buf = itoa::Buffer::new();
            pairs.push((prefix.to_string(), buf.format(*n).to_string()));
        }
        DynValue::Float(n) | DynValue::Currency(n, _, _) | DynValue::Percent(n) => {
            pairs.push((prefix.to_string(), format_float(*n)));
        }
        DynValue::FloatRaw(s) | DynValue::CurrencyRaw(s, _, _)
        | DynValue::String(s) | DynValue::Reference(s) | DynValue::Binary(s)
        | DynValue::Date(s) | DynValue::Timestamp(s) | DynValue::Time(s)
        | DynValue::Duration(s) => {
            pairs.push((prefix.to_string(), s.clone()));
        }
        DynValue::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let child_prefix = format!("{prefix}[{i}]");
                collect_flat_pairs(pairs, item, &child_prefix);
            }
        }
        DynValue::Object(entries) => {
            for (key, val) in entries {
                let child_prefix = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                collect_flat_pairs(pairs, val, &child_prefix);
            }
        }
    }
}

/// Format as flat YAML output, sorted alphabetically with nesting.
pub fn format_flat_yaml(value: &DynValue) -> String {
    // Build a tree from flat paths, then render as YAML
    let mut pairs: Vec<(String, String)> = Vec::new();
    collect_flat_pairs(&mut pairs, value, "");
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    // Build tree structure
    let mut root = YamlNode::Map(Vec::new());
    for (path, val) in &pairs {
        insert_yaml_path(&mut root, path, val);
    }

    let mut output = String::new();
    write_yaml_node(&mut output, &root, 0, false);
    output
}

#[derive(Debug)]
enum YamlNode {
    Leaf(String),
    Map(Vec<(String, YamlNode)>),
    List(Vec<YamlNode>),
}

fn insert_yaml_path(root: &mut YamlNode, path: &str, value: &str) {
    // Parse path into segments, e.g., "employees[0].name" => ["employees", "[0]", "name"]
    let segments = parse_yaml_path(path);
    insert_yaml_recursive(root, &segments, value);
}

fn parse_yaml_path(path: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !current.is_empty() {
                    segments.push(current.clone());
                    current.clear();
                }
            }
            '[' => {
                if !current.is_empty() {
                    segments.push(current.clone());
                    current.clear();
                }
                let mut idx = String::from("[");
                while let Some(&c) = chars.peek() {
                    chars.next();
                    idx.push(c);
                    if c == ']' { break; }
                }
                segments.push(idx);
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn insert_yaml_recursive(node: &mut YamlNode, segments: &[String], value: &str) {
    if segments.is_empty() {
        *node = YamlNode::Leaf(value.to_string());
        return;
    }

    let seg = &segments[0];
    let rest = &segments[1..];

    if seg.starts_with('[') && seg.ends_with(']') {
        // Array index
        let idx: usize = seg[1..seg.len()-1].parse().unwrap_or(0);
        if let YamlNode::List(items) = node {
            while items.len() <= idx {
                items.push(YamlNode::Map(Vec::new()));
            }
            insert_yaml_recursive(&mut items[idx], rest, value);
        }
    } else {
        // Object key
        if let YamlNode::Map(entries) = node {
            // Find existing entry
            let pos = entries.iter().position(|(k, _)| k == seg);
            if let Some(p) = pos {
                insert_yaml_recursive(&mut entries[p].1, rest, value);
            } else {
                // Determine whether next segment is array index
                let child = if rest.first().is_some_and(|s| s.starts_with('[')) {
                    YamlNode::List(Vec::new())
                } else if rest.is_empty() {
                    YamlNode::Leaf(String::new())
                } else {
                    YamlNode::Map(Vec::new())
                };
                entries.push((seg.clone(), child));
                let last = entries.len() - 1;
                insert_yaml_recursive(&mut entries[last].1, rest, value);
            }
        }
    }
}

fn write_yaml_node(output: &mut String, node: &YamlNode, depth: usize, is_list_item: bool) {
    let indent = "  ".repeat(depth);
    match node {
        YamlNode::Leaf(val) => {
            // Quote values that need quoting in YAML
            let formatted = yaml_format_value(val);
            output.push_str(&formatted);
            output.push('\n');
        }
        YamlNode::Map(entries) => {
            for (i, (key, child)) in entries.iter().enumerate() {
                if is_list_item && i == 0 {
                    // First entry after list dash uses reduced indent
                    output.push_str(key);
                    output.push_str(": ");
                } else {
                    output.push_str(&indent);
                    output.push_str(key);
                    output.push_str(": ");
                }
                match child {
                    YamlNode::Leaf(val) => {
                        output.push_str(&yaml_format_value(val));
                        output.push('\n');
                    }
                    YamlNode::Map(_) => {
                        // No trailing space after colon for container values
                        // Remove the space we just pushed
                        output.pop();
                        output.push('\n');
                        write_yaml_node(output, child, depth + 1, false);
                    }
                    YamlNode::List(_) => {
                        output.pop();
                        output.push('\n');
                        write_yaml_node(output, child, depth + 1, false);
                    }
                }
            }
        }
        YamlNode::List(items) => {
            for item in items {
                output.push_str(&indent);
                output.push_str("- ");
                match item {
                    YamlNode::Leaf(val) => {
                        output.push_str(&yaml_format_value(val));
                        output.push('\n');
                    }
                    YamlNode::Map(entries) => {
                        if entries.is_empty() {
                            output.push_str("{}\n");
                        } else {
                            // First entry on same line as dash
                            write_yaml_node(output, item, depth + 1, true);
                        }
                    }
                    YamlNode::List(_) => {
                        output.push('\n');
                        write_yaml_node(output, item, depth + 2, false);
                    }
                }
            }
        }
    }
}

fn yaml_format_value(val: &str) -> String {
    // Check if value needs quoting
    if val.is_empty() {
        return "\"\"".to_string();
    }
    // Quote values containing special YAML characters
    if val.contains(": ") || val.starts_with(':') || val.contains("://")
        || val.contains('#') || val.contains('{') || val.contains('}')
        || val.contains('[') || val.contains(']') || val.contains(',')
        || val.contains('&') || val.contains('*') || val.contains('!')
        || val.contains('|') || val.contains('>') || val.contains('\'')
        || val.contains('%')
        || val.starts_with(' ') || val.ends_with(' ')
        || val.starts_with('"') || val.starts_with('\'')
    {
        return format!("\"{}\"", val.replace('\\', "\\\\").replace('"', "\\\""));
    }
    // Quote boolean-like and null-like values
    match val {
        "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~" | "" => {
            format!("\"{val}\"")
        }
        _ => val.to_string(),
    }
}

// ---------------------------------------------------------------------------
// ODIN Formatter
// ---------------------------------------------------------------------------

/// Format transform output as ODIN text.
///
/// - Produces a `{$}` header with `odin = "1.0.0"`
/// - Strings are double-quoted
/// - Integers use `##` prefix, floats use `#` prefix
/// - Booleans are bare `true`/`false`
/// - Null is `~`
/// - Top-level object keys become `{SectionName}` headers (capitalised)
pub fn format_odin_with_modifiers(
    value: &DynValue,
    modifiers: &std::collections::HashMap<String, crate::types::values::OdinModifiers>,
    include_header: bool,
) -> String {
    let mut output = String::new();
    if include_header {
        output.push_str("{$}\nodin = \"1.0.0\"\n");
    }

    if let DynValue::Object(entries) = value {
        let has_sections = entries
            .iter()
            .any(|(_, v)| matches!(v, DynValue::Object(_) | DynValue::Array(_)));

        if has_sections {
            if include_header {
                output.push_str("{}\n");
            }
            for (key, val) in entries {
                match val {
                    DynValue::Object(_) => {
                        collect_leaf_paths_with_mods(&mut output, key, val, key, modifiers);
                    }
                    DynValue::Array(_) => {}
                    _ => write_odin_assignment_with_mods(&mut output, key, val, key, modifiers),
                }
            }
            for (key, val) in entries {
                match val {
                    DynValue::Object(_) if !is_pure_leaf_chain(val) => {
                        write_odin_section_with_mods(&mut output, key, val, modifiers);
                    }
                    DynValue::Array(items) => {
                        write_odin_array_section(&mut output, key, items);
                    }
                    _ => {}
                }
            }
        } else {
            for (key, val) in entries {
                write_odin_assignment_with_mods(&mut output, key, val, key, modifiers);
            }
        }
    } else {
        write_odin_value(&mut output, value);
        output.push('\n');
    }

    output
}

/// Format a `DynValue` as ODIN text, optionally including the `{$}` header.
pub fn format_odin(value: &DynValue, include_header: bool) -> String {
    let mut output = String::new();
    if include_header {
        output.push_str("{$}\nodin = \"1.0.0\"\n");
    }

    match value {
        DynValue::Object(entries) => {
            let has_sections = entries
                .iter()
                .any(|(_, v)| matches!(v, DynValue::Object(_) | DynValue::Array(_)));

            if has_sections {
                if include_header {
                    output.push_str("{}\n");
                }
                // Output flat top-level fields and leaf chains under root scope
                for (key, val) in entries {
                    match val {
                        DynValue::Object(_) => {
                            // Check if this is a pure leaf chain (no branching)
                            collect_leaf_paths(&mut output, key, val);
                        }
                        DynValue::Array(_) => {} // handled below
                        _ => write_odin_assignment(&mut output, key, val),
                    }
                }
                // Then output proper sections (non-leaf-chain objects and arrays)
                for (key, val) in entries {
                    match val {
                        DynValue::Object(fields) => {
                            if !is_pure_leaf_chain(val) {
                                write_odin_section(&mut output, key, fields, key);
                            }
                        }
                        DynValue::Array(items) => {
                            write_odin_array_section(&mut output, key, items);
                        }
                        _ => {}
                    }
                }
            } else {
                for (key, val) in entries {
                    write_odin_assignment(&mut output, key, val);
                }
            }
        }
        DynValue::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    output.push('\n');
                }
                output.push_str("{item}\n");
                write_odin_fields(&mut output, item);
            }
        }
        _ => {
            write_odin_assignment(&mut output, "value", value);
        }
    }

    output
}

/// Check if a `DynValue::Object` is a pure leaf chain — a chain of single-child objects
/// leading exclusively to scalar values (no branching, no arrays).
fn is_pure_leaf_chain(val: &DynValue) -> bool {
    match val {
        DynValue::Object(fields) => {
            // Must have exactly one entry
            if fields.len() != 1 {
                return false;
            }
            match &fields[0].1 {
                DynValue::Object(_) => is_pure_leaf_chain(&fields[0].1),
                DynValue::Array(_) => false,
                _ => true, // Scalar leaf
            }
        }
        _ => false,
    }
}

/// Collect all leaf paths from a pure leaf chain as dotted path assignments.
fn collect_leaf_paths(output: &mut String, prefix: &str, val: &DynValue) {
    if !is_pure_leaf_chain(val) {
        return;
    }
    collect_leaf_paths_inner(output, prefix, val);
}

fn collect_leaf_paths_inner(output: &mut String, prefix: &str, val: &DynValue) {
    match val {
        DynValue::Object(fields) => {
            for (key, child) in fields {
                let path = format!("{prefix}.{key}");
                match child {
                    DynValue::Object(_) => collect_leaf_paths_inner(output, &path, child),
                    _ => write_odin_assignment(output, &path, child),
                }
            }
        }
        _ => write_odin_assignment(output, prefix, val),
    }
}

/// Write an ODIN value directly to the output buffer — avoids intermediate String allocation.
fn write_odin_value(output: &mut String, value: &DynValue) {
    use std::fmt::Write;
    match value {
        DynValue::Bool(b) => output.push_str(if *b { "?true" } else { "?false" }),
        DynValue::Integer(n) => { output.push_str("##"); let _ = write!(output, "{n}"); }
        DynValue::Float(n) => {
            if n.is_finite() {
                output.push('#');
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    let _ = write!(output, "{}", *n as i64);
                } else {
                    output.push_str(&format_float_raw(*n));
                }
            } else {
                output.push('~');
            }
        }
        DynValue::FloatRaw(s) => { output.push('#'); output.push_str(s); }
        DynValue::CurrencyRaw(s, _dp, code) => {
            output.push_str("#$");
            output.push_str(s);
            if let Some(c) = code { output.push(':'); output.push_str(c); }
        }
        DynValue::Currency(n, dp, code) => {
            if n.is_finite() {
                output.push_str("#$");
                let _ = write!(output, "{:.prec$}", n, prec = *dp as usize);
                if let Some(c) = code { output.push(':'); output.push_str(c); }
            } else {
                output.push('~');
            }
        }
        DynValue::Percent(n) => {
            if n.is_finite() {
                output.push_str("#%");
                let _ = write!(output, "{n}");
            } else {
                output.push('~');
            }
        }
        DynValue::Reference(path) => { output.push('@'); output.push_str(path); }
        DynValue::Binary(data) => { output.push('^'); output.push_str(data); }
        DynValue::Date(s) | DynValue::Duration(s) => output.push_str(s),
        DynValue::Timestamp(s) => output.push_str(&normalize_timestamp(s)),
        DynValue::Time(s) => {
            if !s.starts_with('T') { output.push('T'); }
            output.push_str(s);
        }
        DynValue::String(s) => {
            output.push('"');
            write_escaped_odin_string(output, s);
            output.push('"');
        }
        DynValue::Null | DynValue::Array(_) | DynValue::Object(_) => output.push('~'),
    }
}

/// Wrapper that returns a String — used by tests and callers that need an owned value.
fn odin_value_string(value: &DynValue) -> String {
    let mut s = String::new();
    write_odin_value(&mut s, value);
    s
}

/// Escape special characters in an ODIN string value.
/// Normalize a timestamp to UTC with milliseconds.
///
/// Converts timezone offsets to UTC and ensures `.000Z` suffix.
/// - `2024-12-15T10:30:00Z` → `2024-12-15T10:30:00.000Z`
/// - `2024-12-15T10:30:00+05:30` → `2024-12-15T05:00:00.000Z`
fn normalize_timestamp(s: &str) -> String {
    // Parse: YYYY-MM-DDThh:mm:ss[.fff][Z|+HH:MM|-HH:MM]
    let s = s.trim();
    if s.len() < 19 {
        return s.to_string();
    }
    // Parse date-time components
    let date_part = &s[..10]; // YYYY-MM-DD
    let time_start = if s.as_bytes().get(10) == Some(&b'T') { 11 } else { return s.to_string(); };
    let time_part = &s[time_start..];

    // Parse hours, minutes, seconds
    if time_part.len() < 8 {
        return s.to_string();
    }
    let hh: i32 = time_part[0..2].parse().unwrap_or(0);
    let mm: i32 = time_part[3..5].parse().unwrap_or(0);
    let ss: i32 = time_part[6..8].parse().unwrap_or(0);

    // Parse milliseconds if present
    let mut millis = 0u32;
    let mut rest_idx = 8;
    if time_part.len() > 8 && time_part.as_bytes()[8] == b'.' {
        rest_idx = 9;
        let mut frac_digits = String::new();
        while rest_idx < time_part.len() && time_part.as_bytes()[rest_idx].is_ascii_digit() {
            frac_digits.push(time_part.as_bytes()[rest_idx] as char);
            rest_idx += 1;
        }
        // Pad or truncate to 3 digits
        while frac_digits.len() < 3 {
            frac_digits.push('0');
        }
        millis = frac_digits[..3].parse().unwrap_or(0);
    }

    // Parse timezone offset
    let tz_part = &time_part[rest_idx..];
    let (offset_h, offset_m): (i32, i32) = if tz_part.is_empty() || tz_part == "Z" {
        (0, 0)
    } else if tz_part.len() >= 6 && (tz_part.starts_with('+') || tz_part.starts_with('-')) {
        let sign: i32 = if tz_part.starts_with('-') { -1 } else { 1 };
        let oh: i32 = tz_part[1..3].parse().unwrap_or(0);
        let om: i32 = tz_part[4..6].parse().unwrap_or(0);
        (sign * oh, sign * om)
    } else {
        (0, 0)
    };

    // Convert to total minutes from midnight, subtract offset to get UTC
    let year: i32 = date_part[..4].parse().unwrap_or(2000);
    let month: u32 = date_part[5..7].parse().unwrap_or(1);
    let day: u32 = date_part[8..10].parse().unwrap_or(1);

    let mut total_minutes = (hh * 60 + mm) - (offset_h * 60 + offset_m);
    let total_seconds = ss;
    let mut d = day as i32;
    let mut m = month as i32;
    let mut y = year;

    // Handle day rollover
    if total_minutes < 0 {
        total_minutes += 24 * 60;
        d -= 1;
        if d < 1 {
            m -= 1;
            if m < 1 {
                m = 12;
                y -= 1;
            }
            d = days_in_month_ts(y, m as u32) as i32;
        }
    } else if total_minutes >= 24 * 60 {
        total_minutes -= 24 * 60;
        d += 1;
        let dim = days_in_month_ts(y, m as u32) as i32;
        if d > dim {
            d = 1;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
    }
    let _ = total_seconds; // already handled
    let utc_hh = total_minutes / 60;
    let utc_mm = total_minutes % 60;

    format!("{y:04}-{m:02}-{d:02}T{utc_hh:02}:{utc_mm:02}:{ss:02}.{millis:03}Z")
}

fn days_in_month_ts(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        2 => if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 29 } else { 28 },
        _ => 30,
    }
}

fn escape_odin_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    write_escaped_odin_string(&mut out, s);
    out
}

/// Write escaped ODIN string directly to buffer — avoids intermediate allocation.
fn write_escaped_odin_string(output: &mut String, s: &str) {
    // Fast path: no special chars
    let bytes = s.as_bytes();
    if !bytes.iter().any(|&b| b == b'"' || b == b'\\' || b == b'\n' || b == b'\r' || b == b'\t') {
        output.push_str(s);
        return;
    }
    let mut last = 0;
    for (i, ch) in s.char_indices() {
        let esc = match ch {
            '"' => Some("\\\""),
            '\\' => Some("\\\\"),
            '\n' => Some("\\n"),
            '\r' => Some("\\r"),
            '\t' => Some("\\t"),
            _ => None,
        };
        if let Some(e) = esc {
            output.push_str(&s[last..i]);
            output.push_str(e);
            last = i + ch.len_utf8();
        }
    }
    output.push_str(&s[last..]);
}

fn write_odin_assignment(output: &mut String, key: &str, value: &DynValue) {
    output.push_str(key);
    output.push_str(" = ");
    write_odin_value(output, value);
    output.push('\n');
}

fn write_odin_assignment_with_mods(
    output: &mut String,
    key: &str,
    value: &DynValue,
    full_path: &str,
    modifiers: &std::collections::HashMap<String, crate::types::values::OdinModifiers>,
) {
    output.push_str(key);
    output.push_str(" = ");
    // Prepend modifier prefixes: ! (required), - (deprecated), * (confidential)
    if let Some(mods) = modifiers.get(full_path) {
        if mods.required { output.push('!'); }
        if mods.deprecated { output.push('-'); }
        if mods.confidential { output.push('*'); }
    }
    write_odin_value(output, value);
    output.push('\n');
}

fn collect_leaf_paths_with_mods(
    output: &mut String,
    prefix: &str,
    value: &DynValue,
    full_path_prefix: &str,
    modifiers: &std::collections::HashMap<String, crate::types::values::OdinModifiers>,
) {
    if is_pure_leaf_chain(value) {
        if let DynValue::Object(entries) = value {
            for (key, val) in entries {
                let dotted = format!("{prefix}.{key}");
                let full_path = format!("{full_path_prefix}.{key}");
                match val {
                    DynValue::Object(_) => {
                        collect_leaf_paths_with_mods(output, &dotted, val, &full_path, modifiers);
                    }
                    _ => {
                        write_odin_assignment_with_mods(output, &dotted, val, &full_path, modifiers);
                    }
                }
            }
        }
    }
}

fn write_odin_section_with_mods(
    output: &mut String,
    key: &str,
    value: &DynValue,
    modifiers: &std::collections::HashMap<String, crate::types::values::OdinModifiers>,
) {
    if let DynValue::Object(entries) = value {
        output.push('{');
        output.push_str(key);
        output.push_str("}\n");

        // Phase 1: Inline items (scalars and leaf chains)
        for (child_key, child_val) in entries {
            match child_val {
                DynValue::Object(_) if !is_pure_leaf_chain(child_val) => {}
                DynValue::Array(_) => {}
                DynValue::Object(_) => {
                    let full_path = format!("{key}.{child_key}");
                    collect_leaf_paths_with_mods(output, child_key, child_val, &full_path, modifiers);
                }
                _ => {
                    let full_path = format!("{key}.{child_key}");
                    write_odin_assignment_with_mods(output, child_key, child_val, &full_path, modifiers);
                }
            }
        }
        // Phase 2: Array subsections first, then object subsections.
        // This matches the TS formatter behavior where arrays appear before objects.
        for (child_key, child_val) in entries {
            if let DynValue::Array(items) = child_val {
                write_odin_array_subsection(output, child_key, items);
            }
        }
        // Each child is a direct child of this absolute section
        for (child_key, child_val) in entries {
            match child_val {
                DynValue::Object(sub_entries) if !is_pure_leaf_chain(child_val) => {
                    let sub_path = format!("{key}.{child_key}");
                    write_odin_subsection_with_mods_inner(output, &sub_path, child_key, sub_entries, key, modifiers, false);
                }
                _ => {}
            }
        }
    }
}

fn write_odin_subsection_with_mods(
    output: &mut String,
    full_path: &str,
    name: &str,
    fields: &[(String, DynValue)],
    current_scope: &str,
    modifiers: &std::collections::HashMap<String, crate::types::values::OdinModifiers>,
) {
    write_odin_subsection_with_mods_inner(output, full_path, name, fields, current_scope, modifiers, false);
}

fn write_odin_subsection_with_mods_inner(
    output: &mut String,
    full_path: &str,
    name: &str,
    fields: &[(String, DynValue)],
    current_scope: &str,
    modifiers: &std::collections::HashMap<String, crate::types::values::OdinModifiers>,
    inside_relative: bool,
) {
    // Determine relative vs absolute notation
    // Rule: only direct children of absolute headers can use relative notation
    let parent_of_full = full_path.rsplit_once('.').map_or("", |(p, _)| p);
    let can_use_relative = parent_of_full == current_scope && !inside_relative;
    let this_is_relative;
    if can_use_relative {
        output.push_str("{.");
        output.push_str(name);
        output.push_str("}\n");
        this_is_relative = true;
    } else {
        output.push('{');
        output.push_str(full_path);
        output.push_str("}\n");
        this_is_relative = false;
    }

    // Phase 1: Inline items
    for (child_key, child_val) in fields {
        match child_val {
            DynValue::Object(_) if is_pure_leaf_chain(child_val) => {
                let child_full = format!("{full_path}.{child_key}");
                collect_leaf_paths_with_mods(output, child_key, child_val, &child_full, modifiers);
            }
            DynValue::Object(_) | DynValue::Array(_) => {}
            _ => {
                let child_full = format!("{full_path}.{child_key}");
                write_odin_assignment_with_mods(output, child_key, child_val, &child_full, modifiers);
            }
        }
    }

    // Phase 2: Array subsections first, then object subsections (matches TS behavior)
    for (child_key, child_val) in fields {
        if let DynValue::Array(items) = child_val {
            write_odin_array_subsection(output, child_key, items);
        }
    }
    // Each child is evaluated against this section's scope
    for (child_key, child_val) in fields {
        match child_val {
            DynValue::Object(sub_fields) if !is_pure_leaf_chain(child_val) => {
                let sub_path = format!("{full_path}.{child_key}");
                write_odin_subsection_with_mods_inner(output, &sub_path, child_key, sub_fields, full_path, modifiers, this_is_relative);
            }
            _ => {}
        }
    }
}

fn write_odin_fields(output: &mut String, value: &DynValue) {
    if let DynValue::Object(entries) = value {
        for (key, val) in entries {
            match val {
                DynValue::Object(_) => write_odin_nested(output, key, val),
                DynValue::Array(_) => {} // Arrays handled separately
                _ => write_odin_assignment(output, key, val),
            }
        }
    }
}

/// Write nested object fields as dotted paths.
fn write_odin_nested(output: &mut String, prefix: &str, value: &DynValue) {
    if let DynValue::Object(entries) = value {
        for (key, val) in entries {
            let full_path = format!("{prefix}.{key}");
            match val {
                DynValue::Object(_) => write_odin_nested(output, &full_path, val),
                DynValue::Array(_) => {} // Arrays handled separately
                _ => write_odin_assignment(output, &full_path, val),
            }
        }
    }
}

/// Write a top-level ODIN section with proper sub-section handling.
///
/// `scope_path` tracks the current ODIN scope for determining absolute vs relative paths.
fn write_odin_section(output: &mut String, name: &str, fields: &[(String, DynValue)], scope_path: &str) {
    output.push('{');
    output.push_str(name);
    output.push_str("}\n");

    // Phase 1: Output inline items (scalars + leaf chains) in original field order
    for (key, val) in fields {
        match val {
            DynValue::Object(_) if is_pure_leaf_chain(val) => {
                collect_leaf_paths_inner(output, key, val);
            }
            DynValue::Object(_) | DynValue::Array(_) => {}
            _ => write_odin_assignment(output, key, val),
        }
    }

    // Phase 2: Array subsections first, then object subsections (matches TS behavior)
    for (key, val) in fields {
        if let DynValue::Array(items) = val {
            write_odin_array_subsection(output, key, items);
        }
    }
    // Each child is a direct child of this absolute section, so each gets evaluated
    // against the section scope independently (inside_relative = false).
    for (key, val) in fields {
        match val {
            DynValue::Object(sub_fields) if !is_pure_leaf_chain(val) => {
                let sub_path = format!("{scope_path}.{key}");
                write_odin_subsection_inner(output, &sub_path, key, sub_fields, scope_path, false);
            }
            _ => {}
        }
    }
}

/// Write a sub-section within a parent section.
///
/// `full_path` is the absolute path to this section (e.g., `"nested.person"`).
/// `current_scope` is where the ODIN reader's "cursor" currently is.
fn write_odin_subsection(output: &mut String, full_path: &str, name: &str, fields: &[(String, DynValue)], current_scope: &str) {
    write_odin_subsection_inner(output, full_path, name, fields, current_scope, false);
}

fn write_odin_subsection_inner(output: &mut String, full_path: &str, name: &str, fields: &[(String, DynValue)], current_scope: &str, inside_relative: bool) {
    // Determine if we should use relative {.name} or absolute {parent.name} notation
    // Rule: only direct children of absolute headers can use relative notation
    let parent_of_full = full_path.rsplit_once('.').map_or("", |(p, _)| p);
    let can_use_relative = parent_of_full == current_scope && !inside_relative;
    let this_is_relative;
    if can_use_relative {
        // Relative notation: parent matches current scope exactly
        output.push_str("{.");
        output.push_str(name);
        output.push_str("}\n");
        this_is_relative = true;
    } else {
        // Absolute notation: scope has diverged or inside relative header
        output.push('{');
        output.push_str(full_path);
        output.push_str("}\n");
        this_is_relative = false;
    }

    // Phase 1: Output inline items (scalars + leaf chains) in original field order
    for (key, val) in fields {
        match val {
            DynValue::Object(_) if is_pure_leaf_chain(val) => {
                collect_leaf_paths_inner(output, key, val);
            }
            DynValue::Object(_) | DynValue::Array(_) => {}
            _ => write_odin_assignment(output, key, val),
        }
    }

    // Phase 2: Array subsections first, then object subsections (matches TS behavior)
    for (key, val) in fields {
        if let DynValue::Array(items) = val {
            write_odin_array_subsection(output, key, items);
        }
    }
    // Each child is evaluated against this section's scope
    for (key, val) in fields {
        match val {
            DynValue::Object(sub_fields) if !is_pure_leaf_chain(val) => {
                let sub_path = format!("{full_path}.{key}");
                write_odin_subsection_inner(output, &sub_path, key, sub_fields, full_path, this_is_relative);
            }
            _ => {}
        }
    }
}

/// Collect column names from all items of a tabular array.
/// Supports nested objects: `{name: {first, last}}` becomes `["name.first", "name.last"]`.
/// Columns are collected from all items to handle cases where later items have extra fields.
fn collect_tabular_columns_from_items(items: &[DynValue]) -> Vec<String> {
    let mut columns: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for item in items {
        if let DynValue::Object(fields) = item {
            for (key, val) in fields {
                match val {
                    DynValue::Object(sub_fields) => {
                        for (sub_key, _) in sub_fields {
                            let col = format!("{key}.{sub_key}");
                            if seen.insert(col.clone()) {
                                columns.push(col);
                            }
                        }
                    }
                    _ => {
                        if seen.insert(key.clone()) {
                            columns.push(key.clone());
                        }
                    }
                }
            }
        }
    }
    columns
}

/// Format column names using ODIN relative notation.
/// Consecutive columns sharing a parent use `.child` form:
/// `["name.first", "name.last", "license.number", "license.state"]`
/// → `"name.first, .last, license.number, .state"`
fn format_columns_with_relative(columns: &[String]) -> String {
    let mut parts = Vec::new();
    let mut last_parent: Option<&str> = None;
    for col in columns {
        if let Some(dot) = col.find('.') {
            let parent = &col[..dot];
            let child = &col[dot + 1..];
            if last_parent == Some(parent) {
                parts.push(format!(".{child}"));
            } else {
                parts.push(col.clone());
                last_parent = Some(parent);
            }
        } else {
            parts.push(col.clone());
            last_parent = None;
        }
    }
    parts.join(", ")
}

/// Write a single tabular row, looking up values by column path.
/// Handles nested paths (e.g., `name.first` → fields\[`"name"`\]\[`"first"`\]).
/// Null/missing values produce an empty cell.
fn write_tabular_row(output: &mut String, columns: &[String], fields: &[(String, DynValue)]) {
    let mut vals = Vec::new();
    for col in columns {
        let val = if let Some(dot) = col.find('.') {
            let parent = &col[..dot];
            let child = &col[dot + 1..];
            fields.iter()
                .find(|(k, _)| k == parent)
                .and_then(|(_, v)| match v {
                    DynValue::Object(sub) => sub.iter().find(|(k, _)| k == child).map(|(_, v)| v),
                    _ => None,
                })
        } else {
            fields.iter().find(|(k, _)| k == col).map(|(_, v)| v)
        };
        match val {
            None => vals.push(String::new()),
            Some(DynValue::Null) => vals.push("~".to_string()),
            Some(v) => vals.push(odin_value_string(v)),
        }
    }
    // Trim trailing empty values but keep at least the structure
    output.push_str(&vals.join(", "));
    output.push('\n');
}

/// Write an array section at the top level.
fn write_odin_array_section(output: &mut String, key: &str, items: &[DynValue]) {
    if let Some(DynValue::Object(_)) = items.first() {
        let col_names = collect_tabular_columns_from_items(items);
        let formatted_cols = format_columns_with_relative(&col_names);
        let _ = writeln!(output, "{{{key}[] : {formatted_cols}}}");
        for item in items {
            if let DynValue::Object(fields) = item {
                write_tabular_row(output, &col_names, fields);
            }
        }
    } else {
        // Simple value array (or empty)
        let _ = writeln!(output, "{{{key}[] : ~}}");
        for item in items {
            write_odin_value(output, item);
            output.push('\n');
        }
    }
}

/// Write an array as a sub-section.
fn write_odin_array_subsection(output: &mut String, key: &str, items: &[DynValue]) {
    if let Some(DynValue::Object(_)) = items.first() {
        let col_names = collect_tabular_columns_from_items(items);
        let formatted_cols = format_columns_with_relative(&col_names);
        let _ = writeln!(output, "{{.{key}[] : {formatted_cols}}}");
        for item in items {
            if let DynValue::Object(fields) = item {
                write_tabular_row(output, &col_names, fields);
            }
        }
    } else {
        let _ = writeln!(output, "{{.{key}[] : ~}}");
        for item in items {
            write_odin_value(output, item);
            output.push('\n');
        }
    }
}

// ---------------------------------------------------------------------------
// Format Dispatcher
// ---------------------------------------------------------------------------

/// Dispatch to the appropriate formatter by format name.
///
/// Supported formats: `"json"`, `"xml"`, `"csv"`, `"fixed-width"`, `"flat"`,
/// `"properties"` (alias for flat), `"odin"`.
/// Unknown format names fall back to JSON.
pub fn format_output_with_options(
    value: &DynValue,
    format: &str,
    pretty: bool,
    options: &std::collections::HashMap<String, String>,
) -> String {
    match format {
        "xml" => format_xml_with_options(value, if pretty { 2 } else { 0 }, options),
        "csv" => format_csv_with_opts(value, options),
        "fixed-width" => {
            // When called generically, infer field widths from first record
            let widths = infer_field_widths(value);
            format_fixed_width(value, &widths)
        }
        "flat" | "properties" => {
            let style = options.get("style").map_or("kvp", std::string::String::as_str);
            match style {
                "yaml" => format_flat_yaml(value),
                _ => format_flat_kvp(value),
            }
        }
        "odin" => {
            let include_header = options.get("header").is_some_and(|v| v == "true");
            format_odin(value, include_header)
        }
        // "json" and any unrecognized format default to JSON
        _ => {
            if options.contains_key("indent") || options.contains_key("nulls") || options.contains_key("emptyArrays") {
                format_json_with_opts(value, options)
            } else {
                format_json(value, pretty)
            }
        }
    }
}

/// Format a `DynValue` into the given output format with default options.
pub fn format_output(value: &DynValue, format: &str, pretty: bool) -> String {
    format_output_with_options(value, format, pretty, &std::collections::HashMap::new())
}

/// Infer field widths from the first record in an array of objects.
/// Each field gets a default width of 20 characters.
fn infer_field_widths(value: &DynValue) -> Vec<(String, usize)> {
    match value {
        DynValue::Array(arr) => {
            if let Some(DynValue::Object(fields)) = arr.first() {
                fields.iter().map(|(k, _)| (k.clone(), 20)).collect()
            } else {
                Vec::new()
            }
        }
        DynValue::Object(fields) => fields.iter().map(|(k, _)| (k.clone(), 20)).collect(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::transform::DynValue;

    // ---- XML tests ----

    #[test]
    fn xml_simple_object() {
        let val = DynValue::Object(vec![
            ("name".into(), DynValue::String("Alice".into())),
            ("age".into(), DynValue::Integer(30)),
        ]);
        let xml = format_xml(&val, 2);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<root>"));
        assert!(xml.contains("<name>Alice</name>"));
        assert!(xml.contains("<age>30</age>"));
        assert!(xml.contains("</root>"));
    }

    #[test]
    fn xml_single_root_key() {
        let val = DynValue::Object(vec![(
            "person".into(),
            DynValue::Object(vec![
                ("name".into(), DynValue::String("Bob".into())),
            ]),
        )]);
        let xml = format_xml(&val, 2);
        assert!(xml.contains("<person>"));
        assert!(xml.contains("<name>Bob</name>"));
        assert!(xml.contains("</person>"));
        assert!(!xml.contains("<root>"));
    }

    #[test]
    fn xml_escaping() {
        let val = DynValue::Object(vec![(
            "text".into(),
            DynValue::String("A & B <C> \"D\" 'E'".into()),
        )]);
        let xml = format_xml(&val, 0);
        assert!(xml.contains("A &amp; B &lt;C&gt; &quot;D&quot; &apos;E&apos;"));
    }

    #[test]
    fn xml_null_element() {
        let val = DynValue::Object(vec![("empty".into(), DynValue::Null)]);
        let xml = format_xml(&val, 0);
        assert!(xml.contains("<empty odin:type=\"null\"></empty>"));
    }

    #[test]
    fn xml_array_repeated_elements() {
        let val = DynValue::Object(vec![(
            "item".into(),
            DynValue::Array(vec![
                DynValue::String("a".into()),
                DynValue::String("b".into()),
            ]),
        )]);
        let xml = format_xml(&val, 0);
        assert!(xml.contains("<item>a</item>"));
        assert!(xml.contains("<item>b</item>"));
    }

    #[test]
    fn xml_boolean_and_float() {
        let val = DynValue::Object(vec![
            ("active".into(), DynValue::Bool(true)),
            ("score".into(), DynValue::Float(3.14)),
        ]);
        let xml = format_xml(&val, 2);
        assert!(xml.contains("<active>true</active>"));
        assert!(xml.contains("<score>3.14</score>"));
    }

    // ---- Fixed-width tests ----

    #[test]
    fn fixed_width_basic() {
        let val = DynValue::Array(vec![
            DynValue::Object(vec![
                ("name".into(), DynValue::String("Alice".into())),
                ("age".into(), DynValue::Integer(30)),
            ]),
            DynValue::Object(vec![
                ("name".into(), DynValue::String("Bob".into())),
                ("age".into(), DynValue::Integer(25)),
            ]),
        ]);
        let widths = vec![("name".into(), 10), ("age".into(), 5)];
        let result = format_fixed_width(&val, &widths);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 2);
        // "Alice" left-aligned in 10 chars
        assert_eq!(&lines[0][..10], "Alice     ");
        // "30" right-aligned in 5 chars
        assert_eq!(&lines[0][10..15], "   30");
    }

    #[test]
    fn fixed_width_truncation() {
        let val = DynValue::Array(vec![DynValue::Object(vec![(
            "name".into(),
            DynValue::String("Alexander".into()),
        )])]);
        let widths = vec![("name".into(), 5)];
        let result = format_fixed_width(&val, &widths);
        assert!(result.starts_with("Alexa"));
    }

    #[test]
    fn fixed_width_single_record() {
        let val = DynValue::Object(vec![
            ("id".into(), DynValue::Integer(1)),
            ("val".into(), DynValue::String("x".into())),
        ]);
        let widths = vec![("id".into(), 4), ("val".into(), 4)];
        let result = format_fixed_width(&val, &widths);
        assert_eq!(result.lines().count(), 1);
    }

    // ---- Flat tests ----

    #[test]
    fn flat_simple_object() {
        let val = DynValue::Object(vec![
            ("name".into(), DynValue::String("Alice".into())),
            ("age".into(), DynValue::Integer(30)),
        ]);
        let result = format_flat(&val);
        assert!(result.contains("age=30\n"));
        assert!(result.contains("name=Alice\n"));
    }

    #[test]
    fn flat_nested_dot_notation() {
        let val = DynValue::Object(vec![(
            "person".into(),
            DynValue::Object(vec![(
                "address".into(),
                DynValue::Object(vec![
                    ("city".into(), DynValue::String("NYC".into())),
                ]),
            )]),
        )]);
        let result = format_flat(&val);
        assert!(result.contains("person.address.city=NYC\n"));
    }

    #[test]
    fn flat_array_bracket_notation() {
        let val = DynValue::Object(vec![(
            "items".into(),
            DynValue::Array(vec![
                DynValue::String("a".into()),
                DynValue::String("b".into()),
            ]),
        )]);
        let result = format_flat(&val);
        assert!(result.contains("items[0]=a\n"));
        assert!(result.contains("items[1]=b\n"));
    }

    #[test]
    fn flat_skips_nulls() {
        let val = DynValue::Object(vec![
            ("a".into(), DynValue::String("yes".into())),
            ("b".into(), DynValue::Null),
            ("c".into(), DynValue::Integer(1)),
        ]);
        let result = format_flat(&val);
        assert!(result.contains("a=yes\n"));
        assert!(!result.contains("b="));
        assert!(result.contains("c=1\n"));
    }

    #[test]
    fn flat_boolean_and_float() {
        let val = DynValue::Object(vec![
            ("flag".into(), DynValue::Bool(false)),
            ("pi".into(), DynValue::Float(3.14)),
        ]);
        let result = format_flat(&val);
        assert!(result.contains("flag=false\n"));
        assert!(result.contains("pi=3.14\n"));
    }

    // ---- ODIN tests ----

    #[test]
    fn odin_flat_fields() {
        let val = DynValue::Object(vec![
            ("name".into(), DynValue::String("Alice".into())),
            ("age".into(), DynValue::Integer(30)),
        ]);
        let result = format_odin(&val, false);
        assert!(!result.contains("{$}"));
        assert!(result.contains("name = \"Alice\"\n"));
        assert!(result.contains("age = ##30\n"));
        // With header
        let result_h = format_odin(&val, true);
        assert!(result_h.starts_with("{$}\nodin = \"1.0.0\"\n"));
    }

    #[test]
    fn odin_sections() {
        let val = DynValue::Object(vec![(
            "customer".into(),
            DynValue::Object(vec![
                ("name".into(), DynValue::String("Bob".into())),
                ("active".into(), DynValue::Bool(true)),
            ]),
        )]);
        let result = format_odin(&val, false);
        assert!(result.contains("{customer}\n"));
        assert!(result.contains("name = \"Bob\"\n"));
        assert!(result.contains("active = ?true\n"));
    }

    #[test]
    fn odin_null_and_float() {
        let val = DynValue::Object(vec![
            ("missing".into(), DynValue::Null),
            ("rate".into(), DynValue::Float(9.99)),
        ]);
        let result = format_odin(&val, false);
        assert!(result.contains("missing = ~\n"));
        assert!(result.contains("rate = #9.99\n"));
    }

    #[test]
    fn odin_array_sections() {
        let val = DynValue::Object(vec![(
            "item".into(),
            DynValue::Array(vec![
                DynValue::Object(vec![("id".into(), DynValue::Integer(1))]),
                DynValue::Object(vec![("id".into(), DynValue::Integer(2))]),
            ]),
        )]);
        let result = format_odin(&val, false);
        // Should produce tabular format: {.item[] : id}
        assert!(result.contains("item[]"));
        assert!(result.contains("##1"));
        assert!(result.contains("##2"));
    }

    // ---- format_output dispatcher tests ----

    #[test]
    fn dispatch_json() {
        let val = DynValue::Object(vec![("x".into(), DynValue::Integer(1))]);
        let result = format_output(&val, "json", false);
        assert!(result.contains("\"x\":1"));
    }

    #[test]
    fn dispatch_xml() {
        let val = DynValue::Object(vec![("x".into(), DynValue::Integer(1))]);
        let result = format_output(&val, "xml", true);
        assert!(result.contains("<x>1</x>"));
    }

    #[test]
    fn dispatch_csv() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("a".into(), DynValue::Integer(1)),
        ])]);
        let result = format_output(&val, "csv", false);
        assert!(result.contains("a\n1\n"));
    }

    #[test]
    fn dispatch_flat() {
        let val = DynValue::Object(vec![("k".into(), DynValue::String("v".into()))]);
        let result = format_output(&val, "flat", false);
        assert!(result.contains("k=v\n"));
    }

    #[test]
    fn dispatch_properties_alias() {
        let val = DynValue::Object(vec![("k".into(), DynValue::String("v".into()))]);
        let result = format_output(&val, "properties", false);
        assert!(result.contains("k=v\n"));
    }

    #[test]
    fn dispatch_odin() {
        let val = DynValue::Object(vec![("x".into(), DynValue::Integer(1))]);
        let result = format_output(&val, "odin", false);
        assert!(!result.contains("{$}"));
        assert!(result.contains("x = ##1"));
    }

    #[test]
    fn dispatch_fixed_width() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("name".into(), DynValue::String("Hi".into())),
        ])]);
        let result = format_output(&val, "fixed-width", false);
        assert!(result.contains("Hi"));
        // Default width is 20, so "Hi" should be padded
        assert!(result.len() > 2);
    }

    #[test]
    fn dispatch_unknown_falls_back_to_json() {
        let val = DynValue::Object(vec![("x".into(), DynValue::Integer(1))]);
        let result = format_output(&val, "yaml", false);
        // Falls back to JSON
        assert!(result.contains("\"x\":1"));
    }

    #[test]
    fn odin_string_with_embedded_quotes() {
        let s = DynValue::String("Product \"Quoted\"".to_string());
        let result = odin_value_string(&s);
        assert_eq!(result, "\"Product \\\"Quoted\\\"\"");
    }
}

#[cfg(test)]
mod extended_tests {
    use super::*;
    use crate::types::transform::DynValue;

    // ===================================================================
    // JSON formatter extended tests
    // ===================================================================

    #[test]
    fn json_compact_object() {
        let val = DynValue::Object(vec![
            ("a".into(), DynValue::Integer(1)),
            ("b".into(), DynValue::String("two".into())),
        ]);
        let result = format_json(&val, false);
        assert_eq!(result, r#"{"a":1,"b":"two"}"#);
    }

    #[test]
    fn json_pretty_object() {
        let val = DynValue::Object(vec![
            ("x".into(), DynValue::Integer(1)),
        ]);
        let result = format_json(&val, true);
        assert!(result.contains("  \"x\": 1"));
        assert!(result.contains("{\n"));
        assert!(result.contains("\n}"));
    }

    #[test]
    fn json_nested_objects() {
        let val = DynValue::Object(vec![(
            "outer".into(),
            DynValue::Object(vec![
                ("inner".into(), DynValue::Integer(42)),
            ]),
        )]);
        let result = format_json(&val, false);
        assert_eq!(result, r#"{"outer":{"inner":42}}"#);
    }

    #[test]
    fn json_array_simple() {
        let val = DynValue::Array(vec![
            DynValue::Integer(1),
            DynValue::Integer(2),
            DynValue::Integer(3),
        ]);
        let result = format_json(&val, false);
        assert_eq!(result, "[1,2,3]");
    }

    #[test]
    fn json_array_pretty() {
        let val = DynValue::Array(vec![DynValue::Integer(1), DynValue::Integer(2)]);
        let result = format_json(&val, true);
        assert!(result.contains("[\n"));
        assert!(result.contains("  1"));
        assert!(result.contains("  2"));
    }

    #[test]
    fn json_special_chars_in_string() {
        let val = DynValue::String("line1\nline2\ttab\\backslash\"quote".into());
        let result = format_json(&val, false);
        assert_eq!(result, r#""line1\nline2\ttab\\backslash\"quote""#);
    }

    #[test]
    fn json_unicode_passthrough() {
        let val = DynValue::String("cafe\u{0301}".into());
        let result = format_json(&val, false);
        assert!(result.contains("caf"));
    }

    #[test]
    fn json_null_value() {
        assert_eq!(format_json(&DynValue::Null, false), "null");
    }

    #[test]
    fn json_bool_values() {
        assert_eq!(format_json(&DynValue::Bool(true), false), "true");
        assert_eq!(format_json(&DynValue::Bool(false), false), "false");
    }

    #[test]
    fn json_integer_value() {
        assert_eq!(format_json(&DynValue::Integer(0), false), "0");
        assert_eq!(format_json(&DynValue::Integer(-99), false), "-99");
        assert_eq!(format_json(&DynValue::Integer(i64::MAX), false), i64::MAX.to_string());
    }

    #[test]
    fn json_float_value() {
        let result = format_json(&DynValue::Float(3.14), false);
        assert!(result.starts_with("3.14"));
    }

    #[test]
    fn json_float_infinity_becomes_null() {
        let result = format_json(&DynValue::Float(f64::INFINITY), false);
        assert_eq!(result, "null");
    }

    #[test]
    fn json_float_nan_becomes_null() {
        let result = format_json(&DynValue::Float(f64::NAN), false);
        assert_eq!(result, "null");
    }

    #[test]
    fn json_empty_object() {
        assert_eq!(format_json(&DynValue::Object(Vec::new()), false), "{}");
    }

    #[test]
    fn json_empty_array() {
        assert_eq!(format_json(&DynValue::Array(Vec::new()), false), "[]");
    }

    #[test]
    fn json_currency_value() {
        let val = DynValue::Currency(99.99, 2, Some("USD".into()));
        let result = format_json(&val, false);
        assert!(result.contains("99.99"));
    }

    #[test]
    fn json_reference_as_string() {
        let val = DynValue::Reference("parties[0]".into());
        let result = format_json(&val, false);
        assert_eq!(result, r#""parties[0]""#);
    }

    #[test]
    fn json_binary_as_string() {
        let val = DynValue::Binary("SGVsbG8=".into());
        let result = format_json(&val, false);
        assert_eq!(result, r#""SGVsbG8=""#);
    }

    #[test]
    fn json_date_as_string() {
        let val = DynValue::Date("2024-01-15".into());
        let result = format_json(&val, false);
        assert_eq!(result, r#""2024-01-15""#);
    }

    #[test]
    fn json_deeply_nested() {
        let mut val = DynValue::String("deep".into());
        for i in (0..5).rev() {
            val = DynValue::Object(vec![(format!("l{i}"), val)]);
        }
        let result = format_json(&val, false);
        assert!(result.contains("\"deep\""));
        assert!(result.contains("\"l0\""));
    }

    #[test]
    fn json_control_chars_escaped() {
        let val = DynValue::String("\x00\x01\x1f".into());
        let result = format_json(&val, false);
        assert!(result.contains("\\u0000"));
        assert!(result.contains("\\u0001"));
        assert!(result.contains("\\u001f"));
    }

    #[test]
    fn json_floatraw_value() {
        let val = DynValue::FloatRaw("123456789012345678.90".into());
        let result = format_json(&val, false);
        assert_eq!(result, "123456789012345678.90");
    }

    #[test]
    fn json_percent_value() {
        let val = DynValue::Percent(85.5);
        let result = format_json(&val, false);
        assert!(result.contains("85.5"));
    }

    // ===================================================================
    // CSV formatter extended tests
    // ===================================================================

    #[test]
    fn csv_basic_headers_and_rows() {
        let val = DynValue::Array(vec![
            DynValue::Object(vec![
                ("name".into(), DynValue::String("Alice".into())),
                ("age".into(), DynValue::Integer(30)),
            ]),
            DynValue::Object(vec![
                ("name".into(), DynValue::String("Bob".into())),
                ("age".into(), DynValue::Integer(25)),
            ]),
        ]);
        let result = format_csv(&val);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "name,age");
        assert_eq!(lines[1], "Alice,30");
        assert_eq!(lines[2], "Bob,25");
    }

    #[test]
    fn csv_quoting_commas() {
        let val = DynValue::Array(vec![DynValue::Object(vec![(
            "msg".into(),
            DynValue::String("hello, world".into()),
        )])]);
        let result = format_csv(&val);
        assert!(result.contains("\"hello, world\""));
    }

    #[test]
    fn csv_quoting_quotes() {
        let val = DynValue::Array(vec![DynValue::Object(vec![(
            "msg".into(),
            DynValue::String("say \"hi\"".into()),
        )])]);
        let result = format_csv(&val);
        assert!(result.contains("\"say \"\"hi\"\"\""));
    }

    #[test]
    fn csv_quoting_newlines() {
        let val = DynValue::Array(vec![DynValue::Object(vec![(
            "msg".into(),
            DynValue::String("line1\nline2".into()),
        )])]);
        let result = format_csv(&val);
        assert!(result.contains("\"line1\nline2\""));
    }

    #[test]
    fn csv_null_as_empty() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("a".into(), DynValue::String("x".into())),
            ("b".into(), DynValue::Null),
        ])]);
        let result = format_csv(&val);
        assert!(result.contains("x,\n"));
    }

    #[test]
    fn csv_bool_values() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("t".into(), DynValue::Bool(true)),
            ("f".into(), DynValue::Bool(false)),
        ])]);
        let result = format_csv(&val);
        assert!(result.contains("true,false"));
    }

    #[test]
    fn csv_float_values() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("pi".into(), DynValue::Float(3.14)),
        ])]);
        let result = format_csv(&val);
        assert!(result.contains("3.14"));
    }

    #[test]
    fn csv_unwrap_single_key_array() {
        let val = DynValue::Object(vec![(
            "items".into(),
            DynValue::Array(vec![DynValue::Object(vec![
                ("id".into(), DynValue::Integer(1)),
            ])]),
        )]);
        let result = format_csv(&val);
        assert!(result.contains("id\n1\n"));
    }

    #[test]
    fn csv_empty_array() {
        let val = DynValue::Array(Vec::new());
        let result = format_csv(&val);
        assert_eq!(result, "");
    }

    #[test]
    fn csv_currency_value() {
        let val = DynValue::Array(vec![DynValue::Object(vec![(
            "price".into(),
            DynValue::Currency(19.99, 2, Some("USD".into())),
        )])]);
        let result = format_csv(&val);
        assert!(result.contains("19.99"));
    }

    // ===================================================================
    // XML formatter extended tests
    // ===================================================================

    #[test]
    fn xml_declaration_present() {
        let val = DynValue::Object(vec![("x".into(), DynValue::Integer(1))]);
        let result = format_xml(&val, 0);
        assert!(result.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    }

    #[test]
    fn xml_nested_objects() {
        let val = DynValue::Object(vec![(
            "person".into(),
            DynValue::Object(vec![
                ("address".into(), DynValue::Object(vec![
                    ("city".into(), DynValue::String("NYC".into())),
                ])),
            ]),
        )]);
        let result = format_xml(&val, 2);
        assert!(result.contains("<person>"));
        assert!(result.contains("<address>"));
        assert!(result.contains("<city>NYC</city>"));
        assert!(result.contains("</address>"));
        assert!(result.contains("</person>"));
    }

    #[test]
    fn xml_pretty_indentation() {
        let val = DynValue::Object(vec![(
            "root".into(),
            DynValue::Object(vec![
                ("child".into(), DynValue::String("val".into())),
            ]),
        )]);
        let result = format_xml(&val, 2);
        assert!(result.contains("  <child>val</child>"));
    }

    #[test]
    fn xml_special_chars_escaped() {
        let val = DynValue::Object(vec![(
            "msg".into(),
            DynValue::String("a & b < c > d \"e\" 'f'".into()),
        )]);
        let xml = format_xml(&val, 0);
        assert!(xml.contains("&amp;"));
        assert!(xml.contains("&lt;"));
        assert!(xml.contains("&gt;"));
        assert!(xml.contains("&quot;"));
        assert!(xml.contains("&apos;"));
    }

    #[test]
    fn xml_integer_element() {
        let val = DynValue::Object(vec![("count".into(), DynValue::Integer(42))]);
        let xml = format_xml(&val, 0);
        assert!(xml.contains("<count>42</count>"));
    }

    #[test]
    fn xml_float_element() {
        let val = DynValue::Object(vec![("pi".into(), DynValue::Float(3.14))]);
        let xml = format_xml(&val, 0);
        assert!(xml.contains("<pi>3.14</pi>"));
    }

    #[test]
    fn xml_null_self_closing() {
        let val = DynValue::Object(vec![("empty".into(), DynValue::Null)]);
        let xml = format_xml(&val, 0);
        assert!(xml.contains("<empty odin:type=\"null\"></empty>"));
    }

    #[test]
    fn xml_array_repeats_elements() {
        let val = DynValue::Object(vec![(
            "root".into(),
            DynValue::Object(vec![(
                "item".into(),
                DynValue::Array(vec![
                    DynValue::String("a".into()),
                    DynValue::String("b".into()),
                    DynValue::String("c".into()),
                ]),
            )]),
        )]);
        let xml = format_xml(&val, 0);
        let count = xml.matches("<item>").count();
        assert_eq!(count, 3);
    }

    #[test]
    fn xml_multi_key_root() {
        let val = DynValue::Object(vec![
            ("a".into(), DynValue::Integer(1)),
            ("b".into(), DynValue::Integer(2)),
        ]);
        let xml = format_xml(&val, 0);
        // With multiple top-level keys, wraps in <root>
        assert!(xml.contains("<root>"));
    }

    #[test]
    fn xml_bool_element() {
        let val = DynValue::Object(vec![
            ("yes".into(), DynValue::Bool(true)),
            ("no".into(), DynValue::Bool(false)),
        ]);
        let xml = format_xml(&val, 0);
        assert!(xml.contains("<yes>true</yes>"));
        assert!(xml.contains("<no>false</no>"));
    }

    #[test]
    fn xml_empty_string_element() {
        let val = DynValue::Object(vec![("s".into(), DynValue::String(String::new()))]);
        let xml = format_xml(&val, 0);
        assert!(xml.contains("<s></s>"));
    }

    // ===================================================================
    // Fixed-width formatter extended tests
    // ===================================================================

    #[test]
    fn fixed_width_string_left_aligned() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("name".into(), DynValue::String("Hi".into())),
        ])]);
        let widths = vec![("name".into(), 10)];
        let result = format_fixed_width(&val, &widths);
        assert_eq!(result.trim_end_matches('\n'), "Hi        ");
    }

    #[test]
    fn fixed_width_number_right_aligned() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("num".into(), DynValue::Integer(42)),
        ])]);
        let widths = vec![("num".into(), 10)];
        let result = format_fixed_width(&val, &widths);
        assert_eq!(result.trim_end_matches('\n'), "        42");
    }

    #[test]
    fn fixed_width_truncation_string() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("name".into(), DynValue::String("VeryLongName".into())),
        ])]);
        let widths = vec![("name".into(), 5)];
        let result = format_fixed_width(&val, &widths);
        assert_eq!(result.trim_end_matches('\n'), "VeryL");
    }

    #[test]
    fn fixed_width_truncation_number() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("num".into(), DynValue::Integer(1234567890)),
        ])]);
        let widths = vec![("num".into(), 5)];
        let result = format_fixed_width(&val, &widths);
        assert_eq!(result.trim_end_matches('\n'), "12345");
    }

    #[test]
    fn fixed_width_multiple_records() {
        let val = DynValue::Array(vec![
            DynValue::Object(vec![("id".into(), DynValue::Integer(1))]),
            DynValue::Object(vec![("id".into(), DynValue::Integer(2))]),
            DynValue::Object(vec![("id".into(), DynValue::Integer(3))]),
        ]);
        let widths = vec![("id".into(), 5)];
        let result = format_fixed_width(&val, &widths);
        assert_eq!(result.lines().count(), 3);
    }

    #[test]
    fn fixed_width_missing_field() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("a".into(), DynValue::String("x".into())),
        ])]);
        let widths = vec![("a".into(), 5), ("missing".into(), 5)];
        let result = format_fixed_width(&val, &widths);
        // missing field should produce spaces
        assert_eq!(result.trim_end_matches('\n'), "x         ");
    }

    #[test]
    fn fixed_width_bool_value() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("flag".into(), DynValue::Bool(true)),
        ])]);
        let widths = vec![("flag".into(), 10)];
        let result = format_fixed_width(&val, &widths);
        assert_eq!(result.trim_end_matches('\n'), "true      ");
    }

    #[test]
    fn fixed_width_null_empty() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("x".into(), DynValue::Null),
        ])]);
        let widths = vec![("x".into(), 5)];
        let result = format_fixed_width(&val, &widths);
        assert_eq!(result.trim_end_matches('\n'), "     ");
    }

    #[test]
    fn fixed_width_float_right_aligned() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("f".into(), DynValue::Float(3.14)),
        ])]);
        let widths = vec![("f".into(), 10)];
        let result = format_fixed_width(&val, &widths);
        // Float should be right-aligned
        let line = result.trim_end_matches('\n');
        assert!(line.ends_with("3.14"));
        assert!(line.starts_with(' '));
    }

    #[test]
    fn fixed_width_single_object_not_array() {
        let val = DynValue::Object(vec![
            ("name".into(), DynValue::String("Test".into())),
        ]);
        let widths = vec![("name".into(), 10)];
        let result = format_fixed_width(&val, &widths);
        assert_eq!(result.lines().count(), 1);
        assert_eq!(result.trim_end_matches('\n'), "Test      ");
    }

    // ===================================================================
    // Flat formatter extended tests
    // ===================================================================

    #[test]
    fn flat_simple_scalars() {
        let val = DynValue::Object(vec![
            ("name".into(), DynValue::String("Alice".into())),
            ("age".into(), DynValue::Integer(30)),
            ("active".into(), DynValue::Bool(true)),
            ("pi".into(), DynValue::Float(3.14)),
        ]);
        let result = format_flat(&val);
        assert!(result.contains("active=true\n"));
        assert!(result.contains("age=30\n"));
        assert!(result.contains("name=Alice\n"));
        assert!(result.contains("pi=3.14\n"));
    }

    #[test]
    fn flat_nested_dot_notation() {
        let val = DynValue::Object(vec![(
            "a".into(),
            DynValue::Object(vec![(
                "b".into(),
                DynValue::Object(vec![
                    ("c".into(), DynValue::String("deep".into())),
                ]),
            )]),
        )]);
        let result = format_flat(&val);
        assert!(result.contains("a.b.c=deep\n"));
    }

    #[test]
    fn flat_array_bracket_notation() {
        let val = DynValue::Object(vec![(
            "items".into(),
            DynValue::Array(vec![
                DynValue::Integer(10),
                DynValue::Integer(20),
                DynValue::Integer(30),
            ]),
        )]);
        let result = format_flat(&val);
        assert!(result.contains("items[0]=10\n"));
        assert!(result.contains("items[1]=20\n"));
        assert!(result.contains("items[2]=30\n"));
    }

    #[test]
    fn flat_nulls_skipped() {
        let val = DynValue::Object(vec![
            ("present".into(), DynValue::String("yes".into())),
            ("missing".into(), DynValue::Null),
        ]);
        let result = format_flat(&val);
        assert!(result.contains("present=yes"));
        assert!(!result.contains("missing"));
    }

    #[test]
    fn flat_sorted_alphabetically() {
        let val = DynValue::Object(vec![
            ("z".into(), DynValue::Integer(1)),
            ("a".into(), DynValue::Integer(2)),
            ("m".into(), DynValue::Integer(3)),
        ]);
        let result = format_flat(&val);
        let lines: Vec<&str> = result.lines().collect();
        assert!(lines[0].starts_with("a="));
        assert!(lines[1].starts_with("m="));
        assert!(lines[2].starts_with("z="));
    }

    #[test]
    fn flat_nested_array_of_objects() {
        let val = DynValue::Object(vec![(
            "people".into(),
            DynValue::Array(vec![
                DynValue::Object(vec![("name".into(), DynValue::String("Alice".into()))]),
                DynValue::Object(vec![("name".into(), DynValue::String("Bob".into()))]),
            ]),
        )]);
        let result = format_flat(&val);
        assert!(result.contains("people[0].name=Alice\n"));
        assert!(result.contains("people[1].name=Bob\n"));
    }

    #[test]
    fn flat_empty_object() {
        let val = DynValue::Object(Vec::new());
        let result = format_flat(&val);
        assert_eq!(result, "");
    }

    #[test]
    fn flat_reference_value() {
        let val = DynValue::Object(vec![
            ("ref".into(), DynValue::Reference("path.to.thing".into())),
        ]);
        let result = format_flat(&val);
        assert!(result.contains("ref=path.to.thing\n"));
    }

    // ===================================================================
    // ODIN formatter extended tests
    // ===================================================================

    #[test]
    fn odin_string_type() {
        let val = DynValue::Object(vec![
            ("name".into(), DynValue::String("Alice".into())),
        ]);
        let result = format_odin(&val, false);
        assert!(result.contains("name = \"Alice\"\n"));
    }

    #[test]
    fn odin_integer_type() {
        let val = DynValue::Object(vec![
            ("count".into(), DynValue::Integer(42)),
        ]);
        let result = format_odin(&val, false);
        assert!(result.contains("count = ##42\n"));
    }

    #[test]
    fn odin_float_type() {
        let val = DynValue::Object(vec![
            ("pi".into(), DynValue::Float(3.14)),
        ]);
        let result = format_odin(&val, false);
        assert!(result.contains("pi = #3.14"));
    }

    #[test]
    fn odin_bool_type() {
        let val = DynValue::Object(vec![
            ("yes".into(), DynValue::Bool(true)),
            ("no".into(), DynValue::Bool(false)),
        ]);
        let result = format_odin(&val, false);
        assert!(result.contains("yes = ?true\n"));
        assert!(result.contains("no = ?false\n"));
    }

    #[test]
    fn odin_null_type() {
        let val = DynValue::Object(vec![
            ("empty".into(), DynValue::Null),
        ]);
        let result = format_odin(&val, false);
        assert!(result.contains("empty = ~\n"));
    }

    #[test]
    fn odin_reference_type() {
        let val = odin_value_string(&DynValue::Reference("parties[0]".into()));
        assert_eq!(val, "@parties[0]");
    }

    #[test]
    fn odin_binary_type() {
        let val = odin_value_string(&DynValue::Binary("SGVsbG8=".into()));
        assert_eq!(val, "^SGVsbG8=");
    }

    #[test]
    fn odin_currency_type() {
        let val = odin_value_string(&DynValue::Currency(99.99, 2, Some("USD".into())));
        assert_eq!(val, "#$99.99:USD");
    }

    #[test]
    fn odin_currency_no_code() {
        let val = odin_value_string(&DynValue::Currency(19.95, 2, None));
        assert_eq!(val, "#$19.95");
    }

    #[test]
    fn odin_percent_type() {
        let val = odin_value_string(&DynValue::Percent(85.5));
        assert_eq!(val, "#%85.5");
    }

    #[test]
    fn odin_date_type() {
        let val = odin_value_string(&DynValue::Date("2024-01-15".into()));
        assert_eq!(val, "2024-01-15");
    }

    #[test]
    fn odin_time_type_no_prefix() {
        let val = odin_value_string(&DynValue::Time("10:30:00".into()));
        assert_eq!(val, "T10:30:00");
    }

    #[test]
    fn odin_time_type_with_prefix() {
        let val = odin_value_string(&DynValue::Time("T10:30:00".into()));
        assert_eq!(val, "T10:30:00");
    }

    #[test]
    fn odin_duration_type() {
        let val = odin_value_string(&DynValue::Duration("P1Y2M3D".into()));
        assert_eq!(val, "P1Y2M3D");
    }

    #[test]
    fn odin_header_present() {
        let val = DynValue::Object(vec![("x".into(), DynValue::Integer(1))]);
        let result = format_odin(&val, true);
        assert!(result.starts_with("{$}\nodin = \"1.0.0\"\n"));
    }

    #[test]
    fn odin_header_absent() {
        let val = DynValue::Object(vec![("x".into(), DynValue::Integer(1))]);
        let result = format_odin(&val, false);
        assert!(!result.contains("{$}"));
    }

    #[test]
    fn odin_section_for_nested_object() {
        let val = DynValue::Object(vec![(
            "person".into(),
            DynValue::Object(vec![
                ("name".into(), DynValue::String("Alice".into())),
                ("age".into(), DynValue::Integer(30)),
            ]),
        )]);
        let result = format_odin(&val, false);
        assert!(result.contains("{person}\n"));
        assert!(result.contains("name = \"Alice\"\n"));
        assert!(result.contains("age = ##30\n"));
    }

    #[test]
    fn odin_string_with_special_chars() {
        let val = odin_value_string(&DynValue::String("line1\nline2\ttab\\slash\"quote".into()));
        assert_eq!(val, "\"line1\\nline2\\ttab\\\\slash\\\"quote\"");
    }

    #[test]
    fn odin_float_whole_number() {
        // Float with no fractional part uses # prefix (preserving Float type)
        let val = odin_value_string(&DynValue::Float(42.0));
        assert_eq!(val, "#42");
    }

    #[test]
    fn odin_float_infinity() {
        let val = odin_value_string(&DynValue::Float(f64::INFINITY));
        assert_eq!(val, "~");
    }

    #[test]
    fn odin_floatraw_type() {
        let val = odin_value_string(&DynValue::FloatRaw("1234567890.123456789".into()));
        assert_eq!(val, "#1234567890.123456789");
    }

    #[test]
    fn odin_currencyraw_with_code() {
        let val = odin_value_string(&DynValue::CurrencyRaw("100.00".into(), 2, Some("EUR".into())));
        assert_eq!(val, "#$100.00:EUR");
    }

    #[test]
    fn odin_currencyraw_no_code() {
        let val = odin_value_string(&DynValue::CurrencyRaw("50.00".into(), 2, None));
        assert_eq!(val, "#$50.00");
    }

    #[test]
    fn odin_negative_integer() {
        let val = odin_value_string(&DynValue::Integer(-99));
        assert_eq!(val, "##-99");
    }

    #[test]
    fn odin_array_as_tabular() {
        let val = DynValue::Object(vec![(
            "items".into(),
            DynValue::Array(vec![
                DynValue::Object(vec![
                    ("id".into(), DynValue::Integer(1)),
                    ("name".into(), DynValue::String("a".into())),
                ]),
                DynValue::Object(vec![
                    ("id".into(), DynValue::Integer(2)),
                    ("name".into(), DynValue::String("b".into())),
                ]),
            ]),
        )]);
        let result = format_odin(&val, false);
        assert!(result.contains("items[]"));
        assert!(result.contains("##1"));
        assert!(result.contains("##2"));
    }

    // ===================================================================
    // format_output dispatcher extended tests
    // ===================================================================

    #[test]
    fn dispatch_json_pretty() {
        let val = DynValue::Object(vec![("x".into(), DynValue::Integer(1))]);
        let result = format_output(&val, "json", true);
        assert!(result.contains("  \"x\": 1"));
    }

    #[test]
    fn dispatch_odin_with_options() {
        let val = DynValue::Object(vec![("x".into(), DynValue::Integer(1))]);
        let mut opts = std::collections::HashMap::new();
        opts.insert("header".into(), "true".into());
        let result = format_output_with_options(&val, "odin", false, &opts);
        assert!(result.contains("{$}"));
    }

    #[test]
    fn dispatch_flat_yaml_style() {
        let val = DynValue::Object(vec![
            ("name".into(), DynValue::String("Alice".into())),
        ]);
        let mut opts = std::collections::HashMap::new();
        opts.insert("style".into(), "yaml".into());
        let result = format_output_with_options(&val, "flat", false, &opts);
        assert!(result.contains("name:"));
    }

    #[test]
    fn dispatch_unknown_format_falls_back_json() {
        let val = DynValue::Integer(42);
        let result = format_output(&val, "unknown_format", false);
        assert_eq!(result, "42");
    }

    // ===================================================================
    // Edge cases
    // ===================================================================

    #[test]
    fn json_nested_empty_containers() {
        let val = DynValue::Object(vec![
            ("empty_arr".into(), DynValue::Array(Vec::new())),
            ("empty_obj".into(), DynValue::Object(Vec::new())),
        ]);
        let result = format_json(&val, false);
        assert_eq!(result, r#"{"empty_arr":[],"empty_obj":{}}"#);
    }

    #[test]
    fn csv_non_array_produces_empty() {
        let val = DynValue::String("not an array".into());
        let result = format_csv(&val);
        assert_eq!(result, "");
    }

    #[test]
    fn fixed_width_non_record_produces_empty() {
        let val = DynValue::String("not a record".into());
        let widths = vec![("x".into(), 10)];
        let result = format_fixed_width(&val, &widths);
        assert_eq!(result, "");
    }

    #[test]
    fn xml_escape_all_special() {
        let escaped = xml_escape("& < > \" '");
        assert_eq!(escaped, "&amp; &lt; &gt; &quot; &apos;");
    }

    #[test]
    fn odin_empty_string() {
        let val = odin_value_string(&DynValue::String(String::new()));
        assert_eq!(val, "\"\"");
    }

    #[test]
    fn infer_field_widths_from_array() {
        let val = DynValue::Array(vec![DynValue::Object(vec![
            ("a".into(), DynValue::Integer(1)),
            ("b".into(), DynValue::Integer(2)),
        ])]);
        let widths = infer_field_widths(&val);
        assert_eq!(widths.len(), 2);
        assert_eq!(widths[0], ("a".into(), 20));
        assert_eq!(widths[1], ("b".into(), 20));
    }

    #[test]
    fn infer_field_widths_from_scalar() {
        let val = DynValue::Integer(42);
        let widths = infer_field_widths(&val);
        assert!(widths.is_empty());
    }

    #[test]
    fn format_float_whole_number() {
        assert_eq!(format_float(42.0), "42");
        assert_eq!(format_float(0.0), "0");
    }

    #[test]
    fn format_float_with_fraction() {
        let result = format_float(3.14);
        assert!(result.starts_with("3.14"));
    }

    #[test]
    fn timestamp_normalization_utc() {
        let result = normalize_timestamp("2024-12-15T10:30:00Z");
        assert_eq!(result, "2024-12-15T10:30:00.000Z");
    }

    #[test]
    fn timestamp_normalization_with_offset() {
        let result = normalize_timestamp("2024-12-15T10:30:00+05:30");
        assert_eq!(result, "2024-12-15T05:00:00.000Z");
    }

    #[test]
    fn timestamp_normalization_already_millis() {
        let result = normalize_timestamp("2024-12-15T10:30:00.500Z");
        assert_eq!(result, "2024-12-15T10:30:00.500Z");
    }
}
