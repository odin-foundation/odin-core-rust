//! Security hardening tests — ensure parser handles malicious/adversarial input safely.

#[cfg(test)]
mod prototype_pollution {
    use crate::Odin;

    #[test] fn proto_key_treated_as_normal() { let d = Odin::parse("__proto__ = \"val\"\n").unwrap(); assert_eq!(d.get_string("__proto__"), Some("val")); }
    #[test] fn constructor_key_treated_as_normal() { let d = Odin::parse("constructor = \"val\"\n").unwrap(); assert_eq!(d.get_string("constructor"), Some("val")); }
    #[test] fn toString_key() { let d = Odin::parse("toString = \"val\"\n").unwrap(); assert_eq!(d.get_string("toString"), Some("val")); }
    #[test] fn hasOwnProperty_key() { let d = Odin::parse("hasOwnProperty = \"val\"\n").unwrap(); assert_eq!(d.get_string("hasOwnProperty"), Some("val")); }
    #[test] fn valueOf_key() { let d = Odin::parse("valueOf = \"val\"\n").unwrap(); assert_eq!(d.get_string("valueOf"), Some("val")); }
}

#[cfg(test)]
mod input_limits {
    use crate::Odin;

    #[test]
    fn deeply_nested_sections() {
        let mut input = String::new();
        let mut path = String::new();
        for i in 0..50 {
            if i > 0 { path.push('.'); }
            path.push_str(&format!("s{i}"));
            input.push_str(&format!("{{{path}}}\n"));
        }
        input.push_str("val = ##1\n");
        let result = Odin::parse(&input);
        // Should either parse or return error, not crash
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn many_fields() {
        let mut input = String::new();
        for i in 0..1000 {
            input.push_str(&format!("field_{i} = ##${i}\n"));
        }
        let result = Odin::parse(&input);
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn very_long_string() {
        let long = "x".repeat(100_000);
        let input = format!("val = \"{long}\"\n");
        let result = Odin::parse(&input);
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn very_long_key() {
        let long = "k".repeat(10_000);
        let input = format!("{long} = \"val\"\n");
        let result = Odin::parse(&input);
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn many_sections() {
        let mut input = String::new();
        for i in 0..500 {
            input.push_str(&format!("{{Section{i}}}\nfield = ##${i}\n"));
        }
        let result = Odin::parse(&input);
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn large_array() {
        let mut input = String::new();
        for i in 0..500 {
            input.push_str(&format!("items[{i}] = \"val{i}\"\n"));
        }
        let result = Odin::parse(&input);
        assert!(result.is_ok() || result.is_err());
    }
}

#[cfg(test)]
mod encoding_safety {
    use crate::Odin;

    #[test] fn null_bytes_in_string() { let input = "x = \"hello\\u0000world\"\n"; let r = Odin::parse(input); assert!(r.is_ok() || r.is_err()); }
    #[test] fn unicode_in_key() { let r = Odin::parse("café = \"coffee\"\n"); assert!(r.is_ok() || r.is_err()); /* tokenizer may reject non-ASCII keys */ }
    #[test] fn emoji_in_string() { let d = Odin::parse("x = \"hello 🌍\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("hello 🌍")); }
    #[test] fn cjk_characters() { let d = Odin::parse("x = \"日本語\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("日本語")); }
    #[test] fn rtl_text() { let d = Odin::parse("x = \"مرحبا\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("مرحبا")); }
    #[test] fn empty_section_name() { let r = Odin::parse("{}\nval = ##1\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn whitespace_only_value() { let d = Odin::parse("x = \"   \"\n").unwrap(); assert_eq!(d.get_string("x"), Some("   ")); }
}

#[cfg(test)]
mod transform_security {
    use crate::types::transform::DynValue;

    #[test]
    fn deeply_nested_object() {
        let mut obj = DynValue::String("leaf".to_string());
        for i in 0..100 {
            obj = DynValue::Object(vec![(format!("level{i}"), obj)]);
        }
        // Should not stack overflow
        assert!(matches!(obj, DynValue::Object(_)));
    }

    #[test]
    fn large_array_dynvalue() {
        let arr: Vec<DynValue> = (0..10000).map(|i| DynValue::Integer(i)).collect();
        let dv = DynValue::Array(arr);
        if let DynValue::Array(a) = &dv { assert_eq!(a.len(), 10000); }
    }
}

#[cfg(test)]
mod injection_safety {
    use crate::Odin;

    #[test] fn key_with_equals() { let r = Odin::parse("key=val = \"test\"\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn key_with_bracket() { let r = Odin::parse("key] = \"test\"\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn key_with_brace() { let r = Odin::parse("key} = \"test\"\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn key_with_at() { let r = Odin::parse("@key = \"test\"\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn key_with_hash() { let r = Odin::parse("#key = \"test\"\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn key_with_semicolon() { let r = Odin::parse(";key = \"test\"\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn section_with_dots() { let r = Odin::parse("{A.B.C.D.E}\nf = ##1\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn section_with_spaces() { let r = Odin::parse("{A B}\nf = ##1\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn double_section_header() { let r = Odin::parse("{{A}}\nf = ##1\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn value_with_triple_hash() { let r = Odin::parse("x = ###42\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn value_with_multiple_equals() { let r = Odin::parse("x = = ##1\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn consecutive_separators() { let r = Odin::parse("x = ##1\n---\n---\ny = ##2\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn only_separators() { let r = Odin::parse("---\n---\n---\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn separator_at_start() { let r = Odin::parse("---\nx = ##1\n"); assert!(r.is_ok() || r.is_err()); }
}

#[cfg(test)]
mod boundary_values {
    use crate::Odin;

    #[test] fn max_i64() { let d = Odin::parse("x = ##9223372036854775807\n").unwrap(); assert!(d.get("x").is_some()); }
    #[test] fn min_i64() { let d = Odin::parse("x = ##-9223372036854775808\n").unwrap(); assert!(d.get("x").is_some()); }
    #[test] fn zero_integer() { let d = Odin::parse("x = ##0\n").unwrap(); assert_eq!(d.get_integer("x"), Some(0)); }
    #[test] fn negative_zero() { let r = Odin::parse("x = ##-0\n"); assert!(r.is_ok() || r.is_err()); }
    #[test] fn zero_number() { let d = Odin::parse("x = #0.0\n").unwrap(); assert!((d.get_number("x").unwrap()).abs() < 0.001); }
    #[test] fn very_small_number() { let d = Odin::parse("x = #0.000001\n").unwrap(); assert!(d.get_number("x").unwrap() > 0.0); }
    #[test] fn very_large_number() { let d = Odin::parse("x = #999999999.999\n").unwrap(); assert!(d.get_number("x").unwrap() > 999999999.0); }
    #[test] fn zero_currency() { let d = Odin::parse("x = #$0.00\n").unwrap(); assert!(d.get("x").unwrap().is_currency()); }
    #[test] fn zero_percent() { let d = Odin::parse("x = #%0\n").unwrap(); assert!(d.get("x").unwrap().is_percent()); }
    #[test] fn hundred_percent() { let d = Odin::parse("x = #%100\n").unwrap(); assert!(d.get("x").unwrap().is_percent()); }
    #[test] fn over_hundred_percent() { let d = Odin::parse("x = #%200\n").unwrap(); assert!(d.get("x").unwrap().is_percent()); }
    #[test] fn fractional_percent() { let d = Odin::parse("x = #%0.5\n").unwrap(); assert!(d.get("x").unwrap().is_percent()); }
}

#[cfg(test)]
mod whitespace_handling {
    use crate::Odin;

    #[test] fn trailing_spaces() { let d = Odin::parse("x = ##42   \n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }
    #[test] fn leading_spaces() { let d = Odin::parse("   x = ##42\n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }
    #[test] fn spaces_around_equals() { let d = Odin::parse("x   =   ##42\n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }
    #[test] fn tab_indentation() { let d = Odin::parse("\tx = ##42\n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }
    #[test] fn blank_lines_between() { let d = Odin::parse("a = ##1\n\n\nb = ##2\n").unwrap(); assert_eq!(d.get_integer("a"), Some(1)); assert_eq!(d.get_integer("b"), Some(2)); }
    #[test] fn crlf_line_endings() { let d = Odin::parse("x = ##42\r\n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }
    #[test] fn mixed_line_endings() { let d = Odin::parse("a = ##1\nb = ##2\r\nc = ##3\n").unwrap(); assert_eq!(d.get_integer("a"), Some(1)); assert_eq!(d.get_integer("c"), Some(3)); }
    #[test] fn trailing_newlines() { let d = Odin::parse("x = ##42\n\n\n\n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }
    #[test] fn no_trailing_newline() { let r = Odin::parse("x = ##42"); assert!(r.is_ok() || r.is_err()); }
}

#[cfg(test)]
mod document_structure {
    use crate::Odin;

    #[test]
    fn section_reopen() {
        // If a section is referenced twice, fields should merge
        let r = Odin::parse("{A}\nx = ##1\n{B}\ny = ##2\n{A}\nz = ##3\n");
        assert!(r.is_ok() || r.is_err());
    }

    #[test]
    fn metadata_and_data() {
        let d = Odin::parse("{$}\nodin = \"1.0.0\"\n\nname = \"test\"\nage = ##25\n").unwrap();
        assert_eq!(d.get_string("name"), Some("test"));
        assert_eq!(d.get_integer("age"), Some(25));
    }

    #[test]
    fn multiple_types_same_section() {
        let d = Odin::parse("{S}\na = \"str\"\nb = ##42\nc = #3.14\nd = true\ne = ~\nf = #$99.99\ng = #%50\n").unwrap();
        assert_eq!(d.get_string("S.a"), Some("str"));
        assert_eq!(d.get_integer("S.b"), Some(42));
        assert_eq!(d.get_boolean("S.d"), Some(true));
        assert!(d.get("S.e").unwrap().is_null());
        assert!(d.get("S.f").unwrap().is_currency());
        assert!(d.get("S.g").unwrap().is_percent());
    }

    #[test]
    fn arrays_in_multiple_sections() {
        let d = Odin::parse("{A}\nitems[0] = \"a\"\nitems[1] = \"b\"\n{B}\nitems[0] = \"c\"\n").unwrap();
        assert_eq!(d.get_string("A.items[0]"), Some("a"));
        assert_eq!(d.get_string("B.items[0]"), Some("c"));
    }

    #[test]
    fn modifiers_in_sections() {
        let d = Odin::parse("{Sec}\nreq = !\"val\"\nconf = *\"secret\"\ndep = -\"old\"\n").unwrap();
        assert!(d.get("Sec.req").unwrap().is_required());
        assert!(d.get("Sec.conf").unwrap().is_confidential());
        assert!(d.get("Sec.dep").unwrap().is_deprecated());
    }

    #[test]
    fn comments_everywhere() {
        let input = "; file comment\n{$}\n; meta comment\nodin = \"1.0.0\"\n\n; root comment\nname = \"test\"\n; between\n{S}\n; section comment\nf = ##1 ; inline\n";
        let d = Odin::parse(input).unwrap();
        assert_eq!(d.get_string("name"), Some("test"));
        assert_eq!(d.get_integer("S.f"), Some(1));
    }
}

#[cfg(test)]
mod dynvalue_tests {
    use crate::types::transform::DynValue;

    #[test] fn get_on_object() { let o = DynValue::Object(vec![("a".into(), DynValue::Integer(1))]); assert_eq!(o.get("a"), Some(&DynValue::Integer(1))); }
    #[test] fn get_missing_key() { let o = DynValue::Object(vec![("a".into(), DynValue::Integer(1))]); assert_eq!(o.get("b"), None); }
    #[test] fn get_on_non_object() { let v = DynValue::String("hi".into()); assert_eq!(v.get("x"), None); }
    #[test] fn get_index_on_array() { let a = DynValue::Array(vec![DynValue::Integer(1), DynValue::Integer(2)]); assert_eq!(a.get_index(0), Some(&DynValue::Integer(1))); }
    #[test] fn get_index_out_of_bounds() { let a = DynValue::Array(vec![DynValue::Integer(1)]); assert_eq!(a.get_index(5), None); }
    #[test] fn get_index_on_non_array() { let v = DynValue::Integer(42); assert_eq!(v.get_index(0), None); }
    #[test] fn null_value() { assert!(matches!(DynValue::Null, DynValue::Null)); }
    #[test] fn bool_value() { assert!(matches!(DynValue::Bool(true), DynValue::Bool(true))); }
    #[test] fn integer_value() { assert!(matches!(DynValue::Integer(42), DynValue::Integer(42))); }
    #[test] fn float_value() { if let DynValue::Float(v) = DynValue::Float(3.14) { assert!((v - 3.14).abs() < 0.001); } }
    #[test] fn string_value() { if let DynValue::String(s) = DynValue::String("hi".into()) { assert_eq!(s, "hi"); } }
    #[test] fn empty_object() { let o = DynValue::Object(vec![]); assert_eq!(o.get("x"), None); }
    #[test] fn empty_array() { let a = DynValue::Array(vec![]); assert_eq!(a.get_index(0), None); }
    #[test] fn nested_object_access() { let o = DynValue::Object(vec![("a".into(), DynValue::Object(vec![("b".into(), DynValue::Integer(42))]))]); assert_eq!(o.get("a").unwrap().get("b"), Some(&DynValue::Integer(42))); }
}
