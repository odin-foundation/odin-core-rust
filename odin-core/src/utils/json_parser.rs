//! JSON parser and serializer for the transform engine.
//!
//! Parses JSON text into [`DynValue`] using `serde_json` and serializes [`DynValue`] back to JSON.

use std::fmt::Write as _;

use crate::types::transform::DynValue;

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a JSON string into a [`DynValue`].
pub fn parse_json(input: &str) -> Result<DynValue, String> {
    let v: serde_json::Value = serde_json::from_str(input).map_err(|e| e.to_string())?;
    Ok(DynValue::from_json_value(v))
}

// ---------------------------------------------------------------------------
// Serializer
// ---------------------------------------------------------------------------

/// Serialize a [`DynValue`] to a JSON string.
///
/// When `pretty` is `true`, the output uses 2-space indentation and newlines.
/// When `false`, the output is compact with no extra whitespace.
pub fn to_json(value: &DynValue, pretty: bool) -> String {
    let mut buf = String::new();
    write_value(&mut buf, value, pretty, 0);
    buf
}

fn write_value(buf: &mut String, value: &DynValue, pretty: bool, depth: usize) {
    match value {
        DynValue::Null => buf.push_str("null"),
        DynValue::Bool(true) => buf.push_str("true"),
        DynValue::Bool(false) => buf.push_str("false"),
        DynValue::Integer(n) => { let mut b = itoa::Buffer::new(); buf.push_str(b.format(*n)); }
        DynValue::Float(n) | DynValue::Currency(n, _, _) | DynValue::Percent(n) => write_float(buf, *n),
        DynValue::FloatRaw(s) | DynValue::CurrencyRaw(s, _, _) => buf.push_str(s),
        DynValue::String(s) | DynValue::Reference(s) | DynValue::Binary(s)
        | DynValue::Date(s) | DynValue::Timestamp(s) | DynValue::Time(s)
        | DynValue::Duration(s) => write_json_string(buf, s),
        DynValue::Array(items) => write_array(buf, items, pretty, depth),
        DynValue::Object(entries) => write_object(buf, entries, pretty, depth),
    }
}

fn write_float(buf: &mut String, n: f64) {
    if n.is_infinite() || n.is_nan() {
        buf.push_str("null");
    } else {
        let mut rbuf = ryu::Buffer::new();
        let s = rbuf.format(n);
        buf.push_str(s);
        if !s.contains('.') && !s.contains('e') && !s.contains('E') {
            buf.push_str(".0");
        }
    }
}

fn write_json_string(buf: &mut String, s: &str) {
    buf.push('"');
    for ch in s.chars() {
        match ch {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\u{08}' => buf.push_str("\\b"),
            '\u{0C}' => buf.push_str("\\f"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(buf, "\\u{:04x}", c as u32);
            }
            c => buf.push(c),
        }
    }
    buf.push('"');
}

fn write_array(buf: &mut String, items: &[DynValue], pretty: bool, depth: usize) {
    if items.is_empty() {
        buf.push_str("[]");
        return;
    }
    buf.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            buf.push(',');
        }
        if pretty {
            buf.push('\n');
            push_indent(buf, depth + 1);
        }
        write_value(buf, item, pretty, depth + 1);
    }
    if pretty {
        buf.push('\n');
        push_indent(buf, depth);
    }
    buf.push(']');
}

fn write_object(buf: &mut String, entries: &[(String, DynValue)], pretty: bool, depth: usize) {
    if entries.is_empty() {
        buf.push_str("{}");
        return;
    }
    buf.push('{');
    for (i, (key, val)) in entries.iter().enumerate() {
        if i > 0 {
            buf.push(',');
        }
        if pretty {
            buf.push('\n');
            push_indent(buf, depth + 1);
        }
        write_json_string(buf, key);
        buf.push(':');
        if pretty {
            buf.push(' ');
        }
        write_value(buf, val, pretty, depth + 1);
    }
    if pretty {
        buf.push('\n');
        push_indent(buf, depth);
    }
    buf.push('}');
}

fn push_indent(buf: &mut String, depth: usize) {
    for _ in 0..depth {
        buf.push_str("  ");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse: primitives --------------------------------------------------

    #[test]
    fn parse_null() {
        assert_eq!(parse_json("null").unwrap(), DynValue::Null);
    }

    #[test]
    fn parse_true() {
        assert_eq!(parse_json("true").unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn parse_false() {
        assert_eq!(parse_json("false").unwrap(), DynValue::Bool(false));
    }

    // -- parse: numbers -----------------------------------------------------

    #[test]
    fn parse_integer_zero() {
        assert_eq!(parse_json("0").unwrap(), DynValue::Integer(0));
    }

    #[test]
    fn parse_positive_integer() {
        assert_eq!(parse_json("42").unwrap(), DynValue::Integer(42));
    }

    #[test]
    fn parse_negative_integer() {
        assert_eq!(parse_json("-17").unwrap(), DynValue::Integer(-17));
    }

    #[test]
    fn parse_large_integer() {
        assert_eq!(
            parse_json("9223372036854775807").unwrap(),
            DynValue::Integer(i64::MAX)
        );
    }

    #[test]
    fn parse_float_basic() {
        assert_eq!(parse_json("3.14").unwrap(), DynValue::Float(3.14));
    }

    #[test]
    fn parse_negative_float() {
        assert_eq!(parse_json("-0.5").unwrap(), DynValue::Float(-0.5));
    }

    #[test]
    fn parse_scientific_notation() {
        assert_eq!(parse_json("1.5e10").unwrap(), DynValue::Float(1.5e10));
    }

    #[test]
    fn parse_scientific_negative_exponent() {
        assert_eq!(parse_json("2.5E-3").unwrap(), DynValue::Float(2.5e-3));
    }

    #[test]
    fn parse_scientific_positive_exponent() {
        assert_eq!(parse_json("1e+2").unwrap(), DynValue::Float(1e2));
    }

    #[test]
    fn parse_integer_overflow_becomes_float() {
        // A number too large for i64 should parse as f64.
        let big = "99999999999999999999";
        match parse_json(big).unwrap() {
            DynValue::Float(_) => {}
            other => panic!("expected Float, got {:?}", other),
        }
    }

    // -- parse: strings -----------------------------------------------------

    #[test]
    fn parse_empty_string() {
        assert_eq!(
            parse_json(r#""""#).unwrap(),
            DynValue::String(String::new())
        );
    }

    #[test]
    fn parse_simple_string() {
        assert_eq!(
            parse_json(r#""hello""#).unwrap(),
            DynValue::String("hello".into())
        );
    }

    #[test]
    fn parse_string_escapes() {
        let input = r#""a\"b\\c\/d\be\ff\ng\rh\ti""#;
        let expected = "a\"b\\c/d\u{08}e\u{0C}f\ng\rh\ti";
        assert_eq!(
            parse_json(input).unwrap(),
            DynValue::String(expected.into())
        );
    }

    #[test]
    fn parse_string_unicode_escape() {
        assert_eq!(
            parse_json(r#""\u0041""#).unwrap(),
            DynValue::String("A".into())
        );
    }

    #[test]
    fn parse_string_surrogate_pair() {
        // U+1F600 (grinning face) = \uD83D\uDE00
        assert_eq!(
            parse_json(r#""\uD83D\uDE00""#).unwrap(),
            DynValue::String("\u{1F600}".into())
        );
    }

    #[test]
    fn parse_string_with_utf8() {
        assert_eq!(
            parse_json("\"caf\u{00E9}\"").unwrap(),
            DynValue::String("caf\u{00E9}".into())
        );
    }

    // -- parse: arrays ------------------------------------------------------

    #[test]
    fn parse_empty_array() {
        assert_eq!(parse_json("[]").unwrap(), DynValue::Array(vec![]));
    }

    #[test]
    fn parse_array_of_ints() {
        assert_eq!(
            parse_json("[1, 2, 3]").unwrap(),
            DynValue::Array(vec![
                DynValue::Integer(1),
                DynValue::Integer(2),
                DynValue::Integer(3),
            ])
        );
    }

    #[test]
    fn parse_nested_array() {
        assert_eq!(
            parse_json("[[1], [2, 3]]").unwrap(),
            DynValue::Array(vec![
                DynValue::Array(vec![DynValue::Integer(1)]),
                DynValue::Array(vec![DynValue::Integer(2), DynValue::Integer(3)]),
            ])
        );
    }

    #[test]
    fn parse_mixed_array() {
        let input = r#"[null, true, 42, "hi", []]"#;
        assert_eq!(
            parse_json(input).unwrap(),
            DynValue::Array(vec![
                DynValue::Null,
                DynValue::Bool(true),
                DynValue::Integer(42),
                DynValue::String("hi".into()),
                DynValue::Array(vec![]),
            ])
        );
    }

    // -- parse: objects -----------------------------------------------------

    #[test]
    fn parse_empty_object() {
        assert_eq!(parse_json("{}").unwrap(), DynValue::Object(vec![]));
    }

    #[test]
    fn parse_simple_object() {
        assert_eq!(
            parse_json(r#"{"a": 1, "b": "two"}"#).unwrap(),
            DynValue::Object(vec![
                ("a".into(), DynValue::Integer(1)),
                ("b".into(), DynValue::String("two".into())),
            ])
        );
    }

    #[test]
    fn parse_nested_object() {
        let input = r#"{"x": {"y": true}}"#;
        assert_eq!(
            parse_json(input).unwrap(),
            DynValue::Object(vec![(
                "x".into(),
                DynValue::Object(vec![("y".into(), DynValue::Bool(true))])
            )])
        );
    }

    #[test]
    fn parse_object_with_array_value() {
        let input = r#"{"items": [1, 2]}"#;
        assert_eq!(
            parse_json(input).unwrap(),
            DynValue::Object(vec![(
                "items".into(),
                DynValue::Array(vec![DynValue::Integer(1), DynValue::Integer(2)])
            )])
        );
    }

    // -- parse: whitespace handling -----------------------------------------

    #[test]
    fn parse_with_whitespace() {
        let input = "  {  \"a\"  :  1  }  ";
        assert_eq!(
            parse_json(input).unwrap(),
            DynValue::Object(vec![("a".into(), DynValue::Integer(1))])
        );
    }

    #[test]
    fn parse_with_newlines_and_tabs() {
        let input = "{\n\t\"a\":\n\t\t1\n}";
        assert_eq!(
            parse_json(input).unwrap(),
            DynValue::Object(vec![("a".into(), DynValue::Integer(1))])
        );
    }

    // -- parse: error cases -------------------------------------------------

    #[test]
    fn parse_error_trailing_chars() {
        assert!(parse_json("true false").is_err());
    }

    #[test]
    fn parse_error_unterminated_string() {
        assert!(parse_json(r#""abc"#).is_err());
    }

    #[test]
    fn parse_error_bad_escape() {
        assert!(parse_json(r#""\x""#).is_err());
    }

    #[test]
    fn parse_error_expected_comma_in_array() {
        assert!(parse_json("[1 2]").is_err());
    }

    #[test]
    fn parse_error_expected_comma_in_object() {
        assert!(parse_json(r#"{"a":1 "b":2}"#).is_err());
    }

    #[test]
    fn parse_error_empty_input() {
        assert!(parse_json("").is_err());
    }

    #[test]
    fn parse_error_bad_literal() {
        assert!(parse_json("tru").is_err());
    }

    #[test]
    fn parse_error_digit_after_decimal() {
        assert!(parse_json("1.").is_err());
    }

    #[test]
    fn parse_error_digit_in_exponent() {
        assert!(parse_json("1e").is_err());
    }

    // -- serialize: compact -------------------------------------------------

    #[test]
    fn serialize_null_compact() {
        assert_eq!(to_json(&DynValue::Null, false), "null");
    }

    #[test]
    fn serialize_bool_compact() {
        assert_eq!(to_json(&DynValue::Bool(true), false), "true");
        assert_eq!(to_json(&DynValue::Bool(false), false), "false");
    }

    #[test]
    fn serialize_integer_compact() {
        assert_eq!(to_json(&DynValue::Integer(42), false), "42");
        assert_eq!(to_json(&DynValue::Integer(-7), false), "-7");
    }

    #[test]
    fn serialize_float_compact() {
        assert_eq!(to_json(&DynValue::Float(3.14), false), "3.14");
    }

    #[test]
    fn serialize_float_whole_number() {
        // A float with no fractional part should still have ".0".
        assert_eq!(to_json(&DynValue::Float(5.0), false), "5.0");
    }

    #[test]
    fn serialize_string_compact() {
        assert_eq!(to_json(&DynValue::String("hi".into()), false), r#""hi""#);
    }

    #[test]
    fn serialize_string_escapes() {
        let s = DynValue::String("a\"b\\c\n\r\t\u{08}\u{0C}".into());
        assert_eq!(to_json(&s, false), r#""a\"b\\c\n\r\t\b\f""#);
    }

    #[test]
    fn serialize_control_char() {
        let s = DynValue::String("\u{001F}".into());
        assert_eq!(to_json(&s, false), r#""\u001f""#);
    }

    #[test]
    fn serialize_empty_array_compact() {
        assert_eq!(to_json(&DynValue::Array(vec![]), false), "[]");
    }

    #[test]
    fn serialize_array_compact() {
        let v = DynValue::Array(vec![
            DynValue::Integer(1),
            DynValue::Bool(true),
            DynValue::Null,
        ]);
        assert_eq!(to_json(&v, false), "[1,true,null]");
    }

    #[test]
    fn serialize_empty_object_compact() {
        assert_eq!(to_json(&DynValue::Object(vec![]), false), "{}");
    }

    #[test]
    fn serialize_object_compact() {
        let v = DynValue::Object(vec![
            ("a".into(), DynValue::Integer(1)),
            ("b".into(), DynValue::String("two".into())),
        ]);
        assert_eq!(to_json(&v, false), r#"{"a":1,"b":"two"}"#);
    }

    // -- serialize: pretty --------------------------------------------------

    #[test]
    fn serialize_array_pretty() {
        let v = DynValue::Array(vec![DynValue::Integer(1), DynValue::Integer(2)]);
        let expected = "[\n  1,\n  2\n]";
        assert_eq!(to_json(&v, true), expected);
    }

    #[test]
    fn serialize_object_pretty() {
        let v = DynValue::Object(vec![
            ("a".into(), DynValue::Integer(1)),
            ("b".into(), DynValue::Integer(2)),
        ]);
        let expected = "{\n  \"a\": 1,\n  \"b\": 2\n}";
        assert_eq!(to_json(&v, true), expected);
    }

    #[test]
    fn serialize_nested_pretty() {
        let v = DynValue::Object(vec![(
            "arr".into(),
            DynValue::Array(vec![DynValue::Integer(1)]),
        )]);
        let expected = "{\n  \"arr\": [\n    1\n  ]\n}";
        assert_eq!(to_json(&v, true), expected);
    }

    // -- round-trip ---------------------------------------------------------

    #[test]
    fn roundtrip_complex() {
        let input = r#"{"name":"Alice","age":30,"scores":[95.5,87,100],"address":{"city":"NYC","zip":null},"active":true}"#;
        let parsed = parse_json(input).unwrap();
        let output = to_json(&parsed, false);
        let reparsed = parse_json(&output).unwrap();
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn roundtrip_pretty() {
        let v = DynValue::Object(vec![
            ("x".into(), DynValue::Array(vec![
                DynValue::Integer(1),
                DynValue::Object(vec![("nested".into(), DynValue::Bool(false))]),
            ])),
        ]);
        let pretty = to_json(&v, true);
        let reparsed = parse_json(&pretty).unwrap();
        assert_eq!(v, reparsed);
    }

    #[test]
    fn roundtrip_string_escapes() {
        let original = DynValue::String("line1\nline2\ttab\\slash\"quote".into());
        let json = to_json(&original, false);
        let reparsed = parse_json(&json).unwrap();
        assert_eq!(original, reparsed);
    }

    // -- edge cases ---------------------------------------------------------

    #[test]
    fn parse_deeply_nested() {
        let input = "[[[[[[1]]]]]]";
        let parsed = parse_json(input).unwrap();
        // Drill down 6 levels.
        let mut v = &parsed;
        for _ in 0..5 {
            v = &v.as_array().unwrap()[0];
        }
        assert_eq!(v.as_array().unwrap()[0], DynValue::Integer(1));
    }

    #[test]
    fn serialize_nan_and_inf() {
        assert_eq!(to_json(&DynValue::Float(f64::NAN), false), "null");
        assert_eq!(to_json(&DynValue::Float(f64::INFINITY), false), "null");
        assert_eq!(to_json(&DynValue::Float(f64::NEG_INFINITY), false), "null");
    }

    #[test]
    fn parse_negative_zero() {
        // -0 in JSON is valid; should parse as float -0.0 or integer 0.
        let v = parse_json("-0").unwrap();
        match v {
            DynValue::Integer(0) => {}
            DynValue::Float(f) if f == 0.0 => {}
            other => panic!("unexpected: {:?}", other),
        }
    }
}
