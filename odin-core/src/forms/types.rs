//! Type model for ODIN Forms 1.0 documents.

use std::collections::BTreeMap;

/// Root ODIN Forms document.
#[derive(Debug, Clone, PartialEq)]
pub struct OdinForm {
    /// Document-level metadata (`{$}`).
    pub metadata: FormMetadata,
    /// Default page dimensions and margins (`{$.page}`).
    pub page_defaults: Option<PageDefaults>,
    /// Screen rendering options (`{$.screen}`).
    pub screen: Option<ScreenSettings>,
    /// Multi-language label dictionary (`{$.i18n}`).
    pub i18n: Option<BTreeMap<String, String>>,
    /// Ordered list of form pages.
    pub pages: Vec<FormPage>,
    /// Page templates (`{@tpl_*}`) keyed by template name.
    pub templates: Option<BTreeMap<String, PageTemplate>>,
}

/// Document-level metadata from the `{$}` header.
#[derive(Debug, Clone, PartialEq)]
pub struct FormMetadata {
    /// Human-readable form title.
    pub title: String,
    /// Unique form identifier.
    pub id: String,
    /// Primary language code (e.g. `en`).
    pub lang: String,
    /// ODIN Forms schema version.
    pub version: Option<String>,
}

/// Measurement unit for page coordinates and dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Inches.
    Inch,
    /// Centimeters.
    Cm,
    /// Millimeters.
    Mm,
    /// Points (72 per inch).
    Pt,
}

impl Unit {
    /// String form used in `$.page.unit` and pixel conversions.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Unit::Inch => "inch",
            Unit::Cm => "cm",
            Unit::Mm => "mm",
            Unit::Pt => "pt",
        }
    }

    /// Parse a unit string, defaulting to `inch` for unknown input.
    #[must_use]
    pub fn parse(value: &str) -> Unit {
        match value {
            "cm" => Unit::Cm,
            "mm" => Unit::Mm,
            "pt" => Unit::Pt,
            _ => Unit::Inch,
        }
    }
}

/// Default page dimensions applied to all pages.
#[derive(Debug, Clone, PartialEq)]
pub struct PageDefaults {
    /// Page width in the declared unit.
    pub width: f64,
    /// Page height in the declared unit.
    pub height: f64,
    /// Measurement unit.
    pub unit: Unit,
    /// Per-side page margins.
    pub margin: Option<PageMargins>,
}

/// Per-side page margins.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageMargins {
    /// Top margin.
    pub top: Option<f64>,
    /// Right margin.
    pub right: Option<f64>,
    /// Bottom margin.
    pub bottom: Option<f64>,
    /// Left margin.
    pub left: Option<f64>,
}

/// Optional screen/web rendering settings.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenSettings {
    /// Default zoom factor. 1.0 = 100%.
    pub scale: f64,
}

/// A single form page with an ordered list of elements.
#[derive(Debug, Clone, PartialEq)]
pub struct FormPage {
    /// All elements on this page, in document order.
    pub elements: Vec<FormElement>,
}

/// Stroke properties for geometric elements.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stroke {
    /// Stroke colour as a 6-digit hex string.
    pub stroke: Option<String>,
    /// Stroke width in the page unit.
    pub stroke_width: Option<f64>,
    /// Stroke opacity in [0, 1].
    pub stroke_opacity: Option<f64>,
    /// Stroke dash pattern.
    pub stroke_dasharray: Option<String>,
    /// Line cap style.
    pub stroke_linecap: Option<String>,
    /// Line join style.
    pub stroke_linejoin: Option<String>,
}

/// Fill properties for closed shapes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fill {
    /// Fill colour as a 6-digit hex string, or `none`.
    pub fill: Option<String>,
    /// Fill opacity in [0, 1].
    pub fill_opacity: Option<f64>,
}

/// Typography properties for text-bearing elements.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Font {
    /// Font family name.
    pub font_family: Option<String>,
    /// Font size in points.
    pub font_size: Option<f64>,
    /// Font weight (`normal` or `bold`).
    pub font_weight: Option<String>,
    /// Font style (`normal` or `italic`).
    pub font_style: Option<String>,
    /// Horizontal text alignment.
    pub text_align: Option<String>,
    /// Text colour as a 6-digit hex string.
    pub color: Option<String>,
}

/// Field validation constraints.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Validation {
    /// Whether the field must have a value.
    pub required: Option<bool>,
    /// Regular expression the value must match.
    pub pattern: Option<String>,
    /// Minimum string length.
    pub min_length: Option<i64>,
    /// Maximum string length.
    pub max_length: Option<i64>,
    /// Minimum value (number or date string).
    pub min: Option<String>,
    /// Maximum value (number or date string).
    pub max: Option<String>,
}

/// Barcode symbology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarcodeType {
    /// Code 39.
    Code39,
    /// Code 128.
    Code128,
    /// QR code.
    Qr,
    /// Data Matrix.
    Datamatrix,
    /// PDF417.
    Pdf417,
}

impl BarcodeType {
    /// String form used in markup and the `type` property.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            BarcodeType::Code39 => "code39",
            BarcodeType::Code128 => "code128",
            BarcodeType::Qr => "qr",
            BarcodeType::Datamatrix => "datamatrix",
            BarcodeType::Pdf417 => "pdf417",
        }
    }

    /// Parse a barcode type, defaulting to `code128` for unknown input.
    #[must_use]
    pub fn parse(value: &str) -> BarcodeType {
        match value {
            "code39" => BarcodeType::Code39,
            "qr" => BarcodeType::Qr,
            "datamatrix" => BarcodeType::Datamatrix,
            "pdf417" => BarcodeType::Pdf417,
            _ => BarcodeType::Code128,
        }
    }
}

/// HTML5 input type hint for text fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputType {
    /// Plain text.
    Text,
    /// Email address.
    Email,
    /// Telephone number.
    Tel,
    /// Password.
    Password,
    /// Number.
    Number,
    /// URL.
    Url,
}

impl InputType {
    /// String form used in the HTML `type` attribute.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            InputType::Text => "text",
            InputType::Email => "email",
            InputType::Tel => "tel",
            InputType::Password => "password",
            InputType::Number => "number",
            InputType::Url => "url",
        }
    }

    /// Parse a recognised input type, returning `None` otherwise.
    #[must_use]
    pub fn parse(value: &str) -> Option<InputType> {
        match value {
            "text" => Some(InputType::Text),
            "email" => Some(InputType::Email),
            "tel" => Some(InputType::Tel),
            "password" => Some(InputType::Password),
            "number" => Some(InputType::Number),
            "url" => Some(InputType::Url),
            _ => None,
        }
    }
}

/// Properties shared by every form element.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementBase {
    /// Element name, taken from the path key.
    pub name: String,
    /// Stable identifier for programmatic access.
    pub id: String,
}

/// Properties shared by all field elements.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldBase {
    /// Absolute X position.
    pub x: f64,
    /// Absolute Y position.
    pub y: f64,
    /// Element width.
    pub w: f64,
    /// Element height.
    pub h: f64,
    /// Visible label text. Also the default ARIA label.
    pub label: String,
    /// ARIA label override.
    pub aria_label: Option<String>,
    /// Tab order index.
    pub tabindex: Option<i64>,
    /// Whether the field is read-only.
    pub readonly: Option<bool>,
    /// Validation constraints.
    pub validation: Validation,
    /// ODIN path reference for the field's value.
    pub bind: String,
    /// Per-item vertical offset (region children only).
    pub y_offset: Option<f64>,
    /// Per-item horizontal offset (region children only).
    pub x_offset: Option<f64>,
}

/// A line segment between two endpoints.
#[derive(Debug, Clone, PartialEq)]
pub struct LineElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Start X.
    pub x1: f64,
    /// Start Y.
    pub y1: f64,
    /// End X.
    pub x2: f64,
    /// End Y.
    pub y2: f64,
    /// Stroke properties.
    pub stroke: Stroke,
}

/// A rectangle, optionally with rounded corners.
#[derive(Debug, Clone, PartialEq)]
pub struct RectElement {
    /// Common element properties.
    pub base: ElementBase,
    /// X position.
    pub x: f64,
    /// Y position.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
    /// Horizontal corner radius.
    pub rx: Option<f64>,
    /// Vertical corner radius.
    pub ry: Option<f64>,
    /// Stroke properties.
    pub stroke: Stroke,
    /// Fill properties.
    pub fill: Fill,
}

/// A circle defined by center and radius.
#[derive(Debug, Clone, PartialEq)]
pub struct CircleElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Center X.
    pub cx: f64,
    /// Center Y.
    pub cy: f64,
    /// Radius.
    pub r: f64,
    /// Stroke properties.
    pub stroke: Stroke,
    /// Fill properties.
    pub fill: Fill,
}

/// An ellipse defined by center and two radii.
#[derive(Debug, Clone, PartialEq)]
pub struct EllipseElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Center X.
    pub cx: f64,
    /// Center Y.
    pub cy: f64,
    /// Horizontal radius.
    pub rx: f64,
    /// Vertical radius.
    pub ry: f64,
    /// Stroke properties.
    pub stroke: Stroke,
    /// Fill properties.
    pub fill: Fill,
}

/// A closed polygon defined by points.
#[derive(Debug, Clone, PartialEq)]
pub struct PolygonElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Space-separated coordinate pairs.
    pub points: String,
    /// Stroke properties.
    pub stroke: Stroke,
    /// Fill properties.
    pub fill: Fill,
}

/// An open polyline defined by points.
#[derive(Debug, Clone, PartialEq)]
pub struct PolylineElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Space-separated coordinate pairs.
    pub points: String,
    /// Stroke properties.
    pub stroke: Stroke,
}

/// An SVG-style arbitrary path.
#[derive(Debug, Clone, PartialEq)]
pub struct PathElement {
    /// Common element properties.
    pub base: ElementBase,
    /// SVG path data string.
    pub d: String,
    /// Stroke properties.
    pub stroke: Stroke,
    /// Fill properties.
    pub fill: Fill,
}

/// Static text label.
#[derive(Debug, Clone, PartialEq)]
pub struct TextElement {
    /// Common element properties.
    pub base: ElementBase,
    /// X position.
    pub x: f64,
    /// Y position.
    pub y: f64,
    /// The text string to render.
    pub content: String,
    /// Rotation angle in degrees.
    pub rotate: Option<f64>,
    /// Typography properties.
    pub font: Font,
    /// Per-item vertical offset (region children only).
    pub y_offset: Option<f64>,
    /// Per-item horizontal offset (region children only).
    pub x_offset: Option<f64>,
}

/// Embedded image.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageElement {
    /// Common element properties.
    pub base: ElementBase,
    /// X position.
    pub x: f64,
    /// Y position.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
    /// Base64-encoded image data with format prefix.
    pub src: String,
    /// Accessibility description.
    pub alt: String,
    /// Whether the image renders behind all other elements.
    pub background: Option<bool>,
}

/// 1D or 2D barcode.
#[derive(Debug, Clone, PartialEq)]
pub struct BarcodeElement {
    /// Common element properties.
    pub base: ElementBase,
    /// X position.
    pub x: f64,
    /// Y position.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
    /// Barcode symbology.
    pub barcode_type: BarcodeType,
    /// Data to encode.
    pub content: String,
    /// Accessibility description.
    pub alt: String,
}

/// Single- or multi-line text input field.
#[derive(Debug, Clone, PartialEq)]
pub struct TextFieldElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Field-level properties.
    pub field: FieldBase,
    /// Inline text value.
    pub value: Option<String>,
    /// HTML5 input type hint.
    pub input_type: Option<InputType>,
    /// Input mask pattern.
    pub mask: Option<String>,
    /// Placeholder text.
    pub placeholder: Option<String>,
    /// Whether multiple lines are accepted.
    pub multiline: Option<bool>,
    /// Maximum lines when multiline.
    pub max_lines: Option<i64>,
}

/// Boolean checkbox field.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckboxElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Field-level properties.
    pub field: FieldBase,
    /// Whether the checkbox is checked.
    pub checked: Option<bool>,
}

/// Radio button field, part of a group.
#[derive(Debug, Clone, PartialEq)]
pub struct RadioElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Field-level properties.
    pub field: FieldBase,
    /// Radio group name.
    pub group: String,
    /// Value emitted when selected.
    pub value: String,
}

/// Single-selection dropdown field.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Field-level properties.
    pub field: FieldBase,
    /// Valid option values.
    pub options: Vec<String>,
    /// Currently selected option value.
    pub selected: Option<String>,
    /// Default unselected text.
    pub placeholder: Option<String>,
}

/// Multiple-selection list field.
#[derive(Debug, Clone, PartialEq)]
pub struct MultiselectElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Field-level properties.
    pub field: FieldBase,
    /// Valid option values.
    pub options: Vec<String>,
    /// Currently selected option values.
    pub selected: Option<Vec<String>>,
    /// Minimum selections required.
    pub min_select: Option<i64>,
    /// Maximum selections allowed.
    pub max_select: Option<i64>,
}

/// Date input field.
#[derive(Debug, Clone, PartialEq)]
pub struct DateElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Field-level properties.
    pub field: FieldBase,
    /// Inline date value as an ISO 8601 string.
    pub value: Option<String>,
}

/// Signature capture area.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureElement {
    /// Common element properties.
    pub base: ElementBase,
    /// Field-level properties.
    pub field: FieldBase,
    /// Captured signature data as an ODIN binary literal.
    pub value: Option<String>,
    /// Reference to an associated date field.
    pub date_field: Option<String>,
}

/// A container grouping repeating content bound to an array.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionElement {
    /// Common element properties.
    pub base: ElementBase,
    /// X position.
    pub x: f64,
    /// Y position.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
    /// ODIN path to the array data source.
    pub bind: Option<String>,
    /// Maximum items before overflow.
    pub max: Option<i64>,
    /// `clone` or a template reference.
    pub overflow: Option<String>,
    /// Child elements, repeated per bound item.
    pub children: Vec<FormElement>,
}

/// A page template for dynamically generated overflow pages.
#[derive(Debug, Clone, PartialEq)]
pub struct PageTemplate {
    /// Template name (e.g. `tpl_vehicles_continued`).
    pub name: String,
    /// Marks this as a template.
    pub page_template: bool,
    /// Names the region this template continues.
    pub continues: Option<String>,
    /// Form identifier for continuation pages.
    pub form_id: Option<String>,
    /// Elements contained in the template.
    pub elements: Vec<FormElement>,
}

/// Discriminated union of all concrete form element types.
#[derive(Debug, Clone, PartialEq)]
pub enum FormElement {
    /// Line segment.
    Line(LineElement),
    /// Rectangle.
    Rect(RectElement),
    /// Circle.
    Circle(CircleElement),
    /// Ellipse.
    Ellipse(EllipseElement),
    /// Polygon.
    Polygon(PolygonElement),
    /// Polyline.
    Polyline(PolylineElement),
    /// Path.
    Path(PathElement),
    /// Static text.
    Text(TextElement),
    /// Image.
    Image(ImageElement),
    /// Barcode.
    Barcode(BarcodeElement),
    /// Text field.
    TextField(TextFieldElement),
    /// Checkbox field.
    Checkbox(CheckboxElement),
    /// Radio field.
    Radio(RadioElement),
    /// Select field.
    Select(SelectElement),
    /// Multiselect field.
    Multiselect(MultiselectElement),
    /// Date field.
    Date(DateElement),
    /// Signature field.
    Signature(SignatureElement),
    /// Region container.
    Region(RegionElement),
}

impl FormElement {
    /// Element type discriminator string (e.g. `field.text`, `region`).
    #[must_use]
    pub fn type_str(&self) -> &'static str {
        match self {
            FormElement::Line(_) => "line",
            FormElement::Rect(_) => "rect",
            FormElement::Circle(_) => "circle",
            FormElement::Ellipse(_) => "ellipse",
            FormElement::Polygon(_) => "polygon",
            FormElement::Polyline(_) => "polyline",
            FormElement::Path(_) => "path",
            FormElement::Text(_) => "text",
            FormElement::Image(_) => "img",
            FormElement::Barcode(_) => "barcode",
            FormElement::TextField(_) => "field.text",
            FormElement::Checkbox(_) => "field.checkbox",
            FormElement::Radio(_) => "field.radio",
            FormElement::Select(_) => "field.select",
            FormElement::Multiselect(_) => "field.multiselect",
            FormElement::Date(_) => "field.date",
            FormElement::Signature(_) => "field.signature",
            FormElement::Region(_) => "region",
        }
    }

    /// Element name from its path key.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            FormElement::Line(e) => &e.base.name,
            FormElement::Rect(e) => &e.base.name,
            FormElement::Circle(e) => &e.base.name,
            FormElement::Ellipse(e) => &e.base.name,
            FormElement::Polygon(e) => &e.base.name,
            FormElement::Polyline(e) => &e.base.name,
            FormElement::Path(e) => &e.base.name,
            FormElement::Text(e) => &e.base.name,
            FormElement::Image(e) => &e.base.name,
            FormElement::Barcode(e) => &e.base.name,
            FormElement::TextField(e) => &e.base.name,
            FormElement::Checkbox(e) => &e.base.name,
            FormElement::Radio(e) => &e.base.name,
            FormElement::Select(e) => &e.base.name,
            FormElement::Multiselect(e) => &e.base.name,
            FormElement::Date(e) => &e.base.name,
            FormElement::Signature(e) => &e.base.name,
            FormElement::Region(e) => &e.base.name,
        }
    }

    /// Field-level properties when this element is a field, else `None`.
    #[must_use]
    pub fn as_field(&self) -> Option<&FieldBase> {
        match self {
            FormElement::TextField(e) => Some(&e.field),
            FormElement::Checkbox(e) => Some(&e.field),
            FormElement::Radio(e) => Some(&e.field),
            FormElement::Select(e) => Some(&e.field),
            FormElement::Multiselect(e) => Some(&e.field),
            FormElement::Date(e) => Some(&e.field),
            FormElement::Signature(e) => Some(&e.field),
            _ => None,
        }
    }

    /// Whether this element is a field type.
    #[must_use]
    pub fn is_field(&self) -> bool {
        self.as_field().is_some()
    }
}

/// Rendering target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTarget {
    /// Interactive HTML for screen display.
    Html,
    /// Static HTML/CSS optimised for print.
    PrintCss,
}

/// Options passed to the form renderer.
#[derive(Debug, Clone, Default)]
pub struct RenderFormOptions {
    /// Rendering target.
    pub target: Option<RenderTarget>,
    /// Language code for i18n label resolution.
    pub lang: Option<String>,
    /// Uniform scale factor.
    pub scale: Option<f64>,
    /// Additional CSS class on the root element.
    pub class_name: Option<String>,
}
