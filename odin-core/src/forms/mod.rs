//! ODIN Forms 1.0 — declarative print/screen form definitions.
//!
//! Parse ODIN Forms text into a typed model, render the model to accessible
//! HTML, and generate the supporting CSS.

mod accessibility;
mod css;
mod parser;
mod renderer;
pub mod types;
mod units;

pub use accessibility::{
    contrast_ratio, field_aria_label, field_aria_required, field_group_html, field_label_html,
    generate_field_id, meets_contrast_aa, skip_link_html, sr_only_html, tab_order_sort,
};
pub use css::{generate_form_css, generate_print_css};
pub use parser::parse_form;
pub use renderer::render_form;
pub use units::{from_pixels, to_pixels};

pub use types::{
    BarcodeElement, BarcodeType, CheckboxElement, CircleElement, DateElement, ElementBase,
    EllipseElement, FieldBase, Fill, Font, FormElement, FormMetadata, FormPage, ImageElement,
    InputType, LineElement, MultiselectElement, OdinForm, PageDefaults, PageMargins, PageTemplate,
    PathElement, PolygonElement, PolylineElement, RadioElement, RectElement, RegionElement,
    RenderFormOptions, RenderTarget, ScreenSettings, SelectElement, SignatureElement, Stroke,
    TextElement, TextFieldElement, Unit, Validation,
};

#[cfg(test)]
mod tests;
