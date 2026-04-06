//! ODIN Value Types
//!
//! Unified type system for ODIN values with embedded modifiers.
//! This is the canonical data model (CDM) representation used throughout
//! the SDK, including parsing, transformation, and serialization.
//!
//! Design principles:
//! 1. Each value is self-contained (type + value + modifiers)
//! 2. No separate modifier maps — modifiers are intrinsic to values
//! 3. Factory methods create values with proper types
//! 4. Strict typing prevents non-ODIN types from leaking in
//! 5. Language-agnostic naming for cross-SDK portability

use std::fmt;

// ─────────────────────────────────────────────────────────────────────────────
// Modifiers (embedded in all value types)
// ─────────────────────────────────────────────────────────────────────────────

/// Modifiers that can be applied to any ODIN value.
///
/// In ODIN notation:
/// - `!` prefix = required (called "critical" in some contexts)
/// - `*` prefix = confidential (should be redacted/masked)
/// - `-` prefix = deprecated (obsolete, may be removed)
/// - `:attr` = emit as XML attribute
///
/// Modifiers can be combined: `!*"secret"` is both required and confidential.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OdinModifiers {
    /// Field is required (`!` modifier).
    pub required: bool,
    /// Value should be masked/redacted (`*` modifier).
    pub confidential: bool,
    /// Field is deprecated (`-` modifier).
    pub deprecated: bool,
    /// Emit as XML attribute instead of child element (`:attr` modifier).
    pub attr: bool,
}

impl OdinModifiers {
    /// Returns `true` if no modifiers are set.
    pub fn is_empty(&self) -> bool {
        !self.required && !self.confidential && !self.deprecated && !self.attr
    }

    /// Returns `true` if any modifier is set.
    pub fn has_any(&self) -> bool {
        !self.is_empty()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Directives
// ─────────────────────────────────────────────────────────────────────────────

/// Trailing directives that can follow any ODIN value.
///
/// In ODIN notation, directives follow values with colon prefix:
/// - `:pos 3` — position directive
/// - `:len 8` — length directive
/// - `:format ssn` — format directive
/// - `:trim` — trim directive (no value)
///
/// Example: `field = @_line :pos 3 :len 8 :trim`
#[derive(Debug, Clone, PartialEq)]
pub struct OdinDirective {
    /// Directive name (e.g., "pos", "len", "format", "trim").
    pub name: String,
    /// Optional directive value (e.g., 3, 8, "ssn").
    pub value: Option<DirectiveValue>,
}

/// Value of a directive — either a string or a number.
#[derive(Debug, Clone, PartialEq)]
pub enum DirectiveValue {
    /// String value (e.g., "ssn").
    String(String),
    /// Numeric value (e.g., 3, 8).
    Number(f64),
}

// ─────────────────────────────────────────────────────────────────────────────
// Value Type Discriminator
// ─────────────────────────────────────────────────────────────────────────────

/// All possible ODIN value types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OdinValueType {
    /// Null type (`~`).
    Null,
    /// Boolean type (`true`/`false`).
    Boolean,
    /// String type (quoted or bare word).
    String,
    /// Integer type (`##`).
    Integer,
    /// Decimal number type (`#`).
    Number,
    /// Currency type (`#$`).
    Currency,
    /// Percentage type (`#%`).
    Percent,
    /// Calendar date type.
    Date,
    /// Date-time timestamp type.
    Timestamp,
    /// Time-of-day type.
    Time,
    /// ISO 8601 duration type.
    Duration,
    /// Reference type (`@`).
    Reference,
    /// Binary/base64 type (`^`).
    Binary,
    /// Verb expression type (`%`).
    Verb,
    /// Array type.
    Array,
    /// Object type (nested key-value pairs).
    Object,
}

impl fmt::Display for OdinValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => write!(f, "null"),
            Self::Boolean => write!(f, "boolean"),
            Self::String => write!(f, "string"),
            Self::Integer => write!(f, "integer"),
            Self::Number => write!(f, "number"),
            Self::Currency => write!(f, "currency"),
            Self::Percent => write!(f, "percent"),
            Self::Date => write!(f, "date"),
            Self::Timestamp => write!(f, "timestamp"),
            Self::Time => write!(f, "time"),
            Self::Duration => write!(f, "duration"),
            Self::Reference => write!(f, "reference"),
            Self::Binary => write!(f, "binary"),
            Self::Verb => write!(f, "verb"),
            Self::Array => write!(f, "array"),
            Self::Object => write!(f, "object"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Core Value Enum (16 variants)
// ─────────────────────────────────────────────────────────────────────────────

/// The canonical ODIN value type — a discriminated union of all 16 value types.
///
/// Each variant carries its payload plus optional modifiers and directives.
/// Values are immutable by convention; create new values rather than mutating.
#[derive(Debug, Clone, PartialEq)]
pub enum OdinValue {
    /// Null value (`~`).
    Null {
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Boolean value (`true`, `false`, `?true`, `?false`).
    Boolean {
        /// The boolean payload.
        value: bool,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// String value (bare word or quoted).
    String {
        /// The string payload.
        value: String,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Integer value (`##42`, `##-17`).
    ///
    /// For values beyond `i64::MAX`, the `raw` field preserves the exact string
    /// representation while `value` contains the best-effort numeric value.
    Integer {
        /// The integer payload.
        value: i64,
        /// Original string representation for round-trip preservation of large values.
        raw: Option<String>,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Decimal number value (`#99.99`, `#3.14159`).
    ///
    /// For high-precision values, the `raw` field preserves the exact string
    /// to avoid floating-point precision loss during round-trip.
    Number {
        /// The floating-point payload.
        value: f64,
        /// Number of decimal places (for formatting).
        decimal_places: Option<u8>,
        /// Original string representation for round-trip preservation.
        raw: Option<String>,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Currency value (`#$100.00`, `#$1234.5678`).
    ///
    /// Currency values have a fixed number of decimal places (default 2).
    /// Currency codes can be specified: `#$100.00:USD`.
    Currency {
        /// The currency amount as floating-point.
        value: f64,
        /// Number of decimal places (default 2).
        decimal_places: u8,
        /// Optional currency code (e.g., "USD", "EUR").
        currency_code: Option<String>,
        /// Original string representation for round-trip preservation.
        raw: Option<String>,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Percentage value (`#%0.15` for 15%).
    ///
    /// Stored as decimal in 0-1 range. Values outside range are permitted
    /// (e.g., `#%1.5` for 150%).
    Percent {
        /// The percentage as a decimal (0.15 = 15%).
        value: f64,
        /// Original string representation for round-trip preservation.
        raw: Option<String>,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Date value (`2024-06-15`).
    ///
    /// Dates represent a calendar day without time. Stored as components
    /// rather than using a chrono dependency.
    Date {
        /// Calendar year.
        year: i32,
        /// Month of the year (1-12).
        month: u8,
        /// Day of the month (1-31).
        day: u8,
        /// Original string representation (required for round-trip).
        raw: String,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Timestamp value (`2024-06-15T14:30:00Z`).
    ///
    /// Timestamps represent a specific instant. Stored as milliseconds since
    /// Unix epoch for arithmetic, with `raw` for exact round-trip.
    Timestamp {
        /// Milliseconds since Unix epoch (1970-01-01T00:00:00Z).
        epoch_ms: i64,
        /// Original string representation (required for round-trip).
        raw: String,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Time value (`T14:30:00`, `T09:15:30.500`).
    ///
    /// Times represent a time of day without a date.
    Time {
        /// Time string with T prefix (e.g., "T14:30:00").
        value: String,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Duration value (`P1Y6M`, `PT30M`, `P2W`).
    ///
    /// Durations represent a span of time in ISO 8601 format.
    Duration {
        /// Duration string with P prefix (e.g., "P1Y6M").
        value: String,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Reference to another path (`@policy.id`, `@.current_item`).
    Reference {
        /// Target path (without `@` prefix).
        path: String,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Binary data (`^SGVsbG8=`, `^sha256:abc123...`).
    Binary {
        /// Decoded binary data.
        data: Vec<u8>,
        /// Algorithm if specified (e.g., "sha256", "ed25519").
        algorithm: Option<String>,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Verb expression (`%upper @name`, `%concat @first " " @last`).
    ///
    /// Verb expressions represent transformation operations as first-class values.
    Verb {
        /// Verb name (e.g., "upper", "concat", "lookup").
        verb: String,
        /// Whether this is a custom verb (`%&namespace.verb`).
        is_custom: bool,
        /// Parsed arguments (can include nested verb expressions).
        args: Vec<OdinValue>,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Array of values.
    ///
    /// Arrays hold ordered collections that can be:
    /// - Arrays of objects (for ODIN array-of-records syntax)
    /// - Arrays of typed values (for flat arrays from transforms)
    Array {
        /// Ordered array elements.
        items: Vec<OdinArrayItem>,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },

    /// Object value (nested key-value pairs).
    Object {
        /// Ordered key-value pairs.
        value: Vec<(String, OdinValue)>,
        /// Modifiers applied to this value.
        modifiers: Option<OdinModifiers>,
        /// Trailing directives.
        directives: Vec<OdinDirective>,
    },
}

/// Array item type — either a record (map of fields) or a direct value.
#[derive(Debug, Clone, PartialEq)]
pub enum OdinArrayItem {
    /// An object record with named fields (for ODIN array-of-records syntax).
    Record(Vec<(String, OdinValue)>),
    /// A direct typed value (for flat arrays from transforms).
    Value(OdinValue),
}

// ─────────────────────────────────────────────────────────────────────────────
// Type Accessors
// ─────────────────────────────────────────────────────────────────────────────

impl OdinValue {
    /// Returns the type discriminator for this value.
    pub fn value_type(&self) -> OdinValueType {
        match self {
            Self::Null { .. } => OdinValueType::Null,
            Self::Boolean { .. } => OdinValueType::Boolean,
            Self::String { .. } => OdinValueType::String,
            Self::Integer { .. } => OdinValueType::Integer,
            Self::Number { .. } => OdinValueType::Number,
            Self::Currency { .. } => OdinValueType::Currency,
            Self::Percent { .. } => OdinValueType::Percent,
            Self::Date { .. } => OdinValueType::Date,
            Self::Timestamp { .. } => OdinValueType::Timestamp,
            Self::Time { .. } => OdinValueType::Time,
            Self::Duration { .. } => OdinValueType::Duration,
            Self::Reference { .. } => OdinValueType::Reference,
            Self::Binary { .. } => OdinValueType::Binary,
            Self::Verb { .. } => OdinValueType::Verb,
            Self::Array { .. } => OdinValueType::Array,
            Self::Object { .. } => OdinValueType::Object,
        }
    }

    /// Returns the modifiers for this value, if any.
    pub fn modifiers(&self) -> Option<&OdinModifiers> {
        match self {
            Self::Null { modifiers, .. }
            | Self::Boolean { modifiers, .. }
            | Self::String { modifiers, .. }
            | Self::Integer { modifiers, .. }
            | Self::Number { modifiers, .. }
            | Self::Currency { modifiers, .. }
            | Self::Percent { modifiers, .. }
            | Self::Date { modifiers, .. }
            | Self::Timestamp { modifiers, .. }
            | Self::Time { modifiers, .. }
            | Self::Duration { modifiers, .. }
            | Self::Reference { modifiers, .. }
            | Self::Binary { modifiers, .. }
            | Self::Verb { modifiers, .. }
            | Self::Array { modifiers, .. }
            | Self::Object { modifiers, .. } => modifiers.as_ref(),
        }
    }

    /// Returns the directives for this value.
    pub fn directives(&self) -> &[OdinDirective] {
        match self {
            Self::Null { directives, .. }
            | Self::Boolean { directives, .. }
            | Self::String { directives, .. }
            | Self::Integer { directives, .. }
            | Self::Number { directives, .. }
            | Self::Currency { directives, .. }
            | Self::Percent { directives, .. }
            | Self::Date { directives, .. }
            | Self::Timestamp { directives, .. }
            | Self::Time { directives, .. }
            | Self::Duration { directives, .. }
            | Self::Reference { directives, .. }
            | Self::Binary { directives, .. }
            | Self::Verb { directives, .. }
            | Self::Array { directives, .. }
            | Self::Object { directives, .. } => directives,
        }
    }

    /// Returns `true` if this value has the required modifier.
    pub fn is_required(&self) -> bool {
        self.modifiers().is_some_and(|m| m.required)
    }

    /// Returns `true` if this value has the confidential modifier.
    pub fn is_confidential(&self) -> bool {
        self.modifiers().is_some_and(|m| m.confidential)
    }

    /// Returns `true` if this value has the deprecated modifier.
    pub fn is_deprecated(&self) -> bool {
        self.modifiers().is_some_and(|m| m.deprecated)
    }

    /// Returns `true` if this is a null value.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null { .. })
    }

    /// Returns `true` if this is a boolean value.
    pub fn is_boolean(&self) -> bool {
        matches!(self, Self::Boolean { .. })
    }

    /// Returns `true` if this is a string value.
    pub fn is_string(&self) -> bool {
        matches!(self, Self::String { .. })
    }

    /// Returns `true` if this is an integer value.
    pub fn is_integer(&self) -> bool {
        matches!(self, Self::Integer { .. })
    }

    /// Returns `true` if this is a number value.
    pub fn is_number(&self) -> bool {
        matches!(self, Self::Number { .. })
    }

    /// Returns `true` if this is a currency value.
    pub fn is_currency(&self) -> bool {
        matches!(self, Self::Currency { .. })
    }

    /// Returns `true` if this is a percent value.
    pub fn is_percent(&self) -> bool {
        matches!(self, Self::Percent { .. })
    }

    /// Returns `true` if this is any numeric type (integer, number, currency, or percent).
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            Self::Integer { .. } | Self::Number { .. } | Self::Currency { .. } | Self::Percent { .. }
        )
    }

    /// Returns `true` if this is any temporal type (date, timestamp, time, or duration).
    pub fn is_temporal(&self) -> bool {
        matches!(
            self,
            Self::Date { .. } | Self::Timestamp { .. } | Self::Time { .. } | Self::Duration { .. }
        )
    }

    /// Returns `true` if this is a date value.
    pub fn is_date(&self) -> bool {
        matches!(self, Self::Date { .. })
    }

    /// Returns `true` if this is a timestamp value.
    pub fn is_timestamp(&self) -> bool {
        matches!(self, Self::Timestamp { .. })
    }

    /// Returns `true` if this is a reference value.
    pub fn is_reference(&self) -> bool {
        matches!(self, Self::Reference { .. })
    }

    /// Returns `true` if this is a binary value.
    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Binary { .. })
    }

    /// Returns `true` if this is a verb expression.
    pub fn is_verb(&self) -> bool {
        matches!(self, Self::Verb { .. })
    }

    /// Returns `true` if this is an array value.
    pub fn is_array(&self) -> bool {
        matches!(self, Self::Array { .. })
    }

    /// Returns `true` if this is an object value.
    pub fn is_object(&self) -> bool {
        matches!(self, Self::Object { .. })
    }

    /// Extract the boolean value, if this is a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// Extract the string value, if this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String { value, .. } => Some(value),
            _ => None,
        }
    }

    /// Extract the integer value, if this is an integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// Extract the numeric value as f64 (works for number, integer, currency, percent).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number { value, .. }
            | Self::Currency { value, .. }
            | Self::Percent { value, .. } => Some(*value),
            Self::Integer { value, .. } => Some(*value as f64),
            _ => None,
        }
    }

    /// Extract the reference path, if this is a reference.
    pub fn as_reference(&self) -> Option<&str> {
        match self {
            Self::Reference { path, .. } => Some(path),
            _ => None,
        }
    }

    /// Extract the array items, if this is an array.
    pub fn as_array(&self) -> Option<&[OdinArrayItem]> {
        match self {
            Self::Array { items, .. } => Some(items),
            _ => None,
        }
    }

    /// Create a new value with the given modifiers applied.
    pub fn with_modifiers(mut self, new_modifiers: OdinModifiers) -> Self {
        let mods = if new_modifiers.is_empty() {
            None
        } else {
            Some(new_modifiers)
        };
        match &mut self {
            Self::Null { modifiers, .. }
            | Self::Boolean { modifiers, .. }
            | Self::String { modifiers, .. }
            | Self::Integer { modifiers, .. }
            | Self::Number { modifiers, .. }
            | Self::Currency { modifiers, .. }
            | Self::Percent { modifiers, .. }
            | Self::Date { modifiers, .. }
            | Self::Timestamp { modifiers, .. }
            | Self::Time { modifiers, .. }
            | Self::Duration { modifiers, .. }
            | Self::Reference { modifiers, .. }
            | Self::Binary { modifiers, .. }
            | Self::Verb { modifiers, .. }
            | Self::Array { modifiers, .. }
            | Self::Object { modifiers, .. } => *modifiers = mods,
        }
        self
    }

    /// Return a copy with the given directives attached.
    pub fn with_directives(mut self, new_directives: Vec<OdinDirective>) -> Self {
        match &mut self {
            Self::Null { directives, .. }
            | Self::Boolean { directives, .. }
            | Self::String { directives, .. }
            | Self::Integer { directives, .. }
            | Self::Number { directives, .. }
            | Self::Currency { directives, .. }
            | Self::Percent { directives, .. }
            | Self::Date { directives, .. }
            | Self::Timestamp { directives, .. }
            | Self::Time { directives, .. }
            | Self::Duration { directives, .. }
            | Self::Reference { directives, .. }
            | Self::Binary { directives, .. }
            | Self::Verb { directives, .. }
            | Self::Array { directives, .. }
            | Self::Object { directives, .. } => *directives = new_directives,
        }
        self
    }
}

impl fmt::Display for OdinValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null { .. } => write!(f, "~"),
            Self::Boolean { value, .. } => write!(f, "{value}"),
            Self::String { value, .. } => write!(f, "\"{value}\""),
            Self::Integer { value, raw, .. } => {
                if let Some(r) = raw {
                    write!(f, "##{r}")
                } else {
                    write!(f, "##{value}")
                }
            }
            Self::Number { value, raw, .. } => {
                if let Some(r) = raw {
                    write!(f, "#{r}")
                } else {
                    write!(f, "#{value}")
                }
            }
            Self::Currency { value, raw, currency_code, .. } => {
                if let Some(r) = raw {
                    write!(f, "#${r}")?;
                } else {
                    write!(f, "#${value}")?;
                }
                if let Some(code) = currency_code {
                    write!(f, ":{code}")?;
                }
                Ok(())
            }
            Self::Percent { value, raw, .. } => {
                if let Some(r) = raw {
                    write!(f, "#%{r}")
                } else {
                    write!(f, "#%{value}")
                }
            }
            Self::Date { raw, .. } | Self::Timestamp { raw, .. } => write!(f, "{raw}"),
            Self::Time { value, .. } | Self::Duration { value, .. } => write!(f, "{value}"),
            Self::Reference { path, .. } => write!(f, "@{path}"),
            Self::Binary { algorithm, .. } => {
                if let Some(alg) = algorithm {
                    write!(f, "^{alg}:<data>")
                } else {
                    write!(f, "^<data>")
                }
            }
            Self::Verb { verb, args, .. } => {
                write!(f, "%{verb}")?;
                for arg in args {
                    write!(f, " {arg}")?;
                }
                Ok(())
            }
            Self::Array { items, .. } => write!(f, "[{} items]", items.len()),
            Self::Object { value, .. } => write!(f, "{{{} fields}}", value.len()),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Factory Functions (OdinValues namespace equivalent)
// ─────────────────────────────────────────────────────────────────────────────

/// Factory methods for creating ODIN values.
///
/// These provide convenient constructors with sensible defaults.
pub struct OdinValues;

impl OdinValues {
    /// Create a null value.
    pub fn null() -> OdinValue {
        OdinValue::Null {
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a null value with modifiers.
    pub fn null_with(modifiers: OdinModifiers) -> OdinValue {
        OdinValue::Null {
            modifiers: Some(modifiers),
            directives: Vec::new(),
        }
    }

    /// Create a boolean value.
    pub fn boolean(value: bool) -> OdinValue {
        OdinValue::Boolean {
            value,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a string value.
    pub fn string(value: impl Into<String>) -> OdinValue {
        OdinValue::String {
            value: value.into(),
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create an integer value.
    pub fn integer(value: i64) -> OdinValue {
        OdinValue::Integer {
            value,
            raw: None,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create an integer from a string (for values beyond i64 range, preserves raw).
    pub fn integer_from_str(raw: &str) -> OdinValue {
        let value = raw.parse::<i64>().unwrap_or(0);
        OdinValue::Integer {
            value,
            raw: Some(raw.to_string()),
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a number value.
    pub fn number(value: f64) -> OdinValue {
        OdinValue::Number {
            value,
            decimal_places: None,
            raw: None,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a number value with specified decimal places.
    pub fn number_with_places(value: f64, decimal_places: u8) -> OdinValue {
        OdinValue::Number {
            value,
            decimal_places: Some(decimal_places),
            raw: None,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a currency value.
    pub fn currency(value: f64, decimal_places: u8) -> OdinValue {
        OdinValue::Currency {
            value,
            decimal_places,
            currency_code: None,
            raw: None,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a currency value with a currency code.
    pub fn currency_with_code(value: f64, decimal_places: u8, code: &str) -> OdinValue {
        OdinValue::Currency {
            value,
            decimal_places,
            currency_code: Some(code.to_string()),
            raw: None,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a percent value (0-1 range, e.g., 0.15 = 15%).
    pub fn percent(value: f64) -> OdinValue {
        OdinValue::Percent {
            value,
            raw: None,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a date value from components.
    pub fn date(year: i32, month: u8, day: u8) -> OdinValue {
        let raw = format!("{year:04}-{month:02}-{day:02}");
        OdinValue::Date {
            year,
            month,
            day,
            raw,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a date value from a raw string.
    pub fn date_from_str(raw: &str) -> Option<OdinValue> {
        let parts: Vec<&str> = raw.split('-').collect();
        if parts.len() != 3 {
            return None;
        }
        let year = parts[0].parse::<i32>().ok()?;
        let month = parts[1].parse::<u8>().ok()?;
        let day = parts[2].parse::<u8>().ok()?;
        Some(OdinValue::Date {
            year,
            month,
            day,
            raw: raw.to_string(),
            modifiers: None,
            directives: Vec::new(),
        })
    }

    /// Create a timestamp value from epoch milliseconds and raw string.
    pub fn timestamp(epoch_ms: i64, raw: impl Into<String>) -> OdinValue {
        OdinValue::Timestamp {
            epoch_ms,
            raw: raw.into(),
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a time value (e.g., "T14:30:00").
    pub fn time(value: impl Into<String>) -> OdinValue {
        OdinValue::Time {
            value: value.into(),
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a duration value (e.g., "P1Y6M", "PT30M").
    pub fn duration(value: impl Into<String>) -> OdinValue {
        OdinValue::Duration {
            value: value.into(),
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a reference value (path without `@` prefix).
    pub fn reference(path: impl Into<String>) -> OdinValue {
        OdinValue::Reference {
            path: path.into(),
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a binary value from raw bytes.
    pub fn binary(data: Vec<u8>) -> OdinValue {
        OdinValue::Binary {
            data,
            algorithm: None,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a binary value with an algorithm tag.
    pub fn binary_with_algorithm(data: Vec<u8>, algorithm: &str) -> OdinValue {
        OdinValue::Binary {
            data,
            algorithm: Some(algorithm.to_string()),
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a verb expression value.
    pub fn verb(name: impl Into<String>, args: Vec<OdinValue>) -> OdinValue {
        OdinValue::Verb {
            verb: name.into(),
            is_custom: false,
            args,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create a custom verb expression value.
    pub fn custom_verb(name: impl Into<String>, args: Vec<OdinValue>) -> OdinValue {
        OdinValue::Verb {
            verb: name.into(),
            is_custom: true,
            args,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create an array value.
    pub fn array(items: Vec<OdinArrayItem>) -> OdinValue {
        OdinValue::Array {
            items,
            modifiers: None,
            directives: Vec::new(),
        }
    }

    /// Create an object value from key-value pairs.
    pub fn object(fields: Vec<(String, OdinValue)>) -> OdinValue {
        OdinValue::Object {
            value: fields,
            modifiers: None,
            directives: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_type_discriminator() {
        assert_eq!(OdinValues::null().value_type(), OdinValueType::Null);
        assert_eq!(OdinValues::boolean(true).value_type(), OdinValueType::Boolean);
        assert_eq!(OdinValues::string("hello").value_type(), OdinValueType::String);
        assert_eq!(OdinValues::integer(42).value_type(), OdinValueType::Integer);
        assert_eq!(OdinValues::number(3.14).value_type(), OdinValueType::Number);
        assert_eq!(OdinValues::currency(100.0, 2).value_type(), OdinValueType::Currency);
        assert_eq!(OdinValues::percent(0.15).value_type(), OdinValueType::Percent);
        assert_eq!(OdinValues::date(2024, 6, 15).value_type(), OdinValueType::Date);
        assert_eq!(OdinValues::time("T14:30:00").value_type(), OdinValueType::Time);
        assert_eq!(OdinValues::duration("P1Y6M").value_type(), OdinValueType::Duration);
        assert_eq!(OdinValues::reference("policy.id").value_type(), OdinValueType::Reference);
        assert_eq!(OdinValues::binary(vec![1, 2, 3]).value_type(), OdinValueType::Binary);
    }

    #[test]
    fn test_modifiers() {
        let mods = OdinModifiers {
            required: true,
            confidential: true,
            deprecated: false,
            attr: false,
        };
        let val = OdinValues::string("secret").with_modifiers(mods);
        assert!(val.is_required());
        assert!(val.is_confidential());
        assert!(!val.is_deprecated());
    }

    #[test]
    fn test_empty_modifiers() {
        let mods = OdinModifiers::default();
        assert!(mods.is_empty());
        assert!(!mods.has_any());
    }

    #[test]
    fn test_type_checks() {
        let num = OdinValues::integer(42);
        assert!(num.is_numeric());
        assert!(num.is_integer());
        assert!(!num.is_temporal());

        let date = OdinValues::date(2024, 1, 1);
        assert!(date.is_temporal());
        assert!(!date.is_numeric());
    }

    #[test]
    fn test_value_extraction() {
        assert_eq!(OdinValues::boolean(true).as_bool(), Some(true));
        assert_eq!(OdinValues::string("hello").as_str(), Some("hello"));
        assert_eq!(OdinValues::integer(42).as_i64(), Some(42));
        assert_eq!(OdinValues::number(3.14).as_f64(), Some(3.14));
        assert_eq!(OdinValues::integer(42).as_f64(), Some(42.0));
        assert_eq!(OdinValues::reference("x.y").as_reference(), Some("x.y"));
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", OdinValues::null()), "~");
        assert_eq!(format!("{}", OdinValues::boolean(true)), "true");
        assert_eq!(format!("{}", OdinValues::string("hello")), "\"hello\"");
        assert_eq!(format!("{}", OdinValues::integer(42)), "##42");
        assert_eq!(format!("{}", OdinValues::number(3.14)), "#3.14");
        assert_eq!(format!("{}", OdinValues::reference("x")), "@x");
    }

    // ─── Constructor tests ───────────────────────────────────────────────

    #[test]
    fn test_null_constructor() {
        let v = OdinValues::null();
        assert!(v.is_null());
        assert!(!v.is_string());
        assert!(v.modifiers().is_none());
        assert!(v.directives().is_empty());
    }

    #[test]
    fn test_null_with_modifiers() {
        let mods = OdinModifiers { required: true, ..Default::default() };
        let v = OdinValues::null_with(mods);
        assert!(v.is_null());
        assert!(v.is_required());
    }

    #[test]
    fn test_boolean_true() {
        let v = OdinValues::boolean(true);
        assert!(v.is_boolean());
        assert_eq!(v.as_bool(), Some(true));
        assert_eq!(v.value_type(), OdinValueType::Boolean);
    }

    #[test]
    fn test_boolean_false() {
        let v = OdinValues::boolean(false);
        assert_eq!(v.as_bool(), Some(false));
    }

    #[test]
    fn test_string_constructor() {
        let v = OdinValues::string("test");
        assert!(v.is_string());
        assert_eq!(v.as_str(), Some("test"));
    }

    #[test]
    fn test_string_empty() {
        let v = OdinValues::string("");
        assert!(v.is_string());
        assert_eq!(v.as_str(), Some(""));
    }

    #[test]
    fn test_string_unicode() {
        let v = OdinValues::string("Hello \u{1F600} world");
        assert_eq!(v.as_str(), Some("Hello \u{1F600} world"));
    }

    #[test]
    fn test_integer_constructor() {
        let v = OdinValues::integer(42);
        assert!(v.is_integer());
        assert_eq!(v.as_i64(), Some(42));
    }

    #[test]
    fn test_integer_negative() {
        let v = OdinValues::integer(-100);
        assert_eq!(v.as_i64(), Some(-100));
        assert_eq!(v.as_f64(), Some(-100.0));
    }

    #[test]
    fn test_integer_zero() {
        let v = OdinValues::integer(0);
        assert_eq!(v.as_i64(), Some(0));
    }

    #[test]
    fn test_integer_max() {
        let v = OdinValues::integer(i64::MAX);
        assert_eq!(v.as_i64(), Some(i64::MAX));
    }

    #[test]
    fn test_integer_min() {
        let v = OdinValues::integer(i64::MIN);
        assert_eq!(v.as_i64(), Some(i64::MIN));
    }

    #[test]
    fn test_integer_from_str() {
        let v = OdinValues::integer_from_str("99999999999999999999");
        assert!(v.is_integer());
        // Value wraps to 0 when out of i64 range, but raw is preserved
        match &v {
            OdinValue::Integer { raw, .. } => {
                assert_eq!(raw.as_deref(), Some("99999999999999999999"));
            }
            _ => panic!("Expected integer"),
        }
    }

    #[test]
    fn test_number_constructor() {
        let v = OdinValues::number(3.14);
        assert!(v.is_number());
        assert_eq!(v.as_f64(), Some(3.14));
    }

    #[test]
    fn test_number_zero() {
        let v = OdinValues::number(0.0);
        assert_eq!(v.as_f64(), Some(0.0));
    }

    #[test]
    fn test_number_negative() {
        let v = OdinValues::number(-2.5);
        assert_eq!(v.as_f64(), Some(-2.5));
    }

    #[test]
    fn test_number_with_places() {
        let v = OdinValues::number_with_places(3.14159, 2);
        assert!(v.is_number());
        match &v {
            OdinValue::Number { decimal_places, .. } => assert_eq!(*decimal_places, Some(2)),
            _ => panic!("Expected number"),
        }
    }

    #[test]
    fn test_number_infinity() {
        let v = OdinValues::number(f64::INFINITY);
        assert_eq!(v.as_f64(), Some(f64::INFINITY));
    }

    #[test]
    fn test_number_nan() {
        let v = OdinValues::number(f64::NAN);
        assert!(v.as_f64().unwrap().is_nan());
    }

    #[test]
    fn test_currency_constructor() {
        let v = OdinValues::currency(99.99, 2);
        assert!(v.is_currency());
        assert_eq!(v.as_f64(), Some(99.99));
        assert_eq!(v.value_type(), OdinValueType::Currency);
    }

    #[test]
    fn test_currency_with_code() {
        let v = OdinValues::currency_with_code(100.0, 2, "USD");
        match &v {
            OdinValue::Currency { currency_code, .. } => {
                assert_eq!(currency_code.as_deref(), Some("USD"));
            }
            _ => panic!("Expected currency"),
        }
    }

    #[test]
    fn test_currency_four_decimal() {
        let v = OdinValues::currency(1234.5678, 4);
        assert_eq!(v.as_f64(), Some(1234.5678));
    }

    #[test]
    fn test_percent_constructor() {
        let v = OdinValues::percent(0.15);
        assert!(v.is_percent());
        assert_eq!(v.as_f64(), Some(0.15));
    }

    #[test]
    fn test_percent_over_one() {
        let v = OdinValues::percent(1.5);
        assert_eq!(v.as_f64(), Some(1.5));
    }

    #[test]
    fn test_date_constructor() {
        let v = OdinValues::date(2024, 6, 15);
        assert!(v.is_date());
        assert!(v.is_temporal());
        assert_eq!(v.value_type(), OdinValueType::Date);
    }

    #[test]
    fn test_date_from_str() {
        let v = OdinValues::date_from_str("2024-06-15").unwrap();
        assert!(v.is_date());
        match &v {
            OdinValue::Date { year, month, day, .. } => {
                assert_eq!(*year, 2024);
                assert_eq!(*month, 6);
                assert_eq!(*day, 15);
            }
            _ => panic!("Expected date"),
        }
    }

    #[test]
    fn test_date_from_str_invalid() {
        assert!(OdinValues::date_from_str("not-a-date").is_none());
        assert!(OdinValues::date_from_str("2024-06").is_none());
    }

    #[test]
    fn test_timestamp_constructor() {
        let v = OdinValues::timestamp(1718451000000, "2024-06-15T14:30:00Z");
        assert!(v.is_timestamp());
        assert!(v.is_temporal());
    }

    #[test]
    fn test_time_constructor() {
        let v = OdinValues::time("T14:30:00");
        assert!(v.is_temporal());
        assert_eq!(v.value_type(), OdinValueType::Time);
    }

    #[test]
    fn test_duration_constructor() {
        let v = OdinValues::duration("P1Y6M");
        assert!(v.is_temporal());
        assert_eq!(v.value_type(), OdinValueType::Duration);
    }

    #[test]
    fn test_reference_constructor() {
        let v = OdinValues::reference("policy.id");
        assert!(v.is_reference());
        assert_eq!(v.as_reference(), Some("policy.id"));
    }

    #[test]
    fn test_binary_constructor() {
        let v = OdinValues::binary(vec![1, 2, 3]);
        assert!(v.is_binary());
    }

    #[test]
    fn test_binary_with_algorithm() {
        let v = OdinValues::binary_with_algorithm(vec![0xDE, 0xAD], "sha256");
        match &v {
            OdinValue::Binary { algorithm, data, .. } => {
                assert_eq!(algorithm.as_deref(), Some("sha256"));
                assert_eq!(data, &[0xDE, 0xAD]);
            }
            _ => panic!("Expected binary"),
        }
    }

    #[test]
    fn test_binary_empty() {
        let v = OdinValues::binary(vec![]);
        assert!(v.is_binary());
    }

    #[test]
    fn test_verb_constructor() {
        let v = OdinValues::verb("upper", vec![OdinValues::string("hello")]);
        assert!(v.is_verb());
        assert_eq!(v.value_type(), OdinValueType::Verb);
    }

    #[test]
    fn test_custom_verb_constructor() {
        let v = OdinValues::custom_verb("ns.myverb", vec![]);
        match &v {
            OdinValue::Verb { is_custom, .. } => assert!(*is_custom),
            _ => panic!("Expected verb"),
        }
    }

    #[test]
    fn test_array_constructor() {
        let items = vec![
            OdinArrayItem::Value(OdinValues::integer(1)),
            OdinArrayItem::Value(OdinValues::integer(2)),
        ];
        let v = OdinValues::array(items);
        assert!(v.is_array());
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_array_empty() {
        let v = OdinValues::array(vec![]);
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_array_records() {
        let record = OdinArrayItem::Record(vec![
            ("name".to_string(), OdinValues::string("Alice")),
        ]);
        let v = OdinValues::array(vec![record]);
        assert_eq!(v.as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_object_constructor() {
        let v = OdinValues::object(vec![
            ("key".to_string(), OdinValues::string("value")),
        ]);
        assert!(v.is_object());
    }

    // ─── value_type() tests ─────────────────────────────────────────────

    #[test]
    fn test_value_type_timestamp() {
        assert_eq!(OdinValues::timestamp(0, "1970-01-01T00:00:00Z").value_type(), OdinValueType::Timestamp);
    }

    #[test]
    fn test_value_type_time() {
        assert_eq!(OdinValues::time("T12:00:00").value_type(), OdinValueType::Time);
    }

    #[test]
    fn test_value_type_duration() {
        assert_eq!(OdinValues::duration("PT1H").value_type(), OdinValueType::Duration);
    }

    #[test]
    fn test_value_type_verb() {
        assert_eq!(OdinValues::verb("test", vec![]).value_type(), OdinValueType::Verb);
    }

    #[test]
    fn test_value_type_array() {
        assert_eq!(OdinValues::array(vec![]).value_type(), OdinValueType::Array);
    }

    #[test]
    fn test_value_type_object() {
        assert_eq!(OdinValues::object(vec![]).value_type(), OdinValueType::Object);
    }

    // ─── as_f64() numeric coercion tests ─────────────────────────────────

    #[test]
    fn test_as_f64_currency() {
        assert_eq!(OdinValues::currency(49.99, 2).as_f64(), Some(49.99));
    }

    #[test]
    fn test_as_f64_percent() {
        assert_eq!(OdinValues::percent(0.25).as_f64(), Some(0.25));
    }

    #[test]
    fn test_as_f64_string_returns_none() {
        assert_eq!(OdinValues::string("hello").as_f64(), None);
    }

    #[test]
    fn test_as_f64_null_returns_none() {
        assert_eq!(OdinValues::null().as_f64(), None);
    }

    #[test]
    fn test_as_f64_boolean_returns_none() {
        assert_eq!(OdinValues::boolean(true).as_f64(), None);
    }

    // ─── is_*() predicate tests ──────────────────────────────────────────

    #[test]
    fn test_is_duration() {
        assert!(OdinValues::duration("P1D").is_temporal());
        assert!(!OdinValues::duration("P1D").is_numeric());
    }

    #[test]
    fn test_is_numeric_all_types() {
        assert!(OdinValues::integer(1).is_numeric());
        assert!(OdinValues::number(1.0).is_numeric());
        assert!(OdinValues::currency(1.0, 2).is_numeric());
        assert!(OdinValues::percent(0.5).is_numeric());
        assert!(!OdinValues::string("1").is_numeric());
        assert!(!OdinValues::boolean(true).is_numeric());
        assert!(!OdinValues::null().is_numeric());
    }

    #[test]
    fn test_is_temporal_all_types() {
        assert!(OdinValues::date(2024, 1, 1).is_temporal());
        assert!(OdinValues::timestamp(0, "x").is_temporal());
        assert!(OdinValues::time("T12:00").is_temporal());
        assert!(OdinValues::duration("P1D").is_temporal());
        assert!(!OdinValues::string("2024-01-01").is_temporal());
    }

    #[test]
    fn test_wrong_type_extraction_returns_none() {
        assert_eq!(OdinValues::integer(42).as_bool(), None);
        assert_eq!(OdinValues::integer(42).as_str(), None);
        assert_eq!(OdinValues::string("x").as_i64(), None);
        assert_eq!(OdinValues::string("x").as_reference(), None);
        assert_eq!(OdinValues::integer(42).as_array(), None);
        assert_eq!(OdinValues::integer(42).as_reference(), None);
    }

    // ─── Modifier tests ─────────────────────────────────────────────────

    #[test]
    fn test_modifiers_required_only() {
        let mods = OdinModifiers { required: true, ..Default::default() };
        assert!(!mods.is_empty());
        assert!(mods.has_any());
    }

    #[test]
    fn test_modifiers_confidential_only() {
        let mods = OdinModifiers { confidential: true, ..Default::default() };
        assert!(mods.has_any());
    }

    #[test]
    fn test_modifiers_deprecated_only() {
        let mods = OdinModifiers { deprecated: true, ..Default::default() };
        assert!(mods.has_any());
    }

    #[test]
    fn test_modifiers_attr_only() {
        let mods = OdinModifiers { attr: true, ..Default::default() };
        assert!(mods.has_any());
    }

    #[test]
    fn test_modifiers_all_set() {
        let mods = OdinModifiers {
            required: true,
            confidential: true,
            deprecated: true,
            attr: true,
        };
        let v = OdinValues::string("x").with_modifiers(mods);
        assert!(v.is_required());
        assert!(v.is_confidential());
        assert!(v.is_deprecated());
    }

    #[test]
    fn test_with_empty_modifiers_clears() {
        let mods = OdinModifiers { required: true, ..Default::default() };
        let v = OdinValues::string("x").with_modifiers(mods);
        assert!(v.is_required());
        // Now clear by passing empty modifiers
        let v2 = v.with_modifiers(OdinModifiers::default());
        assert!(!v2.is_required());
        assert!(v2.modifiers().is_none());
    }

    #[test]
    fn test_modifiers_on_null() {
        let mods = OdinModifiers { required: true, ..Default::default() };
        let v = OdinValues::null().with_modifiers(mods);
        assert!(v.is_required());
    }

    #[test]
    fn test_modifiers_on_integer() {
        let mods = OdinModifiers { confidential: true, ..Default::default() };
        let v = OdinValues::integer(42).with_modifiers(mods);
        assert!(v.is_confidential());
    }

    // ─── Directive tests ─────────────────────────────────────────────────

    #[test]
    fn test_with_directives() {
        let d = OdinDirective {
            name: "pos".to_string(),
            value: Some(DirectiveValue::Number(3.0)),
        };
        let v = OdinValues::string("test").with_directives(vec![d]);
        assert_eq!(v.directives().len(), 1);
        assert_eq!(v.directives()[0].name, "pos");
    }

    #[test]
    fn test_directive_string_value() {
        let d = OdinDirective {
            name: "format".to_string(),
            value: Some(DirectiveValue::String("ssn".to_string())),
        };
        let v = OdinValues::string("x").with_directives(vec![d]);
        match &v.directives()[0].value {
            Some(DirectiveValue::String(s)) => assert_eq!(s, "ssn"),
            _ => panic!("Expected string directive"),
        }
    }

    #[test]
    fn test_directive_no_value() {
        let d = OdinDirective {
            name: "trim".to_string(),
            value: None,
        };
        let v = OdinValues::string("x").with_directives(vec![d]);
        assert!(v.directives()[0].value.is_none());
    }

    #[test]
    fn test_multiple_directives() {
        let directives = vec![
            OdinDirective { name: "pos".to_string(), value: Some(DirectiveValue::Number(3.0)) },
            OdinDirective { name: "len".to_string(), value: Some(DirectiveValue::Number(8.0)) },
            OdinDirective { name: "trim".to_string(), value: None },
        ];
        let v = OdinValues::reference("_line").with_directives(directives);
        assert_eq!(v.directives().len(), 3);
    }

    // ─── Display formatting tests ────────────────────────────────────────

    #[test]
    fn test_display_boolean_false() {
        assert_eq!(format!("{}", OdinValues::boolean(false)), "false");
    }

    #[test]
    fn test_display_currency() {
        let display = format!("{}", OdinValues::currency(100.0, 2));
        assert!(display.starts_with("#$"));
    }

    #[test]
    fn test_display_currency_with_code() {
        let display = format!("{}", OdinValues::currency_with_code(100.0, 2, "USD"));
        assert!(display.contains(":USD"));
    }

    #[test]
    fn test_display_percent() {
        let display = format!("{}", OdinValues::percent(0.15));
        assert!(display.starts_with("#%"));
    }

    #[test]
    fn test_display_date() {
        assert_eq!(format!("{}", OdinValues::date(2024, 6, 15)), "2024-06-15");
    }

    #[test]
    fn test_display_timestamp() {
        let v = OdinValues::timestamp(0, "2024-06-15T14:30:00Z");
        assert_eq!(format!("{v}"), "2024-06-15T14:30:00Z");
    }

    #[test]
    fn test_display_time() {
        assert_eq!(format!("{}", OdinValues::time("T14:30:00")), "T14:30:00");
    }

    #[test]
    fn test_display_duration() {
        assert_eq!(format!("{}", OdinValues::duration("P1Y6M")), "P1Y6M");
    }

    #[test]
    fn test_display_binary_no_algorithm() {
        let display = format!("{}", OdinValues::binary(vec![1, 2]));
        assert_eq!(display, "^<data>");
    }

    #[test]
    fn test_display_binary_with_algorithm() {
        let display = format!("{}", OdinValues::binary_with_algorithm(vec![1], "sha256"));
        assert_eq!(display, "^sha256:<data>");
    }

    #[test]
    fn test_display_verb() {
        let v = OdinValues::verb("upper", vec![OdinValues::reference("name")]);
        assert_eq!(format!("{v}"), "%upper @name");
    }

    #[test]
    fn test_display_array() {
        let v = OdinValues::array(vec![
            OdinArrayItem::Value(OdinValues::integer(1)),
            OdinArrayItem::Value(OdinValues::integer(2)),
        ]);
        assert_eq!(format!("{v}"), "[2 items]");
    }

    #[test]
    fn test_display_object() {
        let v = OdinValues::object(vec![
            ("a".to_string(), OdinValues::integer(1)),
        ]);
        assert_eq!(format!("{v}"), "{1 fields}");
    }

    #[test]
    fn test_display_integer_with_raw() {
        let v = OdinValue::Integer {
            value: 0,
            raw: Some("99999999999999999999".to_string()),
            modifiers: None,
            directives: vec![],
        };
        assert_eq!(format!("{v}"), "##99999999999999999999");
    }

    #[test]
    fn test_display_number_with_raw() {
        let v = OdinValue::Number {
            value: 3.14,
            decimal_places: None,
            raw: Some("3.14159265358979323846".to_string()),
            modifiers: None,
            directives: vec![],
        };
        assert_eq!(format!("{v}"), "#3.14159265358979323846");
    }

    // ─── OdinValueType Display tests ─────────────────────────────────────

    #[test]
    fn test_value_type_display() {
        assert_eq!(format!("{}", OdinValueType::Null), "null");
        assert_eq!(format!("{}", OdinValueType::Boolean), "boolean");
        assert_eq!(format!("{}", OdinValueType::String), "string");
        assert_eq!(format!("{}", OdinValueType::Integer), "integer");
        assert_eq!(format!("{}", OdinValueType::Number), "number");
        assert_eq!(format!("{}", OdinValueType::Currency), "currency");
        assert_eq!(format!("{}", OdinValueType::Percent), "percent");
        assert_eq!(format!("{}", OdinValueType::Date), "date");
        assert_eq!(format!("{}", OdinValueType::Timestamp), "timestamp");
        assert_eq!(format!("{}", OdinValueType::Time), "time");
        assert_eq!(format!("{}", OdinValueType::Duration), "duration");
        assert_eq!(format!("{}", OdinValueType::Reference), "reference");
        assert_eq!(format!("{}", OdinValueType::Binary), "binary");
        assert_eq!(format!("{}", OdinValueType::Verb), "verb");
        assert_eq!(format!("{}", OdinValueType::Array), "array");
        assert_eq!(format!("{}", OdinValueType::Object), "object");
    }

    // ─── Clone / Equality tests ──────────────────────────────────────────

    #[test]
    fn test_clone_preserves_value() {
        let v = OdinValues::string("hello");
        let cloned = v.clone();
        assert_eq!(v, cloned);
    }

    #[test]
    fn test_clone_preserves_modifiers() {
        let mods = OdinModifiers { required: true, confidential: true, ..Default::default() };
        let v = OdinValues::integer(42).with_modifiers(mods);
        let cloned = v.clone();
        assert_eq!(v, cloned);
        assert!(cloned.is_required());
        assert!(cloned.is_confidential());
    }

    #[test]
    fn test_equality_same_value() {
        assert_eq!(OdinValues::integer(42), OdinValues::integer(42));
        assert_eq!(OdinValues::string("x"), OdinValues::string("x"));
        assert_eq!(OdinValues::null(), OdinValues::null());
        assert_eq!(OdinValues::boolean(true), OdinValues::boolean(true));
    }

    #[test]
    fn test_inequality_different_values() {
        assert_ne!(OdinValues::integer(42), OdinValues::integer(43));
        assert_ne!(OdinValues::string("a"), OdinValues::string("b"));
        assert_ne!(OdinValues::boolean(true), OdinValues::boolean(false));
    }

    #[test]
    fn test_inequality_different_types() {
        assert_ne!(OdinValues::integer(42), OdinValues::number(42.0));
        assert_ne!(OdinValues::string("true"), OdinValues::boolean(true));
    }
}
