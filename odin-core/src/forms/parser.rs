//! Parser from ODIN Forms text into the typed `OdinForm` model.

use std::collections::BTreeMap;

use crate::types::document::OdinDocument;
use crate::types::values::OdinValue;
use crate::Odin;

use super::types::{
    BarcodeElement, BarcodeType, CheckboxElement, CircleElement, DateElement, ElementBase,
    EllipseElement, Fill, Font, FormElement, FormMetadata, FormPage, ImageElement, InputType,
    LineElement, MultiselectElement, OdinForm, PageDefaults, PageMargins, PageTemplate,
    PathElement, PolygonElement, PolylineElement, RadioElement, RectElement, RegionElement,
    ScreenSettings, SelectElement, SignatureElement, Stroke, TextElement, TextFieldElement, Unit,
    Validation, FieldBase,
};

/// Parse an ODIN forms document into a typed `OdinForm`.
///
/// # Errors
///
/// Returns a `ParseError` if the document body is not valid ODIN.
pub fn parse_form(text: &str) -> Result<OdinForm, crate::ParseError> {
    let (body, template_blocks) = split_templates(text);

    let doc = Odin::parse(&body)?;

    let metadata = extract_metadata(&doc);
    let page_defaults = extract_page_defaults(&doc);
    let screen = extract_screen(&doc);
    let i18n = extract_i18n(&doc);
    let pages = extract_pages(&doc, i18n.as_ref());
    let templates = extract_templates(&template_blocks, i18n.as_ref())?;

    Ok(OdinForm {
        metadata,
        page_defaults,
        screen,
        i18n,
        pages,
        templates,
    })
}

// ── Page template extraction ────────────────────────────────────────────────

struct TemplateBlock {
    name: String,
    text: String,
}

/// Whether a line is a `{@tpl_*}` template header; returns the template name.
fn match_tpl_header(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('{')?.strip_suffix('}')?.trim();
    let name = inner.strip_prefix('@')?;
    if name.starts_with("tpl_")
        && name.len() > 4
        && name[4..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        Some(name.to_string())
    } else {
        None
    }
}

/// Whether a line opens a top-level section (`{$...}`, `{page[N]}`, `{@tpl_...}`).
fn is_top_level_header(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix('{') else {
        return false;
    };
    let rest = rest.trim_start();
    rest.starts_with('$') || rest.starts_with("@tpl_") || page_index_at_start(rest).is_some()
}

/// Parse a leading `page[N]` token, returning the index when present.
fn page_index_at_start(s: &str) -> Option<i64> {
    let rest = s.strip_prefix("page[")?;
    let end = rest.find(']')?;
    rest[..end].parse::<i64>().ok()
}

/// Split a forms document into its core-parseable body and the raw text of each
/// `{@tpl_*}` block.
fn split_templates(text: &str) -> (String, Vec<TemplateBlock>) {
    let mut body_lines: Vec<&str> = Vec::new();
    let mut blocks: Vec<TemplateBlock> = Vec::new();
    let mut in_template = false;

    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(name) = match_tpl_header(line) {
            blocks.push(TemplateBlock {
                name,
                text: String::new(),
            });
            in_template = true;
            continue;
        }
        if in_template {
            if is_top_level_header(line) {
                in_template = false;
                body_lines.push(line);
            } else if let Some(last) = blocks.last_mut() {
                last.text.push_str(line);
                last.text.push('\n');
            }
            continue;
        }
        body_lines.push(line);
    }

    (reanchor(&body_lines.join("\n"), None), blocks)
}

/// Parse a leading anchor header token (`page[N]` or `tpl.name`).
fn anchor_header_token(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('{')?.strip_suffix('}')?.trim();
    if let Some(idx) = page_index_at_start(inner) {
        // Ensure the whole inner is just page[N].
        if inner == format!("page[{idx}]") {
            return Some(inner.to_string());
        }
        return None;
    }
    if let Some(rest) = inner.strip_prefix("tpl.") {
        if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Some(inner.to_string());
        }
    }
    None
}

/// Re-emit the active top-level anchor after each relative tabular header so
/// sibling relative headers resolve under the correct parent.
fn reanchor(text: &str, root_anchor: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut anchor: Option<String> = root_anchor.map(ToString::to_string);
    let mut needs_reanchor = false;

    for line in text.split('\n') {
        if let Some(tok) = anchor_header_token(line) {
            anchor = Some(format!("{{{tok}}}"));
            needs_reanchor = false;
            out.push(line.to_string());
            continue;
        }

        let trimmed = line.trim_start();
        if trimmed.starts_with("{.") {
            if needs_reanchor {
                if let Some(a) = &anchor {
                    out.push(a.clone());
                }
                needs_reanchor = false;
            }
            if is_relative_tabular(trimmed) {
                needs_reanchor = true;
            }
            out.push(line.to_string());
            continue;
        }

        out.push(line.to_string());
    }

    out.join("\n")
}

/// Whether a relative header opens a tabular array (`{.x[] : ...}`).
fn is_relative_tabular(line: &str) -> bool {
    let Some(close) = line.find('}') else {
        return false;
    };
    let inner = &line[..close];
    if let Some(bracket) = inner.find("[]") {
        let after = inner[bracket + 2..].trim_start();
        return after.starts_with(':');
    }
    false
}

/// Parse each template block body into a `PageTemplate`.
fn extract_templates(
    blocks: &[TemplateBlock],
    i18n: Option<&BTreeMap<String, String>>,
) -> Result<Option<BTreeMap<String, PageTemplate>>, crate::ParseError> {
    if blocks.is_empty() {
        return Ok(None);
    }

    let mut templates: BTreeMap<String, PageTemplate> = BTreeMap::new();
    for block in blocks {
        let root = format!("tpl.{}", block.name);
        let synthetic = reanchor(
            &format!("{{{root}}}\n{}", block.text),
            Some(&format!("{{{root}}}")),
        );
        let doc = Odin::parse(&synthetic)?;

        let prefix = format!("{root}.");
        let page_template = get_boolean(&doc, &format!("{prefix}page-template")).unwrap_or(true);
        let continues = get_string(&doc, &format!("{prefix}continues"));
        let form_id = get_string(&doc, &format!("{prefix}form-id"));
        let elements = extract_elements(&doc, &prefix, i18n);

        templates.insert(
            block.name.clone(),
            PageTemplate {
                name: block.name.clone(),
                page_template,
                continues,
                form_id,
                elements,
            },
        );
    }
    Ok(Some(templates))
}

// ── Metadata and settings ───────────────────────────────────────────────────

fn extract_metadata(doc: &OdinDocument) -> FormMetadata {
    FormMetadata {
        title: meta_string(doc, "title").unwrap_or_default(),
        id: meta_string(doc, "id").unwrap_or_default(),
        lang: meta_string(doc, "lang").unwrap_or_else(|| "en".to_string()),
        version: meta_string(doc, "forms"),
    }
}

fn extract_page_defaults(doc: &OdinDocument) -> Option<PageDefaults> {
    let width = meta_number(doc, ".page.width");
    let height = meta_number(doc, ".page.height");
    let unit = meta_string(doc, ".page.unit");
    let margin = extract_margins(doc);

    if width.is_none() && height.is_none() && unit.is_none() {
        return None;
    }

    Some(PageDefaults {
        width: width.unwrap_or(8.5),
        height: height.unwrap_or(11.0),
        unit: unit.map_or(Unit::Inch, |u| Unit::parse(&u)),
        margin,
    })
}

fn extract_margins(doc: &OdinDocument) -> Option<PageMargins> {
    let top = meta_number(doc, ".page.margin.top");
    let right = meta_number(doc, ".page.margin.right");
    let bottom = meta_number(doc, ".page.margin.bottom");
    let left = meta_number(doc, ".page.margin.left");
    if top.is_none() && right.is_none() && bottom.is_none() && left.is_none() {
        return None;
    }
    Some(PageMargins {
        top,
        right,
        bottom,
        left,
    })
}

fn extract_screen(doc: &OdinDocument) -> Option<ScreenSettings> {
    meta_number(doc, ".screen.scale").map(|scale| ScreenSettings { scale })
}

fn extract_i18n(doc: &OdinDocument) -> Option<BTreeMap<String, String>> {
    let mut result: BTreeMap<String, String> = BTreeMap::new();
    for (key, value) in doc.metadata.iter() {
        if let Some(label_key) = key.strip_prefix(".i18n.") {
            if let OdinValue::String { value: s, .. } = value {
                if !label_key.is_empty() {
                    result.insert(label_key.to_string(), s.clone());
                }
            }
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

// ── Pages and elements ──────────────────────────────────────────────────────

fn extract_pages(doc: &OdinDocument, i18n: Option<&BTreeMap<String, String>>) -> Vec<FormPage> {
    let mut indices: Vec<i64> = Vec::new();
    for path in doc.paths() {
        if let Some(rest) = path.strip_prefix("page[") {
            if let Some(end) = rest.find("].") {
                if let Ok(idx) = rest[..end].parse::<i64>() {
                    if !indices.contains(&idx) {
                        indices.push(idx);
                    }
                }
            }
        }
    }
    indices.sort_unstable();

    indices
        .into_iter()
        .map(|index| FormPage {
            elements: extract_elements(doc, &format!("page[{index}]."), i18n),
        })
        .collect()
}

/// Collect element keys (`type.name`) under `prefix` in document order.
fn extract_elements(
    doc: &OdinDocument,
    prefix: &str,
    i18n: Option<&BTreeMap<String, String>>,
) -> Vec<FormElement> {
    let mut keys_ordered: Vec<String> = Vec::new();
    for path in doc.paths() {
        if let Some(rest) = path.strip_prefix(prefix) {
            let parts: Vec<&str> = rest.split('.').collect();
            if parts.len() >= 2 {
                let key = format!("{}.{}", parts[0], parts[1]);
                if !keys_ordered.contains(&key) {
                    keys_ordered.push(key);
                }
            }
        }
    }

    let mut elements: Vec<FormElement> = Vec::new();
    let mut id_counter = 0;
    for key in keys_ordered {
        let mut split = key.splitn(2, '.');
        let element_type = split.next().unwrap_or("");
        let element_name = split.next().unwrap_or("");
        let element_prefix = format!("{prefix}{key}.");
        if let Some(element) =
            build_element(doc, element_type, element_name, &element_prefix, id_counter, i18n)
        {
            elements.push(element);
            id_counter += 1;
        }
    }

    elements
}

#[allow(clippy::too_many_arguments)]
fn build_element(
    doc: &OdinDocument,
    element_type: &str,
    element_name: &str,
    prefix: &str,
    id_counter: i64,
    i18n: Option<&BTreeMap<String, String>>,
) -> Option<FormElement> {
    let id = format!("{element_type}_{element_name}_{id_counter}");
    let base = ElementBase {
        name: element_name.to_string(),
        id,
    };

    match element_type {
        "line" => Some(FormElement::Line(build_line(doc, base, prefix))),
        "rect" => Some(FormElement::Rect(build_rect(doc, base, prefix))),
        "circle" => Some(FormElement::Circle(build_circle(doc, base, prefix))),
        "ellipse" => Some(FormElement::Ellipse(build_ellipse(doc, base, prefix))),
        "polygon" => Some(FormElement::Polygon(build_polygon(doc, base, prefix))),
        "polyline" => Some(FormElement::Polyline(build_polyline(doc, base, prefix))),
        "path" => Some(FormElement::Path(build_path(doc, base, prefix))),
        "text" => Some(FormElement::Text(build_text(doc, base, prefix, i18n))),
        "img" => Some(FormElement::Image(build_image(doc, base, prefix, i18n))),
        "barcode" => Some(FormElement::Barcode(build_barcode(doc, base, prefix, i18n))),
        "field" => Some(build_field(doc, base, prefix, i18n)),
        "region" => Some(FormElement::Region(build_region(doc, base, prefix, i18n))),
        _ => None,
    }
}

// ── Geometric builders ──────────────────────────────────────────────────────

fn build_line(doc: &OdinDocument, base: ElementBase, prefix: &str) -> LineElement {
    LineElement {
        base,
        x1: get_number(doc, &format!("{prefix}x1")).unwrap_or(0.0),
        y1: get_number(doc, &format!("{prefix}y1")).unwrap_or(0.0),
        x2: get_number(doc, &format!("{prefix}x2")).unwrap_or(0.0),
        y2: get_number(doc, &format!("{prefix}y2")).unwrap_or(0.0),
        stroke: extract_stroke(doc, prefix),
    }
}

fn build_rect(doc: &OdinDocument, base: ElementBase, prefix: &str) -> RectElement {
    RectElement {
        base,
        x: get_number(doc, &format!("{prefix}x")).unwrap_or(0.0),
        y: get_number(doc, &format!("{prefix}y")).unwrap_or(0.0),
        w: get_number(doc, &format!("{prefix}w")).unwrap_or(0.0),
        h: get_number(doc, &format!("{prefix}h")).unwrap_or(0.0),
        rx: get_number(doc, &format!("{prefix}rx")),
        ry: get_number(doc, &format!("{prefix}ry")),
        stroke: extract_stroke(doc, prefix),
        fill: extract_fill(doc, prefix),
    }
}

fn build_circle(doc: &OdinDocument, base: ElementBase, prefix: &str) -> CircleElement {
    CircleElement {
        base,
        cx: get_number(doc, &format!("{prefix}cx")).unwrap_or(0.0),
        cy: get_number(doc, &format!("{prefix}cy")).unwrap_or(0.0),
        r: get_number(doc, &format!("{prefix}r")).unwrap_or(0.0),
        stroke: extract_stroke(doc, prefix),
        fill: extract_fill(doc, prefix),
    }
}

fn build_ellipse(doc: &OdinDocument, base: ElementBase, prefix: &str) -> EllipseElement {
    EllipseElement {
        base,
        cx: get_number(doc, &format!("{prefix}cx")).unwrap_or(0.0),
        cy: get_number(doc, &format!("{prefix}cy")).unwrap_or(0.0),
        rx: get_number(doc, &format!("{prefix}rx")).unwrap_or(0.0),
        ry: get_number(doc, &format!("{prefix}ry")).unwrap_or(0.0),
        stroke: extract_stroke(doc, prefix),
        fill: extract_fill(doc, prefix),
    }
}

fn build_polygon(doc: &OdinDocument, base: ElementBase, prefix: &str) -> PolygonElement {
    PolygonElement {
        base,
        points: get_string(doc, &format!("{prefix}points")).unwrap_or_default(),
        stroke: extract_stroke(doc, prefix),
        fill: extract_fill(doc, prefix),
    }
}

fn build_polyline(doc: &OdinDocument, base: ElementBase, prefix: &str) -> PolylineElement {
    PolylineElement {
        base,
        points: get_string(doc, &format!("{prefix}points")).unwrap_or_default(),
        stroke: extract_stroke(doc, prefix),
    }
}

fn build_path(doc: &OdinDocument, base: ElementBase, prefix: &str) -> PathElement {
    PathElement {
        base,
        d: get_string(doc, &format!("{prefix}d")).unwrap_or_default(),
        stroke: extract_stroke(doc, prefix),
        fill: extract_fill(doc, prefix),
    }
}

// ── Content builders ────────────────────────────────────────────────────────

fn build_text(
    doc: &OdinDocument,
    base: ElementBase,
    prefix: &str,
    i18n: Option<&BTreeMap<String, String>>,
) -> TextElement {
    TextElement {
        base,
        x: get_number(doc, &format!("{prefix}x")).unwrap_or(0.0),
        y: get_number(doc, &format!("{prefix}y")).unwrap_or(0.0),
        content: get_label(doc, &format!("{prefix}content"), i18n).unwrap_or_default(),
        rotate: get_number(doc, &format!("{prefix}rotate")),
        font: extract_font(doc, prefix),
        y_offset: get_number(doc, &format!("{prefix}y-offset")),
        x_offset: get_number(doc, &format!("{prefix}x-offset")),
    }
}

fn build_image(
    doc: &OdinDocument,
    base: ElementBase,
    prefix: &str,
    i18n: Option<&BTreeMap<String, String>>,
) -> ImageElement {
    ImageElement {
        base,
        x: get_number(doc, &format!("{prefix}x")).unwrap_or(0.0),
        y: get_number(doc, &format!("{prefix}y")).unwrap_or(0.0),
        w: get_number(doc, &format!("{prefix}w")).unwrap_or(0.0),
        h: get_number(doc, &format!("{prefix}h")).unwrap_or(0.0),
        src: get_binary_literal(doc, &format!("{prefix}src")).unwrap_or_default(),
        alt: get_label(doc, &format!("{prefix}alt"), i18n).unwrap_or_default(),
        background: get_boolean(doc, &format!("{prefix}background")),
    }
}

fn build_barcode(
    doc: &OdinDocument,
    base: ElementBase,
    prefix: &str,
    i18n: Option<&BTreeMap<String, String>>,
) -> BarcodeElement {
    let raw_type = get_string(doc, &format!("{prefix}type"))
        .or_else(|| get_string(doc, &format!("{prefix}barcode-type")))
        .unwrap_or_else(|| "code128".to_string());
    BarcodeElement {
        base,
        x: get_number(doc, &format!("{prefix}x")).unwrap_or(0.0),
        y: get_number(doc, &format!("{prefix}y")).unwrap_or(0.0),
        w: get_number(doc, &format!("{prefix}w")).unwrap_or(0.0),
        h: get_number(doc, &format!("{prefix}h")).unwrap_or(0.0),
        barcode_type: BarcodeType::parse(&raw_type),
        content: get_label(doc, &format!("{prefix}content"), i18n).unwrap_or_default(),
        alt: get_label(doc, &format!("{prefix}alt"), i18n).unwrap_or_default(),
    }
}

// ── Field builder ───────────────────────────────────────────────────────────

fn extract_base_field(
    doc: &OdinDocument,
    prefix: &str,
    i18n: Option<&BTreeMap<String, String>>,
) -> FieldBase {
    let bind_ref = get_reference(doc, &format!("{prefix}bind"));
    FieldBase {
        x: get_number(doc, &format!("{prefix}x")).unwrap_or(0.0),
        y: get_number(doc, &format!("{prefix}y")).unwrap_or(0.0),
        w: get_number(doc, &format!("{prefix}w")).unwrap_or(0.0),
        h: get_number(doc, &format!("{prefix}h")).unwrap_or(0.0),
        label: get_label(doc, &format!("{prefix}label"), i18n).unwrap_or_default(),
        aria_label: get_label(doc, &format!("{prefix}aria-label"), i18n),
        tabindex: get_integer(doc, &format!("{prefix}tabindex")),
        readonly: get_boolean(doc, &format!("{prefix}readonly")),
        validation: Validation {
            required: get_boolean(doc, &format!("{prefix}required")),
            pattern: get_string(doc, &format!("{prefix}pattern")),
            min_length: get_integer(doc, &format!("{prefix}minLength")),
            max_length: get_integer(doc, &format!("{prefix}maxLength")),
            min: get_number(doc, &format!("{prefix}min"))
                .map(format_number)
                .or_else(|| get_scalar_string(doc, &format!("{prefix}min"))),
            max: get_number(doc, &format!("{prefix}max"))
                .map(format_number)
                .or_else(|| get_scalar_string(doc, &format!("{prefix}max"))),
        },
        bind: bind_ref.map(|r| format!("@{r}")).unwrap_or_default(),
        y_offset: get_number(doc, &format!("{prefix}y-offset")),
        x_offset: get_number(doc, &format!("{prefix}x-offset")),
    }
}

fn build_field(
    doc: &OdinDocument,
    base: ElementBase,
    prefix: &str,
    i18n: Option<&BTreeMap<String, String>>,
) -> FormElement {
    let field_type = get_string(doc, &format!("{prefix}type")).unwrap_or_else(|| "text".to_string());
    let field = extract_base_field(doc, prefix, i18n);

    match field_type.as_str() {
        "checkbox" => FormElement::Checkbox(CheckboxElement {
            base,
            field,
            checked: get_boolean(doc, &format!("{prefix}checked")),
        }),
        "radio" => FormElement::Radio(RadioElement {
            base,
            field,
            group: get_string(doc, &format!("{prefix}group")).unwrap_or_default(),
            value: get_string(doc, &format!("{prefix}value")).unwrap_or_default(),
        }),
        "select" => FormElement::Select(SelectElement {
            base,
            field,
            options: extract_options(doc, prefix),
            selected: get_string(doc, &format!("{prefix}selected")),
            placeholder: get_string(doc, &format!("{prefix}placeholder")),
        }),
        "multiselect" => FormElement::Multiselect(MultiselectElement {
            base,
            field,
            options: extract_options(doc, prefix),
            selected: extract_field_array(doc, prefix, "selected"),
            min_select: get_integer(doc, &format!("{prefix}minSelect")),
            max_select: get_integer(doc, &format!("{prefix}maxSelect")),
        }),
        "date" => FormElement::Date(DateElement {
            base,
            field,
            value: get_scalar_string(doc, &format!("{prefix}value")),
        }),
        "signature" => FormElement::Signature(SignatureElement {
            base,
            field,
            value: get_binary_literal(doc, &format!("{prefix}value")),
            date_field: get_reference(doc, &format!("{prefix}date_field"))
                .map(|r| format!("@{r}"))
                .or_else(|| get_string(doc, &format!("{prefix}date_field"))),
        }),
        _ => FormElement::TextField(build_text_field(doc, base, field, prefix)),
    }
}

fn build_text_field(
    doc: &OdinDocument,
    base: ElementBase,
    field: FieldBase,
    prefix: &str,
) -> TextFieldElement {
    let input_type = get_string(doc, &format!("{prefix}inputType"))
        .and_then(|t| InputType::parse(&t));
    TextFieldElement {
        base,
        field,
        value: get_scalar_string(doc, &format!("{prefix}value")),
        input_type,
        mask: get_string(doc, &format!("{prefix}mask")),
        placeholder: get_string(doc, &format!("{prefix}placeholder")),
        multiline: get_boolean(doc, &format!("{prefix}multiline")),
        max_lines: get_integer(doc, &format!("{prefix}maxLines")),
    }
}

fn extract_options(doc: &OdinDocument, prefix: &str) -> Vec<String> {
    extract_field_array(doc, prefix, "options").unwrap_or_default()
}

/// Extract a field's tabular string array, tolerating the extra path segment a
/// relative tabular header introduces.
fn extract_field_array(doc: &OdinDocument, prefix: &str, name: &str) -> Option<Vec<String>> {
    let direct = collect_indexed(doc, &format!("{prefix}{name}"));
    if !direct.is_empty() {
        return Some(direct);
    }

    let suffix = format!(".{name}[");
    let mut indexed: Vec<(i64, String)> = Vec::new();
    for path in doc.paths() {
        if let Some(rest) = path.strip_prefix(prefix) {
            if let Some(pos) = rest.rfind(&suffix) {
                let after = &rest[pos + suffix.len()..];
                if let Some(end) = after.find(']') {
                    if after[end + 1..].is_empty() {
                        if let Ok(idx) = after[..end].parse::<i64>() {
                            if let Some(v) = get_string(doc, path) {
                                indexed.push((idx, v));
                            }
                        }
                    }
                }
            }
        }
    }
    if indexed.is_empty() {
        return None;
    }
    indexed.sort_by_key(|(idx, _)| *idx);
    Some(indexed.into_iter().map(|(_, v)| v).collect())
}

fn collect_indexed(doc: &OdinDocument, base: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    loop {
        let path = format!("{base}[{i}]");
        if !doc.has(&path) {
            break;
        }
        if let Some(v) = get_string(doc, &path) {
            out.push(v);
        }
        i += 1;
    }
    out
}

// ── Region builder ──────────────────────────────────────────────────────────

fn build_region(
    doc: &OdinDocument,
    base: ElementBase,
    prefix: &str,
    i18n: Option<&BTreeMap<String, String>>,
) -> RegionElement {
    let bind = get_reference(doc, &format!("{prefix}bind"));
    let overflow_ref = get_reference(doc, &format!("{prefix}overflow"));
    let overflow = overflow_ref
        .map(|r| format!("@{r}"))
        .or_else(|| get_string(doc, &format!("{prefix}overflow")));

    RegionElement {
        base,
        x: get_number(doc, &format!("{prefix}x")).unwrap_or(0.0),
        y: get_number(doc, &format!("{prefix}y")).unwrap_or(0.0),
        w: get_number(doc, &format!("{prefix}w")).unwrap_or(0.0),
        h: get_number(doc, &format!("{prefix}h")).unwrap_or(0.0),
        bind: bind.map(|r| format!("@{r}")),
        max: get_integer(doc, &format!("{prefix}max")),
        overflow,
        children: extract_region_children(doc, prefix, i18n),
    }
}

fn extract_region_children(
    doc: &OdinDocument,
    prefix: &str,
    i18n: Option<&BTreeMap<String, String>>,
) -> Vec<FormElement> {
    const OWN_PROPS: &[&str] = &["x", "y", "w", "h", "bind", "max", "overflow"];
    const CHILD_TYPES: &[&str] = &["text", "field", "img", "barcode"];

    let mut keys_ordered: Vec<String> = Vec::new();
    for path in doc.paths() {
        if let Some(rest) = path.strip_prefix(prefix) {
            let parts: Vec<&str> = rest.split('.').collect();
            if parts.len() < 2 {
                continue;
            }
            if OWN_PROPS.contains(&parts[0]) || !CHILD_TYPES.contains(&parts[0]) {
                continue;
            }
            let key = format!("{}.{}", parts[0], parts[1]);
            if !keys_ordered.contains(&key) {
                keys_ordered.push(key);
            }
        }
    }

    let mut children: Vec<FormElement> = Vec::new();
    let mut id_counter = 0;
    for key in keys_ordered {
        let mut split = key.splitn(2, '.');
        let child_type = split.next().unwrap_or("");
        let child_name = split.next().unwrap_or("");
        let child_prefix = format!("{prefix}{key}.");
        if let Some(child) =
            build_element(doc, child_type, child_name, &child_prefix, id_counter, i18n)
        {
            children.push(child);
            id_counter += 1;
        }
    }
    children
}

// ── Style mixin extractors ──────────────────────────────────────────────────

fn extract_stroke(doc: &OdinDocument, prefix: &str) -> Stroke {
    Stroke {
        stroke: get_string(doc, &format!("{prefix}stroke")),
        stroke_width: get_number(doc, &format!("{prefix}stroke-width")),
        stroke_opacity: get_number(doc, &format!("{prefix}stroke-opacity")),
        stroke_dasharray: get_string(doc, &format!("{prefix}stroke-dasharray")),
        stroke_linecap: get_string(doc, &format!("{prefix}stroke-linecap")),
        stroke_linejoin: get_string(doc, &format!("{prefix}stroke-linejoin")),
    }
}

fn extract_fill(doc: &OdinDocument, prefix: &str) -> Fill {
    Fill {
        fill: get_string(doc, &format!("{prefix}fill")),
        fill_opacity: get_number(doc, &format!("{prefix}fill-opacity")),
    }
}

fn extract_font(doc: &OdinDocument, prefix: &str) -> Font {
    Font {
        font_family: get_string(doc, &format!("{prefix}font-family")),
        font_size: get_number(doc, &format!("{prefix}font-size")),
        font_weight: get_string(doc, &format!("{prefix}font-weight")),
        font_style: get_string(doc, &format!("{prefix}font-style")),
        text_align: get_string(doc, &format!("{prefix}text-align")),
        color: get_string(doc, &format!("{prefix}color")),
    }
}

// ── Value accessors ─────────────────────────────────────────────────────────

/// Read a string from the `{$}` metadata map by its metadata key.
fn meta_string(doc: &OdinDocument, key: &str) -> Option<String> {
    match doc.metadata.get(&key.to_string()) {
        Some(OdinValue::String { value, .. }) => Some(value.clone()),
        _ => None,
    }
}

/// Read a number from the `{$}` metadata map by its metadata key.
fn meta_number(doc: &OdinDocument, key: &str) -> Option<f64> {
    match doc.metadata.get(&key.to_string()) {
        Some(OdinValue::Number { value, .. }) => Some(*value),
        Some(OdinValue::Integer { value, .. }) => Some(*value as f64),
        _ => None,
    }
}

fn get_string(doc: &OdinDocument, path: &str) -> Option<String> {
    match doc.get(path) {
        Some(OdinValue::String { value, .. }) => Some(value.clone()),
        _ => None,
    }
}

/// Resolve a string property that may be an `@$.i18n.*` reference.
fn get_label(
    doc: &OdinDocument,
    path: &str,
    i18n: Option<&BTreeMap<String, String>>,
) -> Option<String> {
    match doc.get(path) {
        Some(OdinValue::String { value, .. }) => Some(value.clone()),
        Some(OdinValue::Reference { path: ref_path, .. }) => {
            if let Some(key) = ref_path.strip_prefix("$.i18n.") {
                Some(
                    i18n.and_then(|m| m.get(key).cloned())
                        .unwrap_or_else(|| ref_path.clone()),
                )
            } else {
                Some(ref_path.clone())
            }
        }
        _ => None,
    }
}

/// Read a scalar value as a string, preserving raw form for dates/timestamps.
fn get_scalar_string(doc: &OdinDocument, path: &str) -> Option<String> {
    match doc.get(path) {
        Some(OdinValue::String { value, .. }) => Some(value.clone()),
        Some(OdinValue::Date { raw, .. } | OdinValue::Timestamp { raw, .. }) => Some(raw.clone()),
        _ => None,
    }
}

/// Reconstruct an ODIN binary literal (`^algorithm:base64`) for a binary value.
fn get_binary_literal(doc: &OdinDocument, path: &str) -> Option<String> {
    match doc.get(path) {
        Some(OdinValue::Binary { data, algorithm, .. }) => {
            let b64 = crate::utils::base64::encode(data);
            Some(match algorithm {
                Some(algo) => format!("^{algo}:{b64}"),
                None => format!("^{b64}"),
            })
        }
        Some(OdinValue::String { value, .. }) => Some(value.clone()),
        _ => None,
    }
}

fn get_number(doc: &OdinDocument, path: &str) -> Option<f64> {
    match doc.get(path) {
        Some(OdinValue::Number { value, .. }) => Some(*value),
        Some(OdinValue::Integer { value, .. }) => Some(*value as f64),
        _ => None,
    }
}

fn get_integer(doc: &OdinDocument, path: &str) -> Option<i64> {
    match doc.get(path) {
        Some(OdinValue::Integer { value, .. }) => Some(*value),
        Some(OdinValue::Number { value, .. }) => Some(*value as i64),
        _ => None,
    }
}

fn get_boolean(doc: &OdinDocument, path: &str) -> Option<bool> {
    match doc.get(path) {
        Some(OdinValue::Boolean { value, .. }) => Some(*value),
        _ => None,
    }
}

fn get_reference(doc: &OdinDocument, path: &str) -> Option<String> {
    match doc.get(path) {
        Some(OdinValue::Reference { path: ref_path, .. }) => Some(ref_path.clone()),
        _ => None,
    }
}

/// Render a number without a trailing `.0` for whole values.
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.is_finite() {
        format!("{}", value as i64)
    } else {
        let mut buf = ryu::Buffer::new();
        buf.format(value).to_string()
    }
}
