//! Accessibility helpers for the HTML renderer.

use super::types::{FieldBase, FormElement};

/// Stable HTML element ID for a form field.
#[must_use]
pub fn generate_field_id(element_name: &str, page_index: i64) -> String {
    format!("odin-field-{page_index}-{element_name}")
}

/// HTML `<label>` associated with the given input ID.
#[must_use]
pub fn field_label_html(label: &str, input_id: &str) -> String {
    format!("<label for=\"{input_id}\" class=\"odin-form-label\">{label}</label>")
}

/// ARIA label for a field: explicit override or the visible label.
#[must_use]
pub fn field_aria_label(field: &FieldBase) -> &str {
    field.aria_label.as_deref().unwrap_or(&field.label)
}

/// Whether the field exposes `aria-required="true"`.
#[must_use]
pub fn field_aria_required(field: &FieldBase) -> bool {
    field.validation.required.unwrap_or(false)
}

/// Wrap grouped controls in a `<fieldset>` with a `<legend>`.
#[must_use]
pub fn field_group_html(legend: &str, content: &str) -> String {
    format!(
        "<fieldset class=\"odin-form-fieldset\"><legend class=\"odin-form-legend\">{legend}</legend>{content}</fieldset>"
    )
}

/// Skip-navigation link targeting the form content container.
#[must_use]
pub fn skip_link_html(form_title: &str) -> String {
    format!(
        "<a class=\"odin-form-sr-only odin-form-skip\" href=\"#odin-form-content\">Skip to {form_title}</a>"
    )
}

/// Visually-hidden span announced by screen readers.
#[must_use]
pub fn sr_only_html(text: &str) -> String {
    format!("<span class=\"odin-form-sr-only\">{text}</span>")
}

/// Field elements sorted by reading order: top-to-bottom, then left-to-right.
#[must_use]
pub fn tab_order_sort(elements: &[FormElement]) -> Vec<&FormElement> {
    let mut fields: Vec<&FormElement> = elements.iter().filter(|e| e.is_field()).collect();
    fields.sort_by(|a, b| {
        let fa = a.as_field().unwrap();
        let fb = b.as_field().unwrap();
        fa.y.partial_cmp(&fb.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(fa.x.partial_cmp(&fb.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    fields
}

/// Linearize an sRGB channel (0–255) per the WCAG luminance formula.
fn linearize(channel: f64) -> f64 {
    let srgb = channel / 255.0;
    if srgb <= 0.040_45 {
        srgb / 12.92
    } else {
        ((srgb + 0.055) / 1.055).powf(2.4)
    }
}

/// Parse a 6-digit hex colour into an RGB triple.
fn parse_hex(hex: &str) -> Option<(f64, f64, f64)> {
    let clean = hex.strip_prefix('#').unwrap_or(hex);
    if clean.len() != 6 || !clean.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = i64::from_str_radix(&clean[0..2], 16).ok()?;
    let g = i64::from_str_radix(&clean[2..4], 16).ok()?;
    let b = i64::from_str_radix(&clean[4..6], 16).ok()?;
    Some((r as f64, g as f64, b as f64))
}

/// WCAG 2.x relative luminance of a 6-digit hex colour.
fn relative_luminance(hex: &str) -> Option<f64> {
    let (r, g, b) = parse_hex(hex)?;
    Some(0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b))
}

/// WCAG 2.x contrast ratio between two hex colours.
///
/// Returns `None` if either colour is not a valid 6-digit hex string.
#[must_use]
pub fn contrast_ratio(fg: &str, bg: &str) -> Option<f64> {
    let l1 = relative_luminance(fg)?;
    let l2 = relative_luminance(bg)?;
    let lighter = l1.max(l2);
    let darker = l1.min(l2);
    Some((lighter + 0.05) / (darker + 0.05))
}

/// Whether the contrast between `fg` and `bg` meets WCAG AA for the font size.
#[must_use]
pub fn meets_contrast_aa(fg: &str, bg: &str, font_size: f64) -> bool {
    match contrast_ratio(fg, bg) {
        Some(ratio) => {
            if font_size >= 18.0 {
                ratio >= 3.0
            } else {
                ratio >= 4.5
            }
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_id_format() {
        assert_eq!(generate_field_id("name", 0), "odin-field-0-name");
    }

    #[test]
    fn contrast_black_on_white() {
        let ratio = contrast_ratio("#000000", "#ffffff").unwrap();
        assert!((ratio - 21.0).abs() < 0.01);
    }

    #[test]
    fn contrast_aa_thresholds() {
        assert!(meets_contrast_aa("#000000", "#ffffff", 12.0));
        assert!(!meets_contrast_aa("#777777", "#888888", 12.0));
    }
}
