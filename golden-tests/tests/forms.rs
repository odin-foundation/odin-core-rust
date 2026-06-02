//! Golden forms tests — reads cases from sdk/golden/forms/ and runs them
//! against the Rust forms parser and renderer.

use odin_core::forms::{parse_form, render_form, FormElement, OdinForm};
use odin_core::Odin;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Deserialize)]
struct TestSuite {
    suite: String,
    tests: Vec<TestCase>,
}

#[derive(Deserialize)]
struct TestCase {
    id: String,
    #[serde(rename = "formFile")]
    form_file: String,
    #[serde(default, rename = "renderData")]
    render_data: Option<String>,
    #[serde(default, rename = "expectParse")]
    expect_parse: Option<ExpectParse>,
    #[serde(default, rename = "renderContains")]
    render_contains: Option<Vec<String>>,
    #[serde(default, rename = "renderNotContains")]
    render_not_contains: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ExpectParse {
    #[serde(default)]
    pages: Option<usize>,
    #[serde(default)]
    margins: Option<Margins>,
    #[serde(default)]
    templates: Option<HashMap<String, ExpectTemplate>>,
    #[serde(default)]
    page0: Option<ExpectPage>,
}

#[derive(Deserialize)]
struct Margins {
    #[serde(default)]
    top: Option<f64>,
    #[serde(default)]
    right: Option<f64>,
    #[serde(default)]
    bottom: Option<f64>,
    #[serde(default)]
    left: Option<f64>,
}

#[derive(Deserialize)]
struct ExpectTemplate {
    #[serde(default, rename = "pageTemplate")]
    page_template: Option<bool>,
    #[serde(default)]
    continues: Option<String>,
    #[serde(default, rename = "formId")]
    form_id: Option<String>,
    #[serde(default, rename = "elementTypes")]
    element_types: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ExpectPage {
    #[serde(default, rename = "elementTypes")]
    element_types: Option<Vec<String>>,
    #[serde(default)]
    elements: Option<HashMap<String, ExpectElement>>,
}

#[derive(Deserialize)]
struct ExpectElement {
    #[serde(default, rename = "type")]
    type_str: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default, rename = "inputType")]
    input_type: Option<String>,
    #[serde(default)]
    checked: Option<bool>,
    #[serde(default)]
    selected: Option<String>,
    #[serde(default)]
    options: Option<Vec<String>>,
    #[serde(default)]
    min: Option<serde_json::Value>,
    #[serde(default)]
    max: Option<serde_json::Value>,
    #[serde(default)]
    overflow: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    background: Option<bool>,
    #[serde(default, rename = "barcodeType")]
    barcode_type: Option<String>,
    #[serde(default)]
    bind: Option<String>,
    #[serde(default, rename = "childCount")]
    child_count: Option<usize>,
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("golden")
        .join("forms")
}

fn find_element<'a>(form: &'a OdinForm, name: &str) -> &'a FormElement {
    form.pages[0]
        .elements
        .iter()
        .find(|e| e.name() == name)
        .unwrap_or_else(|| panic!("page0 element {name} not found"))
}

fn assert_element(form: &OdinForm, name: &str, expect: &ExpectElement) {
    let el = find_element(form, name);

    if let Some(t) = &expect.type_str {
        assert_eq!(el.type_str(), t, "element {name} type mismatch");
    }
    if let Some(bg) = expect.background {
        if let FormElement::Image(img) = el {
            assert_eq!(img.background, Some(bg), "element {name} background");
        } else {
            panic!("element {name} expected image");
        }
    }
    if let Some(bt) = &expect.barcode_type {
        if let FormElement::Barcode(b) = el {
            assert_eq!(b.barcode_type.as_str(), bt, "element {name} barcode type");
        } else {
            panic!("element {name} expected barcode");
        }
    }
    if let Some(label) = &expect.label {
        let field = el.as_field().unwrap_or_else(|| panic!("element {name} not a field"));
        assert_eq!(&field.label, label, "element {name} label");
    }

    match el {
        FormElement::TextField(f) => {
            if let Some(v) = &expect.value {
                assert_eq!(f.value.as_deref(), Some(v.as_str()), "{name} value");
            }
            if let Some(it) = &expect.input_type {
                assert_eq!(
                    f.input_type.map(odin_core::forms::InputType::as_str),
                    Some(it.as_str()),
                    "{name} inputType"
                );
            }
        }
        FormElement::Checkbox(f) => {
            if let Some(c) = expect.checked {
                assert_eq!(f.checked, Some(c), "{name} checked");
            }
        }
        FormElement::Date(f) => {
            if let Some(v) = &expect.value {
                assert_eq!(f.value.as_deref(), Some(v.as_str()), "{name} value");
            }
            if let Some(serde_json::Value::String(m)) = &expect.min {
                assert_eq!(f.field.validation.min.as_deref(), Some(m.as_str()), "{name} min");
            }
            if let Some(serde_json::Value::String(m)) = &expect.max {
                assert_eq!(f.field.validation.max.as_deref(), Some(m.as_str()), "{name} max");
            }
        }
        FormElement::Select(f) => {
            if let Some(v) = &expect.selected {
                assert_eq!(f.selected.as_deref(), Some(v.as_str()), "{name} selected");
            }
            if let Some(opts) = &expect.options {
                assert_eq!(&f.options, opts, "{name} options");
            }
        }
        FormElement::Radio(f) => {
            if let Some(v) = &expect.value {
                assert_eq!(&f.value, v, "{name} radio value");
            }
        }
        FormElement::Signature(f) => {
            if let Some(v) = &expect.value {
                assert_eq!(f.value.as_deref(), Some(v.as_str()), "{name} signature value");
            }
        }
        FormElement::Region(r) => {
            if let Some(b) = &expect.bind {
                assert_eq!(r.bind.as_deref(), Some(b.as_str()), "{name} bind");
            }
            if let Some(serde_json::Value::Number(m)) = &expect.max {
                assert_eq!(r.max, m.as_i64(), "{name} region max");
            }
            if let Some(o) = &expect.overflow {
                assert_eq!(r.overflow.as_deref(), Some(o.as_str()), "{name} overflow");
            }
            if let Some(cc) = expect.child_count {
                assert_eq!(r.children.len(), cc, "{name} childCount");
            }
        }
        _ => {}
    }
}

fn run_case(case: &TestCase, dir: &std::path::Path) {
    let form_path = dir.join(&case.form_file);
    let text = std::fs::read_to_string(&form_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", form_path.display()));
    let form = parse_form(&text)
        .unwrap_or_else(|e| panic!("parse {} ({}): {e:?}", case.id, case.form_file));

    if let Some(expect) = &case.expect_parse {
        if let Some(pages) = expect.pages {
            assert_eq!(form.pages.len(), pages, "{} page count", case.id);
        }
        if let Some(margins) = &expect.margins {
            let actual = form
                .page_defaults
                .as_ref()
                .and_then(|p| p.margin.as_ref())
                .unwrap_or_else(|| panic!("{} expected margins", case.id));
            assert_eq!(actual.top, margins.top, "{} margin.top", case.id);
            assert_eq!(actual.right, margins.right, "{} margin.right", case.id);
            assert_eq!(actual.bottom, margins.bottom, "{} margin.bottom", case.id);
            assert_eq!(actual.left, margins.left, "{} margin.left", case.id);
        }
        if let Some(templates) = &expect.templates {
            let parsed = form
                .templates
                .as_ref()
                .unwrap_or_else(|| panic!("{} expected templates", case.id));
            for (tpl_name, tpl_expect) in templates {
                let tpl = parsed
                    .get(tpl_name)
                    .unwrap_or_else(|| panic!("{} template {tpl_name} missing", case.id));
                if let Some(pt) = tpl_expect.page_template {
                    assert_eq!(tpl.page_template, pt, "{} {tpl_name} pageTemplate", case.id);
                }
                if let Some(c) = &tpl_expect.continues {
                    assert_eq!(tpl.continues.as_deref(), Some(c.as_str()), "{} {tpl_name} continues", case.id);
                }
                if let Some(fid) = &tpl_expect.form_id {
                    assert_eq!(tpl.form_id.as_deref(), Some(fid.as_str()), "{} {tpl_name} formId", case.id);
                }
                if let Some(types) = &tpl_expect.element_types {
                    let actual: Vec<&str> = tpl.elements.iter().map(FormElement::type_str).collect();
                    assert_eq!(&actual, types, "{} {tpl_name} elementTypes", case.id);
                }
            }
        }
        if let Some(page0) = &expect.page0 {
            if let Some(types) = &page0.element_types {
                let actual: Vec<&str> = form.pages[0]
                    .elements
                    .iter()
                    .map(FormElement::type_str)
                    .collect();
                assert_eq!(&actual, types, "{} page0 elementTypes", case.id);
            }
            if let Some(elements) = &page0.elements {
                for (name, expect_el) in elements {
                    assert_element(&form, name, expect_el);
                }
            }
        }
    }

    if case.render_contains.is_some() || case.render_not_contains.is_some() {
        let data = case
            .render_data
            .as_ref()
            .map(|d| Odin::parse(d).unwrap_or_else(|e| panic!("{} renderData: {e:?}", case.id)));
        let html = render_form(&form, data.as_ref());

        if let Some(contains) = &case.render_contains {
            for needle in contains {
                assert!(
                    html.contains(needle),
                    "{} render should contain `{needle}`\n--- html ---\n{html}",
                    case.id
                );
            }
        }
        if let Some(not_contains) = &case.render_not_contains {
            for needle in not_contains {
                assert!(
                    !html.contains(needle),
                    "{} render should not contain `{needle}`",
                    case.id
                );
            }
        }
    }
}

#[test]
fn golden_forms_suite() {
    let dir = golden_dir();
    let manifest_path = dir.join("manifest.json");
    let content = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest_path.display()));
    let suite: TestSuite = serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("parse manifest: {e}"));
    assert_eq!(suite.suite, "forms");

    let mut passed = 0;
    for case in &suite.tests {
        run_case(case, &dir);
        passed += 1;
    }
    assert_eq!(passed, suite.tests.len());
    println!("forms golden: {passed}/{} passed", suite.tests.len());
}
