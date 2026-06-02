//! HTML/CSS renderer for parsed `OdinForm` documents.

use std::collections::BTreeMap;

use crate::types::document::OdinDocument;
use crate::types::values::OdinValue;

use super::accessibility::{
    field_aria_label, field_aria_required, field_group_html, field_label_html, generate_field_id,
    skip_link_html, tab_order_sort,
};
use super::css::{generate_form_css, generate_print_css};
use super::types::{
    BarcodeElement, CheckboxElement, CircleElement, DateElement, EllipseElement, FormElement,
    ImageElement, LineElement, MultiselectElement, OdinForm, PathElement, PolygonElement,
    PolylineElement, RadioElement, RectElement, RegionElement, SelectElement, SignatureElement,
    TextElement, TextFieldElement,
};
use super::units::to_pixels;

/// Render an `OdinForm` into a complete HTML string.
///
/// `data` optionally supplies an ODIN data document for binding field values
/// and computing region overflow.
#[must_use]
pub fn render_form(form: &OdinForm, data: Option<&OdinDocument>) -> String {
    let title = if form.metadata.title.is_empty() {
        "ODIN Form".to_string()
    } else {
        form.metadata.title.clone()
    };
    let unit = form
        .page_defaults
        .as_ref()
        .map_or("inch", |p| p.unit.as_str())
        .to_string();

    let plan = build_render_plan(form, data);
    let total_pages = plan.len() as i64;
    let page_w = to_pixels(form.page_defaults.as_ref().map_or(8.5, |p| p.width), &unit);
    let page_h = to_pixels(form.page_defaults.as_ref().map_or(11.0, |p| p.height), &unit);

    let mut out = String::new();
    out.push_str(&format!(
        "<form role=\"form\" aria-label=\"{}\" class=\"odin-form\">",
        escape_attr(&title)
    ));
    out.push_str(&skip_link_html(&title));
    out.push_str(&format!(
        "<style>{}\n{}</style>",
        generate_form_css(),
        generate_print_css()
    ));

    for (i, planned) in plan.iter().enumerate() {
        let ctx = RenderContext {
            page_number: i as i64 + 1,
            total_pages,
            unit: &unit,
            data,
            page_width_px: page_w,
            page_height_px: page_h,
        };
        out.push_str(&render_planned_page(planned, &ctx));
    }

    out.push_str("</form>");
    out
}

struct RenderContext<'a> {
    page_number: i64,
    total_pages: i64,
    unit: &'a str,
    data: Option<&'a OdinDocument>,
    page_width_px: f64,
    page_height_px: f64,
}

/// A slice of bound items a region renders on an overflow page.
#[derive(Clone)]
struct ItemSlice {
    start: i64,
    count: i64,
    bind: String,
}

struct PlannedPage<'a> {
    elements: &'a [FormElement],
    item_slices: BTreeMap<String, ItemSlice>,
}

/// Build the ordered list of output pages, expanding region overflow.
fn build_render_plan<'a>(form: &'a OdinForm, data: Option<&OdinDocument>) -> Vec<PlannedPage<'a>> {
    let mut plan: Vec<PlannedPage<'a>> = Vec::new();

    for page in &form.pages {
        plan.push(PlannedPage {
            elements: &page.elements,
            item_slices: BTreeMap::new(),
        });

        let Some(data) = data else { continue };

        for el in &page.elements {
            let FormElement::Region(region) = el else {
                continue;
            };
            let (Some(bind), Some(max), Some(overflow)) =
                (&region.bind, region.max, &region.overflow)
            else {
                continue;
            };
            if max < 1 {
                continue;
            }
            let count = bound_array_length(bind, Some(data));
            if count <= max {
                continue;
            }

            let mut consumed = max;
            let mut template_name = overflow.strip_prefix('@').map(ToString::to_string);
            let mut guard = 0;
            while consumed < count && guard < 10000 {
                guard += 1;
                let tpl = template_name
                    .as_ref()
                    .and_then(|n| form.templates.as_ref().and_then(|t| t.get(n)));
                let tpl_region = tpl.and_then(|t| {
                    t.elements.iter().find_map(|e| match e {
                        FormElement::Region(r) if r.base.name == region.base.name => Some(r),
                        _ => None,
                    })
                });
                let candidate_max = tpl_region.and_then(|r| r.max).unwrap_or(max);
                let page_max = if candidate_max >= 1 { candidate_max } else { max };

                let mut slices: BTreeMap<String, ItemSlice> = BTreeMap::new();
                slices.insert(
                    region.base.name.clone(),
                    ItemSlice {
                        start: consumed,
                        count: page_max.min(count - consumed),
                        bind: bind.clone(),
                    },
                );
                let elements: &'a [FormElement] =
                    tpl.map_or(page.elements.as_slice(), |t| t.elements.as_slice());
                plan.push(PlannedPage {
                    elements,
                    item_slices: slices,
                });
                consumed += page_max;

                if let Some(ov) = tpl_region.and_then(|r| r.overflow.as_ref()) {
                    if let Some(next) = ov.strip_prefix('@') {
                        template_name = Some(next.to_string());
                    }
                }
            }
        }
    }

    plan
}

// ── Page rendering ──────────────────────────────────────────────────────────

fn render_planned_page(page: &PlannedPage, ctx: &RenderContext) -> String {
    let page_index = ctx.page_number - 1;
    let mut out = String::new();
    out.push_str(&format!(
        "<div class=\"odin-form-page\" id=\"odin-form-content\" data-page=\"{}\" style=\"width:{}px;height:{}px;\">",
        ctx.page_number, ctx.page_width_px, ctx.page_height_px
    ));

    // Background images first, then non-field elements, then fields.
    for el in page.elements {
        if let FormElement::Image(img) = el {
            if img.background == Some(true) {
                out.push_str(&render_element(el, page_index, ctx, page));
            }
        }
    }
    for el in page.elements {
        if let FormElement::Image(img) = el {
            if img.background == Some(true) {
                continue;
            }
        }
        if !el.is_field() {
            out.push_str(&render_element(el, page_index, ctx, page));
        }
    }
    for el in tab_order_sort(page.elements) {
        out.push_str(&render_element(el, page_index, ctx, page));
    }

    out.push_str("</div>");
    out
}

fn render_element(
    el: &FormElement,
    page_index: i64,
    ctx: &RenderContext,
    page: &PlannedPage,
) -> String {
    let unit = ctx.unit;
    match el {
        FormElement::Line(e) => render_line(e, unit),
        FormElement::Rect(e) => render_rect(e, unit),
        FormElement::Circle(e) => render_circle(e, unit),
        FormElement::Ellipse(e) => render_ellipse(e, unit),
        FormElement::Polygon(e) => render_polygon(e, unit),
        FormElement::Polyline(e) => render_polyline(e, unit),
        FormElement::Path(e) => render_path(e, unit),
        FormElement::Text(e) => render_text(e, ctx),
        FormElement::Image(e) => render_image(e, ctx),
        FormElement::Barcode(e) => render_barcode(e, ctx),
        FormElement::TextField(e) => render_text_field(e, page_index, ctx),
        FormElement::Checkbox(e) => render_checkbox(e, page_index, ctx),
        FormElement::Radio(e) => render_radio(e, page_index, ctx),
        FormElement::Select(e) => render_select(e, page_index, ctx),
        FormElement::Multiselect(e) => render_multiselect(e, page_index, ctx),
        FormElement::Date(e) => render_date(e, page_index, ctx),
        FormElement::Signature(e) => render_signature(e, page_index, ctx),
        FormElement::Region(e) => render_region(e, ctx, page),
    }
}

// ── Interpolation ───────────────────────────────────────────────────────────

/// Resolve `{@odin.page}` / `{@odin.total_pages}` tokens in a string.
fn interpolate(text: &str, ctx: &RenderContext) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(close) = text[i..].find('}') {
                let token = &text[i + 1..i + close];
                if let Some(name) = token.strip_prefix("@odin.") {
                    match name {
                        "page" => {
                            out.push_str(&ctx.page_number.to_string());
                            i += close + 1;
                            continue;
                        }
                        "total_pages" => {
                            out.push_str(&ctx.total_pages.to_string());
                            i += close + 1;
                            continue;
                        }
                        _ => {}
                    }
                }
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

// ── Geometric elements ──────────────────────────────────────────────────────

fn render_line(el: &LineElement, unit: &str) -> String {
    let x1 = to_pixels(el.x1, unit);
    let y1 = to_pixels(el.y1, unit);
    let x2 = to_pixels(el.x2, unit);
    let y2 = to_pixels(el.y2, unit);
    let stroke = el.stroke.stroke.as_deref().unwrap_or("#000000");
    let sw = el.stroke.stroke_width.map_or(1.0, |w| to_pixels(w, unit));
    format!(
        "<svg class=\"odin-form-element\" style=\"position:absolute;left:0;top:0;width:100%;height:100%;overflow:visible;\"><line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"{}\"/></svg>",
        fmt(x1), fmt(y1), fmt(x2), fmt(y2), stroke, fmt(sw)
    )
}

fn render_rect(el: &RectElement, unit: &str) -> String {
    let x = to_pixels(el.x, unit);
    let y = to_pixels(el.y, unit);
    let w = to_pixels(el.w, unit);
    let h = to_pixels(el.h, unit);
    let border = el.stroke.stroke.as_ref().map_or(String::new(), |s| {
        let sw = el.stroke.stroke_width.map_or(1.0, |w| to_pixels(w, unit));
        format!("border:{}px solid {s};", fmt(sw))
    });
    let bg = match &el.fill.fill {
        Some(f) if f != "none" => format!("background:{f};"),
        _ => String::new(),
    };
    let rx = el.rx.map_or(0.0, |v| to_pixels(v, unit));
    let ry = el.ry.map_or(0.0, |v| to_pixels(v, unit));
    let radius = if rx != 0.0 || ry != 0.0 {
        format!("border-radius:{}px {}px;", fmt(rx), fmt(ry))
    } else {
        String::new()
    };
    format!(
        "<div class=\"odin-form-element\" style=\"position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;{border}{bg}{radius}\"></div>",
        fmt(x), fmt(y), fmt(w), fmt(h)
    )
}

fn render_circle(el: &CircleElement, unit: &str) -> String {
    let cx = to_pixels(el.cx, unit);
    let cy = to_pixels(el.cy, unit);
    let r = to_pixels(el.r, unit);
    let stroke = el.stroke.stroke.as_deref().unwrap_or("#000000");
    let sw = el.stroke.stroke_width.map_or(1.0, |w| to_pixels(w, unit));
    let fill = el.fill.fill.as_deref().unwrap_or("none");
    format!(
        "<svg class=\"odin-form-element\" style=\"position:absolute;left:0;top:0;width:100%;height:100%;overflow:visible;\"><circle cx=\"{}\" cy=\"{}\" r=\"{}\" stroke=\"{}\" stroke-width=\"{}\" fill=\"{}\"/></svg>",
        fmt(cx), fmt(cy), fmt(r), stroke, fmt(sw), fill
    )
}

fn render_ellipse(el: &EllipseElement, unit: &str) -> String {
    let cx = to_pixels(el.cx, unit);
    let cy = to_pixels(el.cy, unit);
    let rx = to_pixels(el.rx, unit);
    let ry = to_pixels(el.ry, unit);
    let stroke = el.stroke.stroke.as_deref().unwrap_or("#000000");
    let sw = el.stroke.stroke_width.map_or(1.0, |w| to_pixels(w, unit));
    let fill = el.fill.fill.as_deref().unwrap_or("none");
    format!(
        "<svg class=\"odin-form-element\" style=\"position:absolute;left:0;top:0;width:100%;height:100%;overflow:visible;\"><ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" stroke=\"{}\" stroke-width=\"{}\" fill=\"{}\"/></svg>",
        fmt(cx), fmt(cy), fmt(rx), fmt(ry), stroke, fmt(sw), fill
    )
}

fn render_polygon(el: &PolygonElement, unit: &str) -> String {
    let points = convert_points(&el.points, unit);
    let stroke = el.stroke.stroke.as_deref().unwrap_or("#000000");
    let sw = el.stroke.stroke_width.map_or(1.0, |w| to_pixels(w, unit));
    let fill = el.fill.fill.as_deref().unwrap_or("none");
    format!(
        "<svg class=\"odin-form-element\" style=\"position:absolute;left:0;top:0;width:100%;height:100%;overflow:visible;\"><polygon points=\"{points}\" stroke=\"{stroke}\" stroke-width=\"{}\" fill=\"{fill}\"/></svg>",
        fmt(sw)
    )
}

fn render_polyline(el: &PolylineElement, unit: &str) -> String {
    let points = convert_points(&el.points, unit);
    let stroke = el.stroke.stroke.as_deref().unwrap_or("#000000");
    let sw = el.stroke.stroke_width.map_or(1.0, |w| to_pixels(w, unit));
    format!(
        "<svg class=\"odin-form-element\" style=\"position:absolute;left:0;top:0;width:100%;height:100%;overflow:visible;\"><polyline points=\"{points}\" stroke=\"{stroke}\" stroke-width=\"{}\" fill=\"none\"/></svg>",
        fmt(sw)
    )
}

fn render_path(el: &PathElement, unit: &str) -> String {
    let stroke = el.stroke.stroke.as_deref().unwrap_or("#000000");
    let sw = el.stroke.stroke_width.map_or(1.0, |w| to_pixels(w, unit));
    let fill = el.fill.fill.as_deref().unwrap_or("none");
    format!(
        "<svg class=\"odin-form-element\" style=\"position:absolute;left:0;top:0;width:100%;height:100%;overflow:visible;\"><path d=\"{}\" stroke=\"{stroke}\" stroke-width=\"{}\" fill=\"{fill}\"/></svg>",
        el.d, fmt(sw)
    )
}

// ── Content elements ────────────────────────────────────────────────────────

fn render_text_at(el: &TextElement, x: f64, y: f64, ctx: &RenderContext) -> String {
    let unit = ctx.unit;
    let px = to_pixels(x, unit);
    let py = to_pixels(y, unit);
    let font_size = el.font.font_size.map_or_else(
        || to_pixels(12.0, "pt"),
        |s| to_pixels(s, "pt"),
    );
    let font_weight = el.font.font_weight.as_deref().unwrap_or("normal");
    let color = el.font.color.as_deref().unwrap_or("#000000");
    let font_family = el
        .font
        .font_family
        .as_ref()
        .map_or(String::new(), |f| format!("font-family:{f};"));
    let font_style = if el.font.font_style.as_deref() == Some("italic") {
        "font-style:italic;"
    } else {
        ""
    };
    let text_align = el
        .font
        .text_align
        .as_ref()
        .map_or(String::new(), |a| format!("text-align:{a};"));
    let content = interpolate(&el.content, ctx);
    format!(
        "<span class=\"odin-form-element\" style=\"position:absolute;left:{}px;top:{}px;font-size:{}px;font-weight:{font_weight};color:{color};{font_family}{font_style}{text_align}\">{}</span>",
        fmt(px), fmt(py), fmt(font_size), escape_html(&content)
    )
}

fn render_text(el: &TextElement, ctx: &RenderContext) -> String {
    render_text_at(el, el.x, el.y, ctx)
}

fn render_image(el: &ImageElement, ctx: &RenderContext) -> String {
    let unit = ctx.unit;
    let x = to_pixels(el.x, unit);
    let y = to_pixels(el.y, unit);
    let w = to_pixels(el.w, unit);
    let h = to_pixels(el.h, unit);
    let src = image_src_to_data_uri(&el.src);
    let alt = interpolate(&el.alt, ctx);
    let z_index = if el.background == Some(true) {
        "z-index:0;"
    } else {
        ""
    };
    format!(
        "<img class=\"odin-form-element\" src=\"{}\" alt=\"{}\" style=\"position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;{z_index}\">",
        escape_attr(&src), escape_attr(&alt), fmt(x), fmt(y), fmt(w), fmt(h)
    )
}

fn render_barcode(el: &BarcodeElement, ctx: &RenderContext) -> String {
    let unit = ctx.unit;
    let x = to_pixels(el.x, unit);
    let y = to_pixels(el.y, unit);
    let w = to_pixels(el.w, unit);
    let h = to_pixels(el.h, unit);
    let alt = interpolate(&el.alt, ctx);
    let content = interpolate(&el.content, ctx);
    format!(
        "<div class=\"odin-form-element odin-form-barcode\" role=\"img\" aria-label=\"{}\" data-barcode-type=\"{}\" data-content=\"{}\" style=\"position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;\"></div>",
        escape_attr(&alt), escape_attr(el.barcode_type.as_str()), escape_attr(&content),
        fmt(x), fmt(y), fmt(w), fmt(h)
    )
}

/// Convert an ODIN binary literal (`^png:base64`) to a data URI.
fn image_src_to_data_uri(src: &str) -> String {
    let Some(rest) = src.strip_prefix('^') else {
        return src.to_string();
    };
    match rest.find(':') {
        None => format!("data:image/png;base64,{rest}"),
        Some(colon) => {
            let format = &rest[..colon];
            let b64 = &rest[colon + 1..];
            format!("data:image/{format};base64,{b64}")
        }
    }
}

// ── Field elements ──────────────────────────────────────────────────────────

fn aria_required_attr(required: bool) -> &'static str {
    if required {
        " aria-required=\"true\""
    } else {
        ""
    }
}

fn render_text_field(el: &TextFieldElement, page_index: i64, ctx: &RenderContext) -> String {
    let unit = ctx.unit;
    let x = to_pixels(el.field.x, unit);
    let y = to_pixels(el.field.y, unit);
    let w = to_pixels(el.field.w, unit);
    let h = to_pixels(el.field.h, unit);
    let id = generate_field_id(&el.base.name, page_index);
    let value = el
        .value
        .clone()
        .or_else(|| lookup_bound_value(&el.field.bind, ctx.data));
    let value_attr = value.map_or(String::new(), |v| format!(" value=\"{}\"", escape_attr(&v)));
    let required = field_aria_required(&el.field);
    let required_attr = if required { " required" } else { "" };
    let readonly_attr = if el.field.readonly == Some(true) {
        " readonly"
    } else {
        ""
    };
    let placeholder_attr = el
        .placeholder
        .as_ref()
        .map_or(String::new(), |p| format!(" placeholder=\"{}\"", escape_attr(p)));
    let input_type = el.input_type.map_or("text", super::types::InputType::as_str);
    let aria = interpolate(field_aria_label(&el.field), ctx);

    format!(
        "<div class=\"odin-form-element\" style=\"position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;\">{}<input type=\"{}\" class=\"odin-form-input\" id=\"{id}\" aria-label=\"{}\"{}{value_attr}{required_attr}{readonly_attr}{placeholder_attr}></div>",
        fmt(x), fmt(y), fmt(w), fmt(h),
        field_label_html(&interpolate(&el.field.label, ctx), &id),
        escape_attr(input_type), escape_attr(&aria), aria_required_attr(required)
    )
}

fn render_checkbox(el: &CheckboxElement, page_index: i64, ctx: &RenderContext) -> String {
    let unit = ctx.unit;
    let x = to_pixels(el.field.x, unit);
    let y = to_pixels(el.field.y, unit);
    let w = to_pixels(el.field.w, unit);
    let h = to_pixels(el.field.h, unit);
    let id = generate_field_id(&el.base.name, page_index);
    let bound = lookup_bound_value(&el.field.bind, ctx.data);
    let is_checked = el.checked.unwrap_or(bound.as_deref() == Some("true"));
    let checked = if is_checked { " checked" } else { "" };
    let required = field_aria_required(&el.field);
    let aria = interpolate(field_aria_label(&el.field), ctx);

    format!(
        "<div class=\"odin-form-element\" style=\"position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;\">{}<input type=\"checkbox\" class=\"odin-form-checkbox\" id=\"{id}\" aria-label=\"{}\"{}{checked}></div>",
        fmt(x), fmt(y), fmt(w), fmt(h),
        field_label_html(&interpolate(&el.field.label, ctx), &id),
        escape_attr(&aria), aria_required_attr(required)
    )
}

fn render_radio(el: &RadioElement, page_index: i64, ctx: &RenderContext) -> String {
    let unit = ctx.unit;
    let x = to_pixels(el.field.x, unit);
    let y = to_pixels(el.field.y, unit);
    let w = to_pixels(el.field.w, unit);
    let h = to_pixels(el.field.h, unit);
    let id = generate_field_id(&el.base.name, page_index);
    let value = lookup_bound_value(&el.field.bind, ctx.data);
    let checked = if value.as_deref() == Some(el.value.as_str()) {
        " checked"
    } else {
        ""
    };
    let required = field_aria_required(&el.field);
    let aria = interpolate(field_aria_label(&el.field), ctx);

    let radio_html = format!(
        "<input type=\"radio\" class=\"odin-form-radio\" id=\"{id}\" name=\"{}\" value=\"{}\" aria-label=\"{}\"{}{checked}><label for=\"{id}\">{}</label>",
        escape_attr(&el.group), escape_attr(&el.value), escape_attr(&aria),
        aria_required_attr(required), escape_html(&interpolate(&el.field.label, ctx))
    );

    format!(
        "<div class=\"odin-form-element\" style=\"position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;\">{}</div>",
        fmt(x), fmt(y), fmt(w), fmt(h),
        field_group_html(&interpolate(&el.field.label, ctx), &radio_html)
    )
}

fn render_select(el: &SelectElement, page_index: i64, ctx: &RenderContext) -> String {
    let unit = ctx.unit;
    let x = to_pixels(el.field.x, unit);
    let y = to_pixels(el.field.y, unit);
    let w = to_pixels(el.field.w, unit);
    let h = to_pixels(el.field.h, unit);
    let id = generate_field_id(&el.base.name, page_index);
    let value = el
        .selected
        .clone()
        .or_else(|| lookup_bound_value(&el.field.bind, ctx.data));
    let required = field_aria_required(&el.field);
    let aria = interpolate(field_aria_label(&el.field), ctx);

    let mut options_html = String::new();
    if let Some(placeholder) = &el.placeholder {
        options_html.push_str(&format!(
            "<option value=\"\">{}</option>",
            escape_html(placeholder)
        ));
    }
    for opt in &el.options {
        let selected = if value.as_deref() == Some(opt.as_str()) {
            " selected"
        } else {
            ""
        };
        options_html.push_str(&format!(
            "<option value=\"{}\"{selected}>{}</option>",
            escape_attr(opt),
            escape_html(opt)
        ));
    }

    format!(
        "<div class=\"odin-form-element\" style=\"position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;\">{}<select class=\"odin-form-select\" id=\"{id}\" aria-label=\"{}\"{}>{options_html}</select></div>",
        fmt(x), fmt(y), fmt(w), fmt(h),
        field_label_html(&interpolate(&el.field.label, ctx), &id),
        escape_attr(&aria), aria_required_attr(required)
    )
}

fn render_multiselect(el: &MultiselectElement, page_index: i64, ctx: &RenderContext) -> String {
    let unit = ctx.unit;
    let x = to_pixels(el.field.x, unit);
    let y = to_pixels(el.field.y, unit);
    let w = to_pixels(el.field.w, unit);
    let h = to_pixels(el.field.h, unit);
    let id = generate_field_id(&el.base.name, page_index);
    let required = field_aria_required(&el.field);
    let aria = interpolate(field_aria_label(&el.field), ctx);

    let selected_values: Vec<String> = if let Some(sel) = &el.selected {
        sel.clone()
    } else {
        lookup_bound_value(&el.field.bind, ctx.data).map_or_else(Vec::new, |v| {
            v.split(',').map(|s| s.trim().to_string()).collect()
        })
    };

    let mut options_html = String::new();
    for opt in &el.options {
        let selected = if selected_values.iter().any(|s| s == opt) {
            " selected"
        } else {
            ""
        };
        options_html.push_str(&format!(
            "<option value=\"{}\"{selected}>{}</option>",
            escape_attr(opt),
            escape_html(opt)
        ));
    }

    format!(
        "<div class=\"odin-form-element\" style=\"position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;\">{}<select multiple class=\"odin-form-select\" id=\"{id}\" aria-label=\"{}\"{}>{options_html}</select></div>",
        fmt(x), fmt(y), fmt(w), fmt(h),
        field_label_html(&interpolate(&el.field.label, ctx), &id),
        escape_attr(&aria), aria_required_attr(required)
    )
}

fn render_date(el: &DateElement, page_index: i64, ctx: &RenderContext) -> String {
    let unit = ctx.unit;
    let x = to_pixels(el.field.x, unit);
    let y = to_pixels(el.field.y, unit);
    let w = to_pixels(el.field.w, unit);
    let h = to_pixels(el.field.h, unit);
    let id = generate_field_id(&el.base.name, page_index);
    let value = el
        .value
        .clone()
        .or_else(|| lookup_bound_value(&el.field.bind, ctx.data));
    let value_attr = value.map_or(String::new(), |v| format!(" value=\"{}\"", escape_attr(&v)));
    let required = field_aria_required(&el.field);
    let required_attr = if required { " required" } else { "" };
    let aria = interpolate(field_aria_label(&el.field), ctx);

    format!(
        "<div class=\"odin-form-element\" style=\"position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;\">{}<input type=\"date\" class=\"odin-form-input\" id=\"{id}\" aria-label=\"{}\"{}{value_attr}{required_attr}></div>",
        fmt(x), fmt(y), fmt(w), fmt(h),
        field_label_html(&interpolate(&el.field.label, ctx), &id),
        escape_attr(&aria), aria_required_attr(required)
    )
}

fn render_signature(el: &SignatureElement, page_index: i64, ctx: &RenderContext) -> String {
    let unit = ctx.unit;
    let x = to_pixels(el.field.x, unit);
    let y = to_pixels(el.field.y, unit);
    let w = to_pixels(el.field.w, unit);
    let h = to_pixels(el.field.h, unit);
    let id = generate_field_id(&el.base.name, page_index);
    let required = field_aria_required(&el.field);
    let aria = interpolate(field_aria_label(&el.field), ctx);

    format!(
        "<div class=\"odin-form-element\" style=\"position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;\">{}<div class=\"odin-form-signature\" id=\"{id}\" aria-label=\"{}\"{} role=\"img\" tabindex=\"0\" style=\"width:100%;height:100%;\"></div></div>",
        fmt(x), fmt(y), fmt(w), fmt(h),
        field_label_html(&interpolate(&el.field.label, ctx), &id),
        escape_attr(&aria), aria_required_attr(required)
    )
}

// ── Region rendering ────────────────────────────────────────────────────────

fn render_region(el: &RegionElement, ctx: &RenderContext, page: &PlannedPage) -> String {
    let unit = ctx.unit;
    let region_x = to_pixels(el.x, unit);
    let region_y = to_pixels(el.y, unit);
    let region_w = to_pixels(el.w, unit);
    let region_h = to_pixels(el.h, unit);

    let slice = page.item_slices.get(&el.base.name);
    let bind = el.bind.clone().or_else(|| slice.map(|s| s.bind.clone()));
    let total = bind.as_ref().map_or(0, |b| bound_array_length(b, ctx.data));

    let (start, count) = if let Some(s) = slice {
        (s.start, s.count)
    } else if total > 0 {
        let c = el.max.map_or(total, |m| m.min(total));
        (0, c)
    } else {
        (0, 1)
    };

    let mut out = String::new();
    out.push_str(&format!(
        "<div class=\"odin-form-element odin-form-region\" data-region=\"{}\" style=\"position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;\">",
        escape_attr(&el.base.name), fmt(region_x), fmt(region_y), fmt(region_w), fmt(region_h)
    ));

    for i in 0..count {
        let item_index = start + i;
        let item_bind = bind.as_ref().map(|b| format!("{b}[{item_index}]"));
        for child in &el.children {
            out.push_str(&render_region_child(child, i, item_bind.as_deref(), ctx));
        }
    }

    out.push_str("</div>");
    out
}

fn render_region_child(
    child: &FormElement,
    i: i64,
    item_bind: Option<&str>,
    ctx: &RenderContext,
) -> String {
    if let FormElement::Text(text) = child {
        let y_offset = text.y_offset.unwrap_or(0.0);
        let x_offset = text.x_offset.unwrap_or(0.0);
        let dx = text.x + x_offset * i as f64;
        let dy = text.y + y_offset * i as f64;
        return render_text_at(text, dx, dy, ctx);
    }

    if let Some(field) = child.as_field() {
        let y_offset = field.y_offset.unwrap_or(0.0);
        let x_offset = field.x_offset.unwrap_or(0.0);
        let dx = field.x + x_offset * i as f64;
        let dy = field.y + y_offset * i as f64;
        let resolved_bind = resolve_relative_bind(&field.bind, item_bind).unwrap_or_default();
        let rebased = rebase_field(child, dx, dy, i, &resolved_bind);
        let child_page_index = -1 - i;
        return render_element(&rebased, child_page_index, ctx, &PlannedPage {
            elements: &[],
            item_slices: BTreeMap::new(),
        });
    }

    String::new()
}

/// Clone a field element with rebased coordinates, a unique name, and resolved bind.
fn rebase_field(child: &FormElement, dx: f64, dy: f64, i: i64, bind: &str) -> FormElement {
    let mut cloned = child.clone();
    let suffix = format!("_{i}");
    match &mut cloned {
        FormElement::TextField(e) => apply_rebase(&mut e.base.name, &mut e.field, dx, dy, bind, &suffix),
        FormElement::Checkbox(e) => apply_rebase(&mut e.base.name, &mut e.field, dx, dy, bind, &suffix),
        FormElement::Radio(e) => apply_rebase(&mut e.base.name, &mut e.field, dx, dy, bind, &suffix),
        FormElement::Select(e) => apply_rebase(&mut e.base.name, &mut e.field, dx, dy, bind, &suffix),
        FormElement::Multiselect(e) => apply_rebase(&mut e.base.name, &mut e.field, dx, dy, bind, &suffix),
        FormElement::Date(e) => apply_rebase(&mut e.base.name, &mut e.field, dx, dy, bind, &suffix),
        FormElement::Signature(e) => apply_rebase(&mut e.base.name, &mut e.field, dx, dy, bind, &suffix),
        _ => {}
    }
    cloned
}

fn apply_rebase(
    name: &mut String,
    field: &mut super::types::FieldBase,
    dx: f64,
    dy: f64,
    bind: &str,
    suffix: &str,
) {
    field.x = dx;
    field.y = dy;
    field.bind = bind.to_string();
    name.push_str(suffix);
}

/// Resolve a region child's `@.field` relative bind against the current item path.
fn resolve_relative_bind(bind: &str, item_bind: Option<&str>) -> Option<String> {
    if bind.is_empty() {
        return None;
    }
    if let Some(rel) = bind.strip_prefix("@.") {
        return item_bind.map(|ib| format!("{ib}.{rel}"));
    }
    Some(bind.to_string())
}

/// Number of items in a bound array path.
fn bound_array_length(bind: &str, data: Option<&OdinDocument>) -> i64 {
    let Some(data) = data else { return 0 };
    let path = bind.strip_prefix('@').unwrap_or(bind);
    let prefix = format!("{path}[");
    let mut max: i64 = -1;
    for p in data.paths() {
        if let Some(rest) = p.strip_prefix(&prefix) {
            if let Some(end) = rest.find(']') {
                let after = &rest[end + 1..];
                if after.is_empty() || after.starts_with('.') {
                    if let Ok(idx) = rest[..end].parse::<i64>() {
                        if idx > max {
                            max = idx;
                        }
                    }
                }
            }
        }
    }
    max + 1
}

// ── Data binding ────────────────────────────────────────────────────────────

fn lookup_bound_value(bind: &str, data: Option<&OdinDocument>) -> Option<String> {
    let data = data?;
    if bind.is_empty() {
        return None;
    }
    let path = bind.strip_prefix('@').unwrap_or(bind);
    if path.is_empty() {
        return None;
    }
    match data.get(path)? {
        OdinValue::String { value, .. } => Some(value.clone()),
        OdinValue::Number { value, .. } => Some(format_number(*value)),
        OdinValue::Integer { value, .. } => Some(value.to_string()),
        OdinValue::Boolean { value, .. } => Some(value.to_string()),
        _ => None,
    }
}

// ── Utilities ───────────────────────────────────────────────────────────────

/// Format a pixel value, dropping a trailing `.0` for whole numbers.
fn fmt(value: f64) -> String {
    if value.fract() == 0.0 && value.is_finite() {
        format!("{}", value as i64)
    } else {
        let mut buf = ryu::Buffer::new();
        buf.format(value).to_string()
    }
}

fn format_number(value: f64) -> String {
    fmt(value)
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Convert an SVG `points` string from page units to pixels.
fn convert_points(points: &str, unit: &str) -> String {
    points
        .split_whitespace()
        .map(|pair| match pair.split_once(',') {
            Some((x, y)) => {
                let px = x.parse::<f64>().map_or_else(|_| x.to_string(), |v| fmt(to_pixels(v, unit)));
                let py = y.parse::<f64>().map_or_else(|_| y.to_string(), |v| fmt(to_pixels(v, unit)));
                format!("{px},{py}")
            }
            None => pair.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}
