//! Schema-validation enforcement: invariant evaluation, currency and percent
//! bounds, override restrictiveness, intersection conflicts, tabular columns,
//! and default-value rules.

use super::{schema_parser, validate};
use crate::types::schema::ValidationResult;

const H: &str = "{$}\nodin = \"1.0.0\"\nschema = \"1.0.0\"\n\n";

fn run(schema_body: &str, input: &str) -> ValidationResult {
    let schema = schema_parser::parse_schema(&format!("{H}{schema_body}")).unwrap();
    let doc = if input.is_empty() {
        crate::Odin::empty()
    } else {
        crate::Odin::parse(input).unwrap()
    };
    validate(&doc, &schema, None)
}

fn codes_at(r: &ValidationResult, path: &str) -> Vec<String> {
    r.errors.iter().filter(|e| e.path == path).map(|e| e.code().to_string()).collect()
}

// ── Invariant expression evaluation ──────────────────────────────────────────

#[test]
fn invariant_three_term_additive_passes() {
    let r = run(
        "{order}\nsubtotal = #$\ntax = #$\nshipping = #$\ntotal = #$\n:invariant total = subtotal + tax + shipping",
        "{order}\nsubtotal = #$10.00\ntax = #$1.00\nshipping = #$2.00\ntotal = #$13.00",
    );
    assert!(r.valid, "errors: {:?}", r.errors);
}

#[test]
fn invariant_three_term_additive_fails() {
    let r = run(
        "{order}\nsubtotal = #$\ntax = #$\nshipping = #$\ntotal = #$\n:invariant total = subtotal + tax + shipping",
        "{order}\nsubtotal = #$10.00\ntax = #$1.00\nshipping = #$2.00\ntotal = #$99.00",
    );
    assert!(!r.valid);
    assert!(codes_at(&r, "order").contains(&"V008".to_string()));
}

#[test]
fn invariant_parentheses_and_precedence() {
    let schema = "{discount}\nsubtotal = #$\npercentage = #\nfixed_amount = #$\ntotal = #$\n:invariant total = subtotal - (subtotal * percentage / 100) - fixed_amount";
    assert!(run(schema, "{discount}\nsubtotal = #$100.00\npercentage = #10\nfixed_amount = #$5.00\ntotal = #$85.00").valid);
    assert!(!run(schema, "{discount}\nsubtotal = #$100.00\npercentage = #10\nfixed_amount = #$5.00\ntotal = #$80.00").valid);
}

#[test]
fn invariant_logical_or() {
    let schema = "{discount}\npercentage = #\nfixed_amount = #$\n:invariant percentage == 0 || fixed_amount == 0";
    assert!(run(schema, "{discount}\npercentage = #0\nfixed_amount = #$5.00").valid);
    assert!(!run(schema, "{discount}\npercentage = #10\nfixed_amount = #$5.00").valid);
}

#[test]
fn invariant_logical_and_and_negation() {
    let schema = "{f}\na = #\nb = #\n:invariant !(a > 10) && b < 5";
    assert!(run(schema, "{f}\na = #3\nb = #2").valid);
    assert!(!run(schema, "{f}\na = #20\nb = #2").valid);
}

#[test]
fn invariant_modulo() {
    let schema = "{n}\nx = ##\n:invariant x % 2 == 0";
    assert!(run(schema, "{n}\nx = ##4").valid);
    assert!(!run(schema, "{n}\nx = ##5").valid);
}

#[test]
fn invariant_temporal_operands() {
    let schema = "{r}\nstart = date\nend = date\n:invariant end >= start";
    assert!(run(schema, "{r}\nstart = 2020-01-01\nend = 2020-02-01").valid);
    assert!(!run(schema, "{r}\nstart = 2020-03-01\nend = 2020-02-01").valid);
}

#[test]
fn invariant_null_operand_is_false() {
    let r = run(
        "{o}\ntotal = #$\nsubtotal = #$\ntax = ~#$\n:invariant total = subtotal + tax",
        "{o}\ntotal = #$10.00\nsubtotal = #$10.00\ntax = ~",
    );
    assert!(!r.valid);
    assert!(codes_at(&r, "o").contains(&"V008".to_string()));
}

#[test]
fn invariant_absent_operand_does_not_apply() {
    let r = run(
        "{o}\ntotal = #$\nsubtotal = #$\ntax = #$\n:invariant total = subtotal + tax",
        "{o}\ntotal = #$10.00",
    );
    assert!(r.valid, "errors: {:?}", r.errors);
}

#[test]
fn invariant_malformed_is_v008() {
    let r = run("{o}\nx = #\n:invariant x + + ", "{o}\nx = #1");
    assert!(!r.valid);
    assert!(codes_at(&r, "o").contains(&"V008".to_string()));
}

// ── Currency decimal-place enforcement ───────────────────────────────────────

#[test]
fn currency_places_accepts_declared() {
    assert!(run("{w}\nbtc = #$.8", "{w}\nbtc = #$1.00000000").valid);
}

#[test]
fn currency_places_rejects_too_few() {
    let r = run("{w}\nbtc = #$.8", "{w}\nbtc = #$1.00");
    assert!(!r.valid);
    assert!(codes_at(&r, "w.btc").contains(&"V003".to_string()));
}

#[test]
fn currency_defaults_to_two_places() {
    assert!(run("{w}\nprice = #$", "{w}\nprice = #$9.99").valid);
    assert!(!run("{w}\nprice = #$", "{w}\nprice = #$9.999").valid);
}

// ── Percent bounds enforcement ───────────────────────────────────────────────

#[test]
fn percent_bounds_accepts_in_range() {
    assert!(run("{r}\nrate = #%:(0..1)", "{r}\nrate = #%0.5").valid);
}

#[test]
fn percent_bounds_rejects_out_of_range() {
    let r = run("{r}\nrate = #%:(0..1)", "{r}\nrate = #%1.5");
    assert!(!r.valid);
    assert!(codes_at(&r, "r.rate").contains(&"V003".to_string()));
}

#[test]
fn percent_bounds_rejects_below_min() {
    assert!(!run("{r}\nrate = #%:(0.1..1)", "{r}\nrate = #%0.05").valid);
}

// ── Override restrictiveness ──────────────────────────────────────────────────

#[test]
fn override_narrow_bounds_accepted() {
    assert!(run("{@base}\namount = #$:(0..1000)\n\n{@narrow}\n= @base :override\namount = #$:(0..100)", "").valid);
}

#[test]
fn override_widen_bounds_v017() {
    let r = run("{@base}\namount = #$:(0..100)\n\n{@wide}\n= @base :override\namount = #$:(0..1000)", "");
    assert!(!r.valid);
    assert!(codes_at(&r, "@wide.amount").contains(&"V017".to_string()));
}

#[test]
fn override_optional_to_required_ok_reverse_fails() {
    assert!(run("{@base}\nname =\n\n{@d}\n= @base :override\nname = !", "").valid);
    let r = run("{@base}\nname = !\n\n{@d}\n= @base :override\nname =", "");
    assert!(!r.valid);
    assert!(codes_at(&r, "@d.name").contains(&"V017".to_string()));
}

#[test]
fn override_remove_nullable_ok_add_fails() {
    assert!(run("{@base}\nx = ~#\n\n{@d}\n= @base :override\nx = #", "").valid);
    let r = run("{@base}\nx = #\n\n{@d}\n= @base :override\nx = ~#", "");
    assert!(!r.valid);
    assert!(codes_at(&r, "@d.x").contains(&"V017".to_string()));
}

#[test]
fn override_change_type_v017() {
    let r = run("{@base}\nx = #\n\n{@d}\n= @base :override\nx =", "");
    assert!(!r.valid);
    assert!(codes_at(&r, "@d.x").contains(&"V017".to_string()));
}

#[test]
fn override_path_level_composition() {
    let r = run("{@base}\namount = #$:(0..100)\n\n{order}\n= @base :override\namount = #$:(0..1000)", "");
    assert!(!r.valid);
    assert!(codes_at(&r, "order.amount").contains(&"V017".to_string()));
}

#[test]
fn override_untouched_field_ok() {
    assert!(run("{@base}\na = #$:(0..100)\nb = !\n\n{@d}\n= @base :override\na = #$:(0..50)", "").valid);
}

// ── Intersection field conflicts ──────────────────────────────────────────────

#[test]
fn intersection_conflict_v017() {
    let r = run("{@a}\nx = !\n\n{@b}\nx = !##\n\n{cust}\n= @a & @b", "{cust}\nx = ##5");
    assert!(!r.valid);
    assert!(codes_at(&r, "@cust.x").contains(&"V017".to_string()));
}

#[test]
fn intersection_disjoint_or_identical_ok() {
    assert!(run("{@a}\nx = !\nname = !\n\n{@b}\nx = !\nage = !##\n\n{cust}\n= @a & @b", "{cust}\nx = \"hi\"\nname = \"n\"\nage = ##5").valid);
}

#[test]
fn intersection_three_way_conflict() {
    let r = run("{@a}\nx = !\n\n{@b}\ny = !\n\n{@c}\nx = !##\n\n{cust}\n= @a & @b & @c", "{cust}\nx = \"hi\"\ny = \"z\"");
    assert!(!r.valid);
    assert!(codes_at(&r, "@cust.x").contains(&"V017".to_string()));
}

// ── Tabular column rules ──────────────────────────────────────────────────────

#[test]
fn tabular_primitive_columns_ok() {
    assert!(run("{contacts[] : name, email}\nname = !\nemail = !", "{contacts[0]}\nname = \"a\"\nemail = \"b\"").valid);
}

#[test]
fn tabular_typeref_column_v017() {
    let r = run("{@addr}\nline1 = !\n\n{customers[] : name, address}\nname = !\naddress = @addr", "{customers[0]}\nname = \"a\"");
    assert!(!r.valid);
    assert!(codes_at(&r, "customers[].address").contains(&"V017".to_string()));
}

#[test]
fn tabular_single_level_dotted_ok() {
    assert!(run("{rows[] : id, label}\nid = !##\nlabel = !", "{rows[0]}\nid = ##1\nlabel = \"x\"").valid);
}

// ── Default value rules ───────────────────────────────────────────────────────

#[test]
fn default_within_constraints_on_optional_ok() {
    assert!(run("{root}\npriority = ##:(1..5) ##3", "").valid);
}

#[test]
fn default_on_required_v017() {
    let r = run("{root}\nstatus = !(\"a\", \"b\") \"a\"", "{root}\nstatus = \"a\"");
    assert!(!r.valid);
    assert!(codes_at(&r, "root.status").contains(&"V017".to_string()));
}

#[test]
fn default_violates_bounds_v017() {
    let r = run("{root}\npriority = ##:(1..5) ##9", "");
    assert!(!r.valid);
    assert!(codes_at(&r, "root.priority").contains(&"V017".to_string()));
}

#[test]
fn default_violates_enum_v017() {
    let r = run("{root}\nstatus = (\"a\", \"b\") \"c\"", "");
    assert!(!r.valid);
    assert!(codes_at(&r, "root.status").contains(&"V017".to_string()));
}

#[test]
fn default_matches_enum_ok() {
    assert!(run("{root}\nstatus = (\"a\", \"b\") \"a\"", "").valid);
}
