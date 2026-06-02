//! Unit tests for ODIN Forms parsing and rendering.

use super::*;
use crate::Odin;

const INLINE_VALUES: &str = r####"{$}
odin = "1.0.0"
forms = "1.0.0"
title = "Inline Values"
id = "inline_form"
lang = "en"

{$.page}
width = #8.5
height = #11
unit = "inch"

{page[0]}
{.field.name}
type = "text"
x = #0.6
y = #1.55
w = #3.5
h = #0.3
label = "Full Name"
value = "John Smith"
inputType = "email"
bind = @insured.name

{.field.agree}
type = "checkbox"
x = #0.5
y = #8
w = #0.2
h = #0.2
label = "I agree"
checked = ?true
bind = @application.termsAccepted

{.field.dob}
type = "date"
x = #4
y = #2.25
w = #1.5
h = #0.25
label = "Date of Birth"
value = 1985-03-15
min = 1900-01-01
max = 2010-01-01
bind = @insured.birthDate

{.field.state}
type = "select"
x = #4
y = #3
w = #1.5
h = #0.3
label = "State"
selected = "TX"
bind = @insured.address.state

{.field.state.options[] : ~}
"AL"
"CA"
"NY"
"TX"
"####;

const GEOMETRIC: &str = r####"{$}
odin = "1.0.0"
forms = "1.0.0"
title = "Geometric Elements"
id = "geometric_form"
lang = "en"

{$.page}
width = #8.5
height = #11
unit = "inch"

{page[0]}
{.circle.seal}
cx = #2
cy = #2
r = #0.75
stroke = "#003366"
stroke-width = #0.02
fill = "#e6f0ff"

{.ellipse.stamp}
cx = #5
cy = #2
rx = #1
ry = #0.5
stroke = "#660000"
stroke-width = #0.02
fill = "none"

{.polygon.badge}
points = "1,4 2,4 2.5,5 1.5,5.5 0.5,5"
stroke = "#000000"
stroke-width = #0.01
fill = "#fff8e6"

{.polyline.trend}
points = "3,4 3.5,4.6 4,4.2 4.5,5 5,4.3"
stroke = "#006600"
stroke-width = #0.02

{.path.arrow}
d = "M 6,4 L 7,4 L 6.5,5 Z"
stroke = "#333333"
stroke-width = #0.01
fill = "#cccccc"
"####;

const CONTENT: &str = r####"{$}
odin = "1.0.0"
forms = "1.0.0"
title = "Content Elements"
id = "content_form"
lang = "en"

{$.i18n}
en.field_name = "Full Legal Name"

{$.page}
width = #8.5
height = #11
unit = "inch"
margin.top = #0.5
margin.right = #0.25
margin.bottom = #0.6
margin.left = #0.75

{page[0]}
{.img.template}
x = #0
y = #0
w = #8.5
h = #11
src = ^png:iVBORw0KGgo=
alt = "Page template"
background = ?true

{.barcode.doc}
x = #7
y = #0.5
w = #1
h = #1
type = "qr"
content = "DOC-2024-001234"
alt = "Document tracking code"

{.field.name}
type = "text"
x = #0.6
y = #1.55
w = #3.5
h = #0.3
label = @$.i18n.en.field_name
bind = @insured.name
"####;

const SIGNATURE_RADIO: &str = r####"{$}
odin = "1.0.0"
forms = "1.0.0"
title = "Signature and Radio"
id = "sigradio_form"
lang = "en"

{$.page}
width = #8.5
height = #11
unit = "inch"

{page[0]}
{.field.gender_m}
type = "radio"
x = #0.6
y = #2
w = #0.2
h = #0.2
label = "Male"
group = "gender"
value = "M"
bind = @applicant.gender

{.field.gender_f}
type = "radio"
x = #1.6
y = #2
w = #0.2
h = #0.2
label = "Female"
group = "gender"
value = "F"
bind = @applicant.gender

{.field.gender_x}
type = "radio"
x = #2.6
y = #2
w = #0.2
h = #0.2
label = "Nonbinary"
group = "gender"
value = "X"
bind = @applicant.gender

{.field.applicant_sig}
type = "signature"
x = #0.6
y = #8
w = #3
h = #0.6
label = "Applicant Signature"
required = ?true
value = ^png:iVBORw0KGgo=
date_field = @page[0].field.sig_date
bind = @applicant.signature

{.field.sig_date}
type = "date"
x = #4
y = #8
w = #1.5
h = #0.3
label = "Date Signed"
value = 2026-01-15
bind = @applicant.signatureDate
"####;

const PAGE_TEMPLATE: &str = r####"{$}
odin = "1.0.0"
forms = "1.0.0"
title = "Template Form"
id = "tpl_form"
lang = "en"

{$.page}
width = #8.5
height = #11
unit = "inch"

{page[0]}
{.text.header}
x = #0.5
y = #0.5
content = "Vehicles — Page {@odin.page} of {@odin.total_pages}"
font-size = ##14
font-weight = "bold"

{.region.vehicles}
x = #0.5
y = #1.2
w = #7.5
h = #6
bind = @policy.vehicles
max = ##3
overflow = @tpl_vehicles_continued

{.region.vehicles.field.vin}
x = #0
y = #0.15
y-offset = #1.8
w = #4
h = #0.3
label = "VIN"
bind = @.vin

{@tpl_vehicles_continued}
page-template = ?true
continues = "region.vehicles"
form-id = "PA (Cont)"

{.text.header}
x = #0.5
y = #0.5
content = "Additional Vehicles — Page {@odin.page} of {@odin.total_pages}"
font-size = ##14
font-weight = "bold"

{.region.vehicles}
x = #0.5
y = #1
w = #7.5
h = #8
max = ##4
overflow = @tpl_vehicles_continued

{.region.vehicles.field.vin}
x = #0
y = #0.15
y-offset = #1.2
w = #4
h = #0.3
label = "VIN"
bind = @.vin
"####;

fn page0(form: &OdinForm) -> &FormPage {
    &form.pages[0]
}

fn find<'a>(page: &'a FormPage, name: &str) -> &'a FormElement {
    page.elements
        .iter()
        .find(|e| e.name() == name)
        .unwrap_or_else(|| panic!("element {name} not found"))
}

#[test]
fn parses_inline_text_field() {
    let form = parse_form(INLINE_VALUES).unwrap();
    let page = page0(&form);
    let FormElement::TextField(f) = find(page, "name") else {
        panic!("expected text field");
    };
    assert_eq!(f.value.as_deref(), Some("John Smith"));
    assert_eq!(f.input_type, Some(InputType::Email));
    assert_eq!(f.field.bind, "@insured.name");
}

#[test]
fn parses_inline_checkbox() {
    let form = parse_form(INLINE_VALUES).unwrap();
    let FormElement::Checkbox(f) = find(page0(&form), "agree") else {
        panic!("expected checkbox");
    };
    assert_eq!(f.checked, Some(true));
}

#[test]
fn parses_date_with_min_max() {
    let form = parse_form(INLINE_VALUES).unwrap();
    let FormElement::Date(f) = find(page0(&form), "dob") else {
        panic!("expected date");
    };
    assert_eq!(f.value.as_deref(), Some("1985-03-15"));
    assert_eq!(f.field.validation.min.as_deref(), Some("1900-01-01"));
    assert_eq!(f.field.validation.max.as_deref(), Some("2010-01-01"));
}

#[test]
fn parses_select_options_and_selected() {
    let form = parse_form(INLINE_VALUES).unwrap();
    let FormElement::Select(f) = find(page0(&form), "state") else {
        panic!("expected select");
    };
    assert_eq!(f.selected.as_deref(), Some("TX"));
    assert_eq!(f.options, vec!["AL", "CA", "NY", "TX"]);
}

#[test]
fn renders_inline_values_html() {
    let form = parse_form(INLINE_VALUES).unwrap();
    let html = render_form(&form, None);
    assert!(html.contains("type=\"email\""));
    assert!(html.contains("value=\"John Smith\""));
    assert!(html.contains("value=\"1985-03-15\""));
    assert!(html.contains("<option value=\"TX\" selected>"));
}

#[test]
fn parses_geometric_elements() {
    let form = parse_form(GEOMETRIC).unwrap();
    let page = page0(&form);
    let types: Vec<&str> = page.elements.iter().map(FormElement::type_str).collect();
    assert_eq!(types, vec!["circle", "ellipse", "polygon", "polyline", "path"]);
}

#[test]
fn renders_geometric_shapes() {
    let form = parse_form(GEOMETRIC).unwrap();
    let html = render_form(&form, None);
    assert!(html.contains("<circle cx=\"192\" cy=\"192\" r=\"72\" stroke=\"#003366\""));
    assert!(html.contains("<ellipse cx=\"480\" cy=\"192\" rx=\"96\" ry=\"48\" stroke=\"#660000\""));
    assert!(html.contains("<polygon points=\"96,384 192,384 240,480 144,528 48,480\" stroke=\"#000000\""));
    assert!(html.contains("<polyline points=\"288,384 336,441.6 384,403.2 432,480 480,412.8\" stroke=\"#006600\" stroke-width=\"1.92\" fill=\"none\"/>"));
    assert!(html.contains("<path d=\"M 6,4 L 7,4 L 6.5,5 Z\" stroke=\"#333333\""));
}

#[test]
fn parses_content_elements_and_margins() {
    let form = parse_form(CONTENT).unwrap();
    let margins = form
        .page_defaults
        .as_ref()
        .and_then(|p| p.margin.as_ref())
        .unwrap();
    assert_eq!(margins.top, Some(0.5));
    assert_eq!(margins.right, Some(0.25));
    assert_eq!(margins.bottom, Some(0.6));
    assert_eq!(margins.left, Some(0.75));

    let page = page0(&form);
    let FormElement::Image(img) = find(page, "template") else {
        panic!("expected image");
    };
    assert_eq!(img.background, Some(true));

    let FormElement::Barcode(bc) = find(page, "doc") else {
        panic!("expected barcode");
    };
    assert_eq!(bc.barcode_type, BarcodeType::Qr);

    let FormElement::TextField(name) = find(page, "name") else {
        panic!("expected text field");
    };
    assert_eq!(name.field.label, "Full Legal Name");
}

#[test]
fn renders_content_elements() {
    let form = parse_form(CONTENT).unwrap();
    let html = render_form(&form, None);
    assert!(html.contains("z-index:0;"));
    assert!(html.contains("data:image/png;base64,"));
    assert!(html.contains("data-barcode-type=\"qr\""));
    assert!(html.contains("Full Legal Name"));
}

#[test]
fn parses_radio_and_signature() {
    let form = parse_form(SIGNATURE_RADIO).unwrap();
    let page = page0(&form);
    let FormElement::Radio(m) = find(page, "gender_m") else {
        panic!("expected radio");
    };
    assert_eq!(m.value, "M");
    let FormElement::Radio(f) = find(page, "gender_f") else {
        panic!("expected radio");
    };
    assert_eq!(f.value, "F");

    let FormElement::Signature(sig) = find(page, "applicant_sig") else {
        panic!("expected signature");
    };
    assert_eq!(sig.value.as_deref(), Some("^png:iVBORw0KGgo="));

    let FormElement::Date(d) = find(page, "sig_date") else {
        panic!("expected date");
    };
    assert_eq!(d.value.as_deref(), Some("2026-01-15"));
}

#[test]
fn renders_radio_group_with_bound_selection() {
    let form = parse_form(SIGNATURE_RADIO).unwrap();
    let data = Odin::parse("{applicant}\ngender = \"F\"").unwrap();
    let html = render_form(&form, Some(&data));
    assert!(html.contains("<input type=\"radio\" class=\"odin-form-radio\" id=\"odin-field-0-gender_m\" name=\"gender\" value=\"M\""));
    assert!(html.contains("name=\"gender\" value=\"F\" aria-label=\"Female\" checked>"));
    assert!(html.contains("name=\"gender\" value=\"X\" aria-label=\"Nonbinary\">"));
    assert!(html.contains("<div class=\"odin-form-signature\" id=\"odin-field-0-applicant_sig\" aria-label=\"Applicant Signature\" aria-required=\"true\" role=\"img\" tabindex=\"0\""));
    assert!(html.contains("value=\"2026-01-15\""));
    assert!(!html.contains("name=\"gender\" value=\"M\" aria-label=\"Male\" checked"));
}

#[test]
fn parses_page_template() {
    let form = parse_form(PAGE_TEMPLATE).unwrap();
    assert_eq!(form.pages.len(), 1);
    let templates = form.templates.as_ref().unwrap();
    let tpl = templates.get("tpl_vehicles_continued").unwrap();
    assert!(tpl.page_template);
    assert_eq!(tpl.continues.as_deref(), Some("region.vehicles"));
    assert_eq!(tpl.form_id.as_deref(), Some("PA (Cont)"));
    let tpl_types: Vec<&str> = tpl.elements.iter().map(FormElement::type_str).collect();
    assert_eq!(tpl_types, vec!["text", "region"]);

    let page = page0(&form);
    let page_types: Vec<&str> = page.elements.iter().map(FormElement::type_str).collect();
    assert_eq!(page_types, vec!["text", "region"]);

    let FormElement::Region(region) = find(page, "vehicles") else {
        panic!("expected region");
    };
    assert_eq!(region.bind.as_deref(), Some("@policy.vehicles"));
    assert_eq!(region.max, Some(3));
    assert_eq!(region.overflow.as_deref(), Some("@tpl_vehicles_continued"));
    assert_eq!(region.children.len(), 1);
}

#[test]
fn renders_region_overflow() {
    let form = parse_form(PAGE_TEMPLATE).unwrap();
    let data = Odin::parse(
        "{policy}\n{.vehicles[0]}\nvin = \"V0\"\n{.vehicles[1]}\nvin = \"V1\"\n{.vehicles[2]}\nvin = \"V2\"\n{.vehicles[3]}\nvin = \"V3\"\n{.vehicles[4]}\nvin = \"V4\"",
    )
    .unwrap();
    let html = render_form(&form, Some(&data));
    assert!(html.contains("Page 1 of 2"));
    assert!(html.contains("Page 2 of 2"));
    assert!(html.contains("value=\"V0\""));
    assert!(html.contains("value=\"V3\""));
    assert!(html.contains("value=\"V4\""));
    assert!(!html.contains("{@odin.page}"));
    assert!(!html.contains("{@odin.total_pages}"));
}

#[test]
fn metadata_extracted() {
    let form = parse_form(INLINE_VALUES).unwrap();
    assert_eq!(form.metadata.title, "Inline Values");
    assert_eq!(form.metadata.id, "inline_form");
    assert_eq!(form.metadata.lang, "en");
    assert_eq!(form.metadata.version.as_deref(), Some("1.0.0"));
}

#[test]
fn css_is_scoped() {
    let css = generate_form_css();
    assert!(css.contains(".odin-form-page"));
    let print = generate_print_css();
    assert!(print.contains("@media print"));
}

#[test]
fn pixel_conversions_round_trip() {
    assert_eq!(to_pixels(1.0, "inch"), 96.0);
    assert_eq!(from_pixels(96.0, "inch"), 1.0);
}
