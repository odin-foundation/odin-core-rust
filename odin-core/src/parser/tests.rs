//! Parser tests — comprehensive tests for ODIN text parsing.

use crate::Odin;

// ─── String Parsing ──────────────────────────────────────────────────────────

#[test] fn parse_simple_string() { let d = Odin::parse("x = \"hello\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("hello")); }
#[test] fn parse_empty_string() { let d = Odin::parse("x = \"\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("")); }
#[test] fn parse_string_with_spaces() { let d = Odin::parse("x = \"hello world\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("hello world")); }
#[test] fn parse_string_with_special_chars() { let d = Odin::parse("x = \"a!@#$%\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("a!@#$%")); }
#[test] fn parse_string_with_escaped_quote() { let d = Odin::parse("x = \"say \\\"hello\\\"\"\n").unwrap(); assert!(d.get_string("x").unwrap().contains("hello")); }
#[test] fn parse_string_with_newline_escape() { let d = Odin::parse("x = \"line1\\nline2\"\n").unwrap(); assert!(d.get_string("x").is_some()); }
#[test] fn parse_string_with_tab_escape() { let d = Odin::parse("x = \"col1\\tcol2\"\n").unwrap(); assert!(d.get_string("x").is_some()); }
#[test] fn parse_string_with_backslash() { let d = Odin::parse("x = \"path\\\\to\\\\file\"\n").unwrap(); assert!(d.get_string("x").is_some()); }

// ─── Integer Parsing ─────────────────────────────────────────────────────────

#[test] fn parse_integer_simple() { let d = Odin::parse("x = ##42\n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }
#[test] fn parse_integer_zero() { let d = Odin::parse("x = ##0\n").unwrap(); assert_eq!(d.get_integer("x"), Some(0)); }
#[test] fn parse_integer_negative() { let d = Odin::parse("x = ##-10\n").unwrap(); assert_eq!(d.get_integer("x"), Some(-10)); }
#[test] fn parse_integer_large() { let d = Odin::parse("x = ##999999\n").unwrap(); assert_eq!(d.get_integer("x"), Some(999999)); }
#[test] fn parse_integer_one() { let d = Odin::parse("x = ##1\n").unwrap(); assert_eq!(d.get_integer("x"), Some(1)); }
#[test] fn parse_integer_neg_one() { let d = Odin::parse("x = ##-1\n").unwrap(); assert_eq!(d.get_integer("x"), Some(-1)); }

// ─── Number Parsing ──────────────────────────────────────────────────────────

#[test] fn parse_number_decimal() { let d = Odin::parse("x = #3.14\n").unwrap(); assert!((d.get_number("x").unwrap() - 3.14).abs() < 0.001); }
#[test] fn parse_number_negative() { let d = Odin::parse("x = #-1.5\n").unwrap(); assert!((d.get_number("x").unwrap() + 1.5).abs() < 0.001); }
#[test] fn parse_number_zero() { let d = Odin::parse("x = #0.0\n").unwrap(); assert!((d.get_number("x").unwrap()).abs() < 0.001); }
#[test] fn parse_number_large() { let d = Odin::parse("x = #1000000.5\n").unwrap(); assert!((d.get_number("x").unwrap() - 1000000.5).abs() < 1.0); }
#[test] fn parse_number_small() { let d = Odin::parse("x = #0.001\n").unwrap(); assert!((d.get_number("x").unwrap() - 0.001).abs() < 0.0001); }

// ─── Boolean Parsing ─────────────────────────────────────────────────────────

#[test] fn parse_bool_true() { let d = Odin::parse("x = true\n").unwrap(); assert_eq!(d.get_boolean("x"), Some(true)); }
#[test] fn parse_bool_false() { let d = Odin::parse("x = false\n").unwrap(); assert_eq!(d.get_boolean("x"), Some(false)); }

// ─── Null Parsing ────────────────────────────────────────────────────────────

#[test] fn parse_null() { let d = Odin::parse("x = ~\n").unwrap(); assert!(d.get("x").unwrap().is_null()); }

// ─── Currency Parsing ────────────────────────────────────────────────────────

#[test] fn parse_currency() { let d = Odin::parse("x = #$100.00\n").unwrap(); assert!(d.get("x").unwrap().is_currency()); }
#[test] fn parse_currency_zero() { let d = Odin::parse("x = #$0.00\n").unwrap(); assert!(d.get("x").unwrap().is_currency()); }
#[test] fn parse_currency_large() { let d = Odin::parse("x = #$999999.99\n").unwrap(); assert!(d.get("x").unwrap().is_currency()); }

// ─── Percent Parsing ─────────────────────────────────────────────────────────

#[test] fn parse_percent() { let d = Odin::parse("x = #%50\n").unwrap(); assert!(d.get("x").unwrap().is_percent()); }
#[test] fn parse_percent_decimal() { let d = Odin::parse("x = #%99.9\n").unwrap(); assert!(d.get("x").unwrap().is_percent()); }

// ─── Date Parsing ────────────────────────────────────────────────────────────

#[test] fn parse_date_standard() { let d = Odin::parse("x = 2024-01-15\n").unwrap(); assert!(d.get("x").unwrap().is_date()); }
#[test] fn parse_date_leap() { let d = Odin::parse("x = 2024-02-29\n").unwrap(); assert!(d.get("x").unwrap().is_date()); }
#[test] fn parse_date_dec31() { let d = Odin::parse("x = 2024-12-31\n").unwrap(); assert!(d.get("x").unwrap().is_date()); }
#[test] fn parse_date_jan01() { let d = Odin::parse("x = 2024-01-01\n").unwrap(); assert!(d.get("x").unwrap().is_date()); }

// ─── Timestamp Parsing ──────────────────────────────────────────────────────

#[test] fn parse_ts_utc() { let d = Odin::parse("x = 2024-01-15T10:30:00Z\n").unwrap(); assert!(d.get("x").unwrap().is_timestamp()); }
#[test] fn parse_ts_offset_pos() { let d = Odin::parse("x = 2024-01-15T10:30:00+05:30\n").unwrap(); assert!(d.get("x").unwrap().is_timestamp()); }
#[test] fn parse_ts_offset_neg() { let d = Odin::parse("x = 2024-01-15T10:30:00-08:00\n").unwrap(); assert!(d.get("x").unwrap().is_timestamp()); }
#[test] fn parse_ts_millis() { let d = Odin::parse("x = 2024-01-15T10:30:00.123Z\n").unwrap(); assert!(d.get("x").unwrap().is_timestamp()); }

// ─── Time Parsing ────────────────────────────────────────────────────────────

#[test] fn parse_time() { let d = Odin::parse("x = T10:30:00\n").unwrap(); assert!(d.get("x").unwrap().is_temporal()); }
#[test] fn parse_time_midnight() { let d = Odin::parse("x = T00:00:00\n").unwrap(); assert!(d.get("x").unwrap().is_temporal()); }
#[test] fn parse_time_end_of_day() { let d = Odin::parse("x = T23:59:59\n").unwrap(); assert!(d.get("x").unwrap().is_temporal()); }

// ─── Duration Parsing ────────────────────────────────────────────────────────

#[test] fn parse_duration_days() { let d = Odin::parse("x = P30D\n").unwrap(); assert!(d.get("x").is_some()); }
#[test] fn parse_duration_hours() { let d = Odin::parse("x = PT24H\n").unwrap(); assert!(d.get("x").is_some()); }
#[test] fn parse_duration_full() { let d = Odin::parse("x = P1Y2M3DT4H5M6S\n").unwrap(); assert!(d.get("x").is_some()); }
#[test] fn parse_duration_year_month() { let d = Odin::parse("x = P1Y6M\n").unwrap(); assert!(d.get("x").is_some()); }

// ─── Reference Parsing ──────────────────────────────────────────────────────

#[test] fn parse_reference() { let d = Odin::parse("x = @other\n").unwrap(); assert!(d.get("x").unwrap().is_reference()); }
#[test] fn parse_reference_dotted() { let d = Odin::parse("x = @path.to.thing\n").unwrap(); assert!(d.get("x").unwrap().is_reference()); }
#[test] fn parse_reference_array() { let d = Odin::parse("x = @items[0]\n").unwrap(); assert!(d.get("x").unwrap().is_reference()); }

// ─── Binary Parsing ─────────────────────────────────────────────────────────

#[test] fn parse_binary() { let d = Odin::parse("x = ^SGVsbG8=\n").unwrap(); assert!(d.get("x").unwrap().is_binary()); }

// ─── Section Parsing ─────────────────────────────────────────────────────────

#[test] fn parse_section() { let d = Odin::parse("{S}\nf = ##1\n").unwrap(); assert_eq!(d.get_integer("S.f"), Some(1)); }
#[test] fn parse_nested_section() { let d = Odin::parse("{A}\n{A.B}\nf = ##1\n").unwrap(); assert_eq!(d.get_integer("A.B.f"), Some(1)); }
#[test] fn parse_multiple_sections() { let d = Odin::parse("{A}\nx = ##1\n{B}\ny = ##2\n").unwrap(); assert_eq!(d.get_integer("A.x"), Some(1)); assert_eq!(d.get_integer("B.y"), Some(2)); }
#[test] fn parse_section_multiple_fields() { let d = Odin::parse("{S}\na = ##1\nb = ##2\nc = ##3\n").unwrap(); assert_eq!(d.get_integer("S.a"), Some(1)); assert_eq!(d.get_integer("S.b"), Some(2)); assert_eq!(d.get_integer("S.c"), Some(3)); }

// ─── Array Parsing ───────────────────────────────────────────────────────────

#[test] fn parse_array_strings() { let d = Odin::parse("x[0] = \"a\"\nx[1] = \"b\"\n").unwrap(); assert_eq!(d.get_string("x[0]"), Some("a")); assert_eq!(d.get_string("x[1]"), Some("b")); }
#[test] fn parse_array_integers() { let d = Odin::parse("x[0] = ##1\nx[1] = ##2\nx[2] = ##3\n").unwrap(); assert_eq!(d.get_integer("x[0]"), Some(1)); assert_eq!(d.get_integer("x[2]"), Some(3)); }
#[test] fn parse_array_mixed() { let d = Odin::parse("x[0] = \"a\"\nx[1] = ##42\nx[2] = true\n").unwrap(); assert_eq!(d.get_string("x[0]"), Some("a")); assert_eq!(d.get_integer("x[1]"), Some(42)); assert_eq!(d.get_boolean("x[2]"), Some(true)); }
#[test] fn parse_array_in_section() { let d = Odin::parse("{S}\nitems[0] = \"x\"\nitems[1] = \"y\"\n").unwrap(); assert_eq!(d.get_string("S.items[0]"), Some("x")); }

// ─── Modifier Parsing ────────────────────────────────────────────────────────

#[test] fn parse_required() { let d = Odin::parse("x = !\"val\"\n").unwrap(); assert!(d.get("x").unwrap().is_required()); }
#[test] fn parse_confidential() { let d = Odin::parse("x = *\"secret\"\n").unwrap(); assert!(d.get("x").unwrap().is_confidential()); }
#[test] fn parse_deprecated() { let d = Odin::parse("x = -\"old\"\n").unwrap(); assert!(d.get("x").unwrap().is_deprecated()); }
#[test] fn parse_combined_modifiers() { let d = Odin::parse("x = !-*\"val\"\n").unwrap(); let v = d.get("x").unwrap(); assert!(v.is_required()); assert!(v.is_deprecated()); assert!(v.is_confidential()); }
#[test] fn parse_required_integer() { let d = Odin::parse("x = !##42\n").unwrap(); assert!(d.get("x").unwrap().is_required()); assert_eq!(d.get_integer("x"), Some(42)); }
#[test] fn parse_required_boolean() { let d = Odin::parse("x = !true\n").unwrap(); assert!(d.get("x").unwrap().is_required()); }
#[test] fn parse_required_null() { let d = Odin::parse("x = !~\n").unwrap(); assert!(d.get("x").unwrap().is_required()); }
#[test] fn parse_confidential_null() { let d = Odin::parse("x = *~\n").unwrap(); assert!(d.get("x").unwrap().is_confidential()); }
#[test] fn parse_required_number() { let d = Odin::parse("x = !#3.14\n").unwrap(); assert!(d.get("x").unwrap().is_required()); }
#[test] fn parse_required_currency() { let d = Odin::parse("x = !#$99.99\n").unwrap(); assert!(d.get("x").unwrap().is_required()); assert!(d.get("x").unwrap().is_currency()); }

// ─── Comment Parsing ─────────────────────────────────────────────────────────

#[test] fn comment_ignored() { let d = Odin::parse("; comment\nx = ##1\n").unwrap(); assert_eq!(d.get_integer("x"), Some(1)); }
#[test] fn inline_comment() { let d = Odin::parse("x = ##1 ; inline\n").unwrap(); assert_eq!(d.get_integer("x"), Some(1)); }
#[test] fn only_comments() { let d = Odin::parse("; just comments\n; more\n").unwrap(); assert_eq!(d.assignments.len(), 0); }
#[test] fn comment_between_fields() { let d = Odin::parse("a = ##1\n; separator\nb = ##2\n").unwrap(); assert_eq!(d.get_integer("a"), Some(1)); assert_eq!(d.get_integer("b"), Some(2)); }

// ─── Metadata Parsing ────────────────────────────────────────────────────────

#[test] fn metadata_section() { let d = Odin::parse("{$}\nodin = \"1.0.0\"\n\nx = ##1\n").unwrap(); assert!(d.metadata.get(&"odin".to_string()).is_some()); }
#[test] fn metadata_with_fields() { let d = Odin::parse("{$}\nodin = \"1.0.0\"\n\nname = \"doc\"\n").unwrap(); assert_eq!(d.get_string("name"), Some("doc")); }

// ─── Edge Cases ──────────────────────────────────────────────────────────────

#[test] fn empty_input() { let d = Odin::parse("").unwrap(); assert_eq!(d.assignments.len(), 0); }
#[test] fn whitespace_only() { let d = Odin::parse("   \n\n  \n").unwrap(); assert_eq!(d.assignments.len(), 0); }
#[test] fn unterminated_string() { assert!(Odin::parse("x = \"unterminated\n").is_err()); }
#[test] fn bare_string_error() { assert!(Odin::parse("x = bareword\n").is_err()); }
#[test] fn negative_array_index() { assert!(Odin::parse("x[-1] = \"bad\"\n").is_err()); }
#[test] fn non_contiguous_array() { assert!(Odin::parse("x[0] = \"a\"\nx[2] = \"c\"\n").is_err()); }
// Index that overflows i64 must error, not silently collide at [0].
#[test] fn array_index_overflow_errors() { assert!(Odin::parse("x[99999999999999999999] = \"bad\"\n").is_err()); }
// Same overflow but with leading zeros — normalization path must not mask it.
#[test] fn array_index_overflow_with_leading_zeros_errors() { assert!(Odin::parse("x[0099999999999999999999] = \"bad\"\n").is_err()); }
// Tabular section header parses cleanly (regression guard for dedupe of the
// `contains("[] :") || contains("[] :")` dead-code condition).
#[test] fn tabular_section_header_parses() {
    let d = Odin::parse("{rows[] : id, name}\n##0, \"Alice\"\n##1, \"Bob\"\n").unwrap();
    assert_eq!(d.get_string("rows[0].name"), Some("Alice"));
    assert_eq!(d.get_string("rows[1].name"), Some("Bob"));
    assert_eq!(d.get_integer("rows[0].id"), Some(0));
}
// Header-inline `:if` emits a synthetic `<section>._if` assignment capturing
// the unquoted expression text up to the closing brace.
#[test] fn header_inline_if_directive() {
    let d = Odin::parse("{DuiDetails :if @driver.hasDui = true}\nstate = \"TX\"\n").unwrap();
    assert_eq!(d.get_string("DuiDetails._if"), Some("@driver.hasDui = true"));
    assert_eq!(d.get_string("DuiDetails.state"), Some("TX"));
}
// Header-inline `:if` with a verb-expression condition.
#[test] fn header_inline_if_verb_directive() {
    let d = Odin::parse("{HighRisk :if %eq @driver.tier \"dui\"}\nband = \"high\"\n").unwrap();
    assert_eq!(d.get_string("HighRisk._if"), Some("%eq @driver.tier \"dui\""));
    assert_eq!(d.get_string("HighRisk.band"), Some("high"));
}
// Header-inline `:elif` / `:else` emit synthetic chain directives.
#[test] fn header_inline_elif_else_directives() {
    let d = Odin::parse("{A :if @x}\nv = \"a\"\n{B :elif %lt @y ##5}\nv = \"b\"\n{C :else}\nv = \"c\"\n").unwrap();
    assert_eq!(d.get_string("B._elif"), Some("%lt @y ##5"));
    assert_eq!(d.get_string("C._else"), Some("true"));
}
// Header-inline `:type` emits a synthetic `<section>._type` assignment.
#[test] fn header_inline_type_directive() {
    let d = Odin::parse("{Vehicle :type \"car\"}\nmake = \"Honda\"\n").unwrap();
    assert_eq!(d.get_string("Vehicle._type"), Some("car"));
    assert_eq!(d.get_string("Vehicle.make"), Some("Honda"));
}
#[test] fn multiple_newlines() { let d = Odin::parse("x = ##1\n\n\n\ny = ##2\n").unwrap(); assert_eq!(d.get_integer("x"), Some(1)); assert_eq!(d.get_integer("y"), Some(2)); }
#[test] fn trailing_whitespace() { let d = Odin::parse("x = ##42   \n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }
#[test] fn leading_whitespace() { let d = Odin::parse("  x = ##42\n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }

// ─── Import/Schema Directives ────────────────────────────────────────────────

#[test] fn import_directive() { let d = Odin::parse("@import \"types.schema.odin\"\n").unwrap(); assert!(!d.imports.is_empty()); }
#[test] fn import_with_alias() { let d = Odin::parse("@import \"types.schema.odin\" as types\n").unwrap(); assert!(!d.imports.is_empty()); assert_eq!(d.imports[0].alias.as_deref(), Some("types")); }
#[test] fn schema_directive() { let d = Odin::parse("@schema \"my-schema\"\n").unwrap(); assert!(!d.schemas.is_empty()); }

// ─── Multiple Documents ──────────────────────────────────────────────────────

#[test] fn two_docs() { let docs = Odin::parse_documents("x = ##1\n---\ny = ##2\n").unwrap(); assert_eq!(docs.len(), 2); }
#[test] fn three_docs() { let docs = Odin::parse_documents("a = ##1\n---\nb = ##2\n---\nc = ##3\n").unwrap(); assert_eq!(docs.len(), 3); }
#[test] fn single_doc() { let docs = Odin::parse_documents("x = ##1\n").unwrap(); assert_eq!(docs.len(), 1); }

// ─── Extended String Tests ─────────────────────────────────────────────────

#[test] fn string_with_semicolon() { let d = Odin::parse("x = \"has ; semi\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("has ; semi")); }
#[test] fn string_with_equals() { let d = Odin::parse("x = \"a = b\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("a = b")); }
#[test] fn string_with_braces() { let d = Odin::parse("x = \"{inside}\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("{inside}")); }
#[test] fn string_with_brackets() { let d = Odin::parse("x = \"arr[0]\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("arr[0]")); }
#[test] fn string_with_hash() { let d = Odin::parse("x = \"#not-number\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("#not-number")); }
#[test] fn string_with_at() { let d = Odin::parse("x = \"@not-ref\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("@not-ref")); }
#[test] fn string_with_tilde() { let d = Odin::parse("x = \"~not-null\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("~not-null")); }
#[test] fn string_with_caret() { let d = Odin::parse("x = \"^not-binary\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("^not-binary")); }
#[test] fn string_with_all_escapes() { let d = Odin::parse("x = \"\\n\\t\\r\\\\\\\"\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("\n\t\r\\\"")); }
#[test] fn unicode_emoji_string() { let d = Odin::parse("x = \"🎉🚀💻\"\n").unwrap(); assert_eq!(d.get_string("x"), Some("🎉🚀💻")); }
#[test] fn long_string() { let s = "a".repeat(1000); let input = format!("x = \"{s}\"\n"); let d = Odin::parse(&input).unwrap(); assert_eq!(d.get_string("x").unwrap().len(), 1000); }

// ─── Extended Number Tests ─────────────────────────────────────────────────

#[test] fn number_small_decimal() { let d = Odin::parse("x = #0.001\n").unwrap(); assert!(d.get_number("x").unwrap() > 0.0); }
#[test] fn number_negative_decimal() { let d = Odin::parse("x = #-0.5\n").unwrap(); assert!(d.get_number("x").unwrap() < 0.0); }
#[test] fn integer_max() { let d = Odin::parse("x = ##2147483647\n").unwrap(); assert_eq!(d.get_integer("x"), Some(2147483647)); }
#[test] fn integer_min() { let d = Odin::parse("x = ##-2147483648\n").unwrap(); assert_eq!(d.get_integer("x"), Some(-2147483648)); }
#[test] fn currency_large() { let d = Odin::parse("x = #$999999.99\n").unwrap(); assert!(d.get("x").unwrap().is_currency()); }
#[test] fn currency_one_cent() { let d = Odin::parse("x = #$0.01\n").unwrap(); assert!(d.get("x").unwrap().is_currency()); }
#[test] fn percent_decimal() { let d = Odin::parse("x = #%33.33\n").unwrap(); assert!(d.get("x").unwrap().is_percent()); }
#[test] fn percent_negative() { let r = Odin::parse("x = #%-5\n"); assert!(r.is_ok() || r.is_err()); }

// ─── Extended Section Tests ────────────────────────────────────────────────

#[test] fn section_then_root() { let d = Odin::parse("{A}\na = ##1\n").unwrap(); assert_eq!(d.get_integer("A.a"), Some(1)); }
#[test] fn three_level_section() { let d = Odin::parse("{A}\n{A.B}\n{A.B.C}\nf = ##42\n").unwrap(); assert_eq!(d.get_integer("A.B.C.f"), Some(42)); }
#[test] fn section_with_many_types() {
    let d = Odin::parse("{S}\ns = \"str\"\ni = ##1\nn = #1.5\nb = true\nnul = ~\nc = #$1.00\np = #%50\n").unwrap();
    assert_eq!(d.get_string("S.s"), Some("str"));
    assert_eq!(d.get_integer("S.i"), Some(1));
    assert_eq!(d.get_boolean("S.b"), Some(true));
    assert!(d.get("S.nul").unwrap().is_null());
    assert!(d.get("S.c").unwrap().is_currency());
    assert!(d.get("S.p").unwrap().is_percent());
}

// ─── Extended Array Tests ──────────────────────────────────────────────────

#[test] fn array_single() { let d = Odin::parse("a[0] = \"only\"\n").unwrap(); assert_eq!(d.get_string("a[0]"), Some("only")); }
#[test] fn array_five_elements() {
    let d = Odin::parse("a[0] = ##0\na[1] = ##1\na[2] = ##2\na[3] = ##3\na[4] = ##4\n").unwrap();
    assert_eq!(d.get_integer("a[0]"), Some(0));
    assert_eq!(d.get_integer("a[4]"), Some(4));
}
#[test] fn array_boolean_elements() {
    let d = Odin::parse("a[0] = true\na[1] = false\na[2] = true\n").unwrap();
    assert_eq!(d.get_boolean("a[0]"), Some(true));
    assert_eq!(d.get_boolean("a[1]"), Some(false));
}
#[test] fn array_null_elements() {
    let d = Odin::parse("a[0] = ~\na[1] = ~\n").unwrap();
    assert!(d.get("a[0]").unwrap().is_null());
}
#[test] fn multiple_arrays_in_section() {
    let d = Odin::parse("{S}\na[0] = ##1\na[1] = ##2\nb[0] = \"x\"\nb[1] = \"y\"\n").unwrap();
    assert_eq!(d.get_integer("S.a[0]"), Some(1));
    assert_eq!(d.get_string("S.b[0]"), Some("x"));
}

// ─── Extended Modifier Tests ───────────────────────────────────────────────

#[test] fn required_number() { let d = Odin::parse("x = !#3.14\n").unwrap(); assert!(d.get("x").unwrap().is_required()); }
#[test] fn confidential_number() { let d = Odin::parse("x = *#3.14\n").unwrap(); assert!(d.get("x").unwrap().is_confidential()); }
#[test] fn deprecated_number() { let d = Odin::parse("x = -#3.14\n").unwrap(); assert!(d.get("x").unwrap().is_deprecated()); }
#[test] fn required_currency() { let d = Odin::parse("x = !#$50.00\n").unwrap(); assert!(d.get("x").unwrap().is_required()); assert!(d.get("x").unwrap().is_currency()); }
#[test] fn confidential_currency() { let d = Odin::parse("x = *#$50.00\n").unwrap(); assert!(d.get("x").unwrap().is_confidential()); }
#[test] fn deprecated_boolean() { let d = Odin::parse("x = -true\n").unwrap(); assert!(d.get("x").unwrap().is_deprecated()); }
#[test] fn required_date() { let d = Odin::parse("x = !2024-01-15\n").unwrap(); assert!(d.get("x").unwrap().is_required()); }
#[test] fn confidential_reference() { let d = Odin::parse("x = *@other\n").unwrap(); assert!(d.get("x").unwrap().is_confidential()); }

// ─── Extended Comment Tests ────────────────────────────────────────────────

#[test] fn comment_before_section() { let d = Odin::parse("; comment\n{S}\nf = ##1\n").unwrap(); assert_eq!(d.get_integer("S.f"), Some(1)); }
#[test] fn comment_in_section() { let d = Odin::parse("{S}\n; comment\nf = ##1\n").unwrap(); assert_eq!(d.get_integer("S.f"), Some(1)); }
#[test] fn multiple_inline_comments() {
    let d = Odin::parse("a = ##1 ; first\nb = ##2 ; second\nc = ##3 ; third\n").unwrap();
    assert_eq!(d.get_integer("a"), Some(1));
    assert_eq!(d.get_integer("b"), Some(2));
    assert_eq!(d.get_integer("c"), Some(3));
}

// ─── Extended Edge Cases ───────────────────────────────────────────────────

#[test] fn crlf_endings() { let d = Odin::parse("x = ##42\r\ny = ##1\r\n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }
#[test] fn tab_before_key() { let d = Odin::parse("\tx = ##42\n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }
#[test] fn spaces_around_equals() { let d = Odin::parse("x   =   ##42\n").unwrap(); assert_eq!(d.get_integer("x"), Some(42)); }
#[test] fn many_blank_lines() { let d = Odin::parse("x = ##1\n\n\n\n\n\ny = ##2\n").unwrap(); assert_eq!(d.get_integer("y"), Some(2)); }
#[test] fn no_trailing_newline() { let r = Odin::parse("x = ##42"); assert!(r.is_ok() || r.is_err()); }

// ─── Extended Multi-Document ───────────────────────────────────────────────

#[test] fn five_documents() { let d = Odin::parse_documents("a=##1\n---\nb=##2\n---\nc=##3\n---\nd=##4\n---\ne=##5\n"); assert!(d.is_ok() || d.is_err()); }
#[test] fn parse_last_doc() { let d = Odin::parse("a = ##1\n---\nb = ##2\n---\nc = ##3\n").unwrap(); assert_eq!(d.get_integer("c"), Some(3)); }
#[test] fn docs_with_sections() { let docs = Odin::parse_documents("{A}\nf = ##1\n---\n{B}\nf = ##2\n").unwrap(); assert_eq!(docs.len(), 2); }
#[test] fn docs_independent() { let docs = Odin::parse_documents("x = ##1\n---\ny = ##2\n").unwrap(); assert!(docs[1].get("x").is_none()); }

// ─── Comment Preservation ──────────────────────────────────────────────────

#[test]
fn preserve_comments_collects_standalone_and_trailing() {
    let src = "; leading note\nx = ##1 ; trailing\n; between\ny = ##2\n";
    let opts = crate::parser::ParseOptions { preserve_comments: true, ..Default::default() };
    let d = Odin::parse_with_options(src, &opts).unwrap();
    assert_eq!(d.comments.len(), 3);
    assert_eq!(d.comments[0].text, "leading note");
    assert_eq!(d.comments[1].text, "trailing");
    assert_eq!(d.comments[2].text, "between");
}

#[test]
fn preserve_comments_off_by_default() {
    let src = "; ignored\nx = ##1\n";
    let d = Odin::parse(src).unwrap();
    assert_eq!(d.comments.len(), 0);
}

// ─── Lookup Table ($table) Type Fidelity ───────────────────────────────────

#[test]
fn lookup_table_preserves_typed_cells() {
    let src = "{$table.RATE[v, c, base, factor]}\n\"sedan\", \"liability\", ##250, #1.15\n\"truck\", \"collision\", ##300, #1.30\n";
    let d = Odin::parse(src).unwrap();
    use crate::types::values::OdinValue;
    match d.metadata.get(&"table.RATE[0].base".to_string()).unwrap() {
        OdinValue::Integer { value, .. } => assert_eq!(*value, 250),
        other => panic!("expected Integer, got {other:?}"),
    }
    match d.metadata.get(&"table.RATE[0].factor".to_string()).unwrap() {
        OdinValue::Number { value, .. } => assert!((value - 1.15).abs() < 1e-9),
        other => panic!("expected Number, got {other:?}"),
    }
    match d.metadata.get(&"table.RATE[1].factor".to_string()).unwrap() {
        OdinValue::Number { value, .. } => assert!((value - 1.30).abs() < 1e-9),
        other => panic!("expected Number, got {other:?}"),
    }
}

#[test]
fn lookup_table_dot_prefix_form() {
    let src = "{$.table.STATUS[code, name, active]}\n\"A\", \"Active\", ?true\n\"P\", \"Pending\", ?false\n";
    let d = Odin::parse(src).unwrap();
    use crate::types::values::OdinValue;
    assert!(matches!(d.metadata.get(&"table.STATUS[0].code".to_string()), Some(OdinValue::String { .. })));
    match d.metadata.get(&"table.STATUS[0].active".to_string()).unwrap() {
        OdinValue::Boolean { value, .. } => assert!(*value),
        other => panic!("expected Boolean, got {other:?}"),
    }
}

// ─── Malformed-input safety (no panics on truncated input) ─────────────────

#[test]
fn no_panic_on_tabular_assignment_eof_after_equals() {
    // Truncated tabular assignment ending exactly at `=`.
    let _ = Odin::parse("{features[] : name, enabled}\n_loop =");
}

#[test]
fn no_panic_on_tabular_assignment_whitespace_then_eof() {
    let _ = Odin::parse("{features[] : name, enabled}\n_loop = ");
}

#[test]
fn unclosed_bracket_in_path_is_rejected() {
    let r = Odin::parse("a[ = ##1\n");
    assert!(r.is_err(), "expected error for unclosed bracket, got {r:?}");
}

#[test]
fn stray_close_bracket_in_path_is_rejected() {
    let r = Odin::parse("a] = ##1\n");
    assert!(r.is_err(), "expected error for stray close bracket, got {r:?}");
}

#[test]
fn tabular_columns_capped() {
    let mut header = String::from("{x[] : ");
    for i in 0..2000 {
        if i > 0 { header.push_str(", "); }
        header.push_str(&format!("c{i}"));
    }
    header.push('}');
    header.push('\n');
    let r = Odin::parse(&header);
    assert!(r.is_err(), "expected error for excessive tabular columns");
}

// ─── Top-level $.path metadata assignment (canonical form) ──────────────────

#[test]
fn top_level_meta_assignment_routes_to_metadata() {
    let d = Odin::parse("$.odin = \"1.0.0\"\nname = \"Bob\"\n").unwrap();
    assert_eq!(d.get_string("$.odin"), Some("1.0.0"));
    assert_eq!(d.metadata.get(&"odin".to_string()).and_then(|v| v.as_str()), Some("1.0.0"));
    assert_eq!(d.get_string("name"), Some("Bob"));
}

#[test]
fn parse_canonicalize_metadata_roundtrip_is_idempotent() {
    let input = "{$}\nodin = \"1.0.0\"\nid = \"doc1\"\n\nname = \"Bob\"\n";
    let doc = Odin::parse(input).unwrap();
    let c1 = Odin::canonicalize(&doc);
    let text = String::from_utf8(c1.clone()).unwrap();
    assert!(text.contains("$.odin = \"1.0.0\""), "canonical flattens metadata: {text}");
    let doc2 = Odin::parse(&text).unwrap();
    let c2 = Odin::canonicalize(&doc2);
    assert_eq!(c1, c2, "parse(canonicalize(doc)) must be idempotent");
}

// ─── Integer fractional rejection (## non-integral) ─────────────────────────

#[test]
fn integer_with_fraction_is_rejected() {
    let r = Odin::parse("x = ##4.2\n");
    assert!(r.is_err(), "expected error for fractional integer, got {r:?}");
    assert_eq!(r.unwrap_err().code(), "P006");
}

#[test]
fn negative_integer_with_fraction_is_rejected() {
    let r = Odin::parse("x = ##-3.7\n");
    assert!(r.is_err(), "expected error for negative fractional integer, got {r:?}");
    assert_eq!(r.unwrap_err().code(), "P006");
}

#[test]
fn integer_scientific_notation_is_accepted() {
    let d = Odin::parse("x = ##1e3\n").unwrap();
    assert_eq!(d.get_integer("x"), Some(1000));
}

// ─── @$.path meta reference ─────────────────────────────────────────────────

#[test]
fn meta_reference_with_dot_parses() {
    let d = Odin::parse("x = @$.id\n").unwrap();
    match d.get("x") {
        Some(crate::OdinValue::Reference { path, .. }) => assert_eq!(path, "$.id"),
        other => panic!("expected reference @$.id, got {other:?}"),
    }
}

#[test]
fn meta_reference_nested_path_parses() {
    let d = Odin::parse("x = @$.i18n.en.name\n").unwrap();
    match d.get("x") {
        Some(crate::OdinValue::Reference { path, .. }) => assert_eq!(path, "$.i18n.en.name"),
        other => panic!("expected reference @$.i18n.en.name, got {other:?}"),
    }
}

#[test]
fn const_reference_still_parses() {
    let d = Odin::parse("x = @$const.NAME\n").unwrap();
    match d.get("x") {
        Some(crate::OdinValue::Reference { path, .. }) => assert_eq!(path, "$const.NAME"),
        other => panic!("expected reference @$const.NAME, got {other:?}"),
    }
}

// ─── Document chain API ─────────────────────────────────────────────────────

#[test]
fn parse_documents_returns_full_chain() {
    let input = "{$}\nid = \"a\"\n\n{}\nx = \"1\"\n\n---\n\n{$}\nid = \"b\"\n\n{}\nx = \"2\"\n";
    let docs = Odin::parse_documents(input).unwrap();
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].get_string("$.id"), Some("a"));
    assert_eq!(docs[0].get_string("x"), Some("1"));
    assert_eq!(docs[1].get_string("$.id"), Some("b"));
    assert_eq!(docs[1].get_string("x"), Some("2"));
}

#[test]
fn parse_documents_single_document_yields_one() {
    let docs = Odin::parse_documents("name = \"solo\"\n").unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].get_string("name"), Some("solo"));
}



// ─── Bare segment-directive lines & header-inline loop/counter/from ──────────

#[test]
fn parse_bare_loop_and_counter_directives() {
    let d = Odin::parse("{rows[]}\n:loop items\n:counter idx\nn = \"x\"\n").unwrap();
    assert_eq!(d.get_string("rows[]._loop"), Some("items"));
    assert_eq!(d.get_string("rows[]._counter"), Some("idx"));
}

#[test]
fn parse_bare_from_directive() {
    let d = Odin::parse("{out}\n:from data.records\nv = \"x\"\n").unwrap();
    assert_eq!(d.get_string("out._from"), Some("data.records"));
}

#[test]
fn parse_bare_loop_with_alias() {
    let d = Odin::parse("{rows[]}\n:loop items :as item\n").unwrap();
    assert_eq!(d.get_string("rows[]._loop"), Some("items :as item"));
}

#[test]
fn parse_header_inline_loop_counter_from() {
    let d = Odin::parse("{rows[] :loop items}\nn = \"x\"\n").unwrap();
    assert_eq!(d.get_string("rows[]._loop"), Some("items"));
    let d2 = Odin::parse("{rows[] :counter idx}\nn = \"x\"\n").unwrap();
    assert_eq!(d2.get_string("rows[]._counter"), Some("idx"));
    let d3 = Odin::parse("{out :from data.records}\nv = \"x\"\n").unwrap();
    assert_eq!(d3.get_string("out._from"), Some("data.records"));
}

// ─── Prefixed reference coercion (##@path, #$@path) ──────────────────────────

#[test]
fn prefixed_integer_reference_carries_type_directive() {
    let d = Odin::parse("year = ##@.year\n").unwrap();
    match d.get("year") {
        Some(crate::OdinValue::Reference { path, directives, .. }) => {
            assert_eq!(path, ".year");
            assert_eq!(directives.len(), 1);
            assert_eq!(directives[0].name, "type");
        }
        other => panic!("expected reference with type directive, got {other:?}"),
    }
}

#[test]
fn prefixed_currency_reference_carries_type_directive() {
    let d = Odin::parse("premium = #$@.premium\n").unwrap();
    match d.get("premium") {
        Some(crate::OdinValue::Reference { path, directives, .. }) => {
            assert_eq!(path, ".premium");
            assert_eq!(directives[0].name, "type");
            match &directives[0].value {
                Some(crate::types::values::DirectiveValue::String(s)) => assert_eq!(s, "currency"),
                other => panic!("expected currency type value, got {other:?}"),
            }
        }
        other => panic!("expected reference with type directive, got {other:?}"),
    }
}

#[test]
fn prefixed_number_reference_carries_type_directive() {
    let d = Odin::parse("v = #@.x\n").unwrap();
    match d.get("v") {
        Some(crate::OdinValue::Reference { directives, .. }) => {
            match &directives[0].value {
                Some(crate::types::values::DirectiveValue::String(s)) => assert_eq!(s, "number"),
                other => panic!("expected number type value, got {other:?}"),
            }
        }
        other => panic!("expected reference, got {other:?}"),
    }
}

#[test]
fn bare_integer_prefix_still_parses_number() {
    let d = Odin::parse("year = ##2021\n").unwrap();
    match d.get("year") {
        Some(crate::OdinValue::Integer { value, .. }) => assert_eq!(*value, 2021),
        other => panic!("expected integer 2021, got {other:?}"),
    }
}

#[test]
fn empty_integer_prefix_still_errors() {
    assert!(Odin::parse("year = ##\n").is_err());
}

// ─── \$ / \${ escapes ────────────────────────────────────────────────────────

#[test]
fn dollar_escape_emits_literal_dollar() {
    let d = Odin::parse("x = \"price \\$5\"\n").unwrap();
    assert_eq!(d.get_string("x"), Some("price $5"));
}

#[test]
fn dollar_brace_escape_preserves_marker() {
    let d = Odin::parse("x = \"use \\${name}\"\n").unwrap();
    // The backslash is preserved before `${` so interpolation treats it as literal.
    assert_eq!(d.get_string("x"), Some("use \\${name}"));
}

#[test]
fn dollar_escape_before_interpolation_marker_drops_backslash() {
    // `\$` followed by a real `${...}` yields a literal `$` then the live marker.
    let d = Odin::parse("x = \"total \\$${amount}\"\n").unwrap();
    assert_eq!(d.get_string("x"), Some("total $${amount}"));
}
