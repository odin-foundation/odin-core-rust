//! ODIN Schema types for document validation.

use std::collections::HashMap;

/// A parsed ODIN schema.
#[derive(Debug, Clone)]
pub struct OdinSchemaDefinition {
    /// Schema metadata.
    pub metadata: SchemaMetadata,
    /// Import directives.
    pub imports: Vec<SchemaImport>,
    /// Named type definitions.
    pub types: HashMap<String, SchemaType>,
    /// Top-level field definitions.
    pub fields: HashMap<String, SchemaField>,
    /// Array definitions.
    pub arrays: HashMap<String, SchemaArray>,
    /// Object-level constraints.
    pub constraints: HashMap<String, Vec<SchemaObjectConstraint>>,
}

/// Schema metadata from the `{$}` header.
#[derive(Debug, Clone, Default)]
pub struct SchemaMetadata {
    /// Schema identifier.
    pub id: Option<String>,
    /// Human-readable title.
    pub title: Option<String>,
    /// Schema description.
    pub description: Option<String>,
    /// Schema version.
    pub version: Option<String>,
}

/// An import in a schema file.
#[derive(Debug, Clone)]
pub struct SchemaImport {
    /// Import file path.
    pub path: String,
    /// Optional alias for the imported schema.
    pub alias: Option<String>,
}

/// A named type definition in a schema.
#[derive(Debug, Clone)]
pub struct SchemaType {
    /// Type name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// Fields defined in this type.
    pub fields: Vec<SchemaField>,
    /// Parent types for composition (from `@Child : @Parent & @Other`).
    pub parents: Vec<String>,
}

/// A field definition in a schema.
#[derive(Debug, Clone)]
pub struct SchemaField {
    /// Field name.
    pub name: String,
    /// Field type definition.
    pub field_type: SchemaFieldType,
    /// Whether this field is required.
    pub required: bool,
    /// Whether this field is confidential.
    pub confidential: bool,
    /// Whether this field is deprecated.
    pub deprecated: bool,
    /// Whether this field is immutable — once a value is set, it cannot
    /// be changed or deleted. The SDK records this flag; enforcement is
    /// the responsibility of the storage layer (e.g. andvari-engine
    /// rejects writes that would mutate or delete a prior value).
    pub immutable: bool,
    /// Optional description.
    pub description: Option<String>,
    /// Validation constraints.
    pub constraints: Vec<SchemaConstraint>,
    /// Default value if not provided.
    pub default_value: Option<String>,
    /// Conditional requirements (`:if field op value` / `:unless field op value`).
    pub conditionals: Vec<SchemaConditional>,
}

/// A conditional requirement on a field.
#[derive(Debug, Clone)]
pub struct SchemaConditional {
    /// The field path to evaluate the condition against.
    pub field: String,
    /// The comparison operator.
    pub operator: ConditionalOperator,
    /// The expected value.
    pub value: ConditionalValue,
    /// If true, this is an `:unless` condition (negated).
    pub unless: bool,
}

/// A conditional comparison operator.
#[derive(Debug, Clone, PartialEq)]
pub enum ConditionalOperator {
    /// Equal (`=`).
    Eq,
    /// Not equal (`!=`).
    NotEq,
    /// Greater than (`>`).
    Gt,
    /// Less than (`<`).
    Lt,
    /// Greater than or equal (`>=`).
    Gte,
    /// Less than or equal (`<=`).
    Lte,
}

/// A conditional value.
#[derive(Debug, Clone)]
pub enum ConditionalValue {
    /// String literal value.
    String(String),
    /// Numeric value.
    Number(f64),
    /// Boolean value.
    Bool(bool),
}

/// The type of a schema field.
#[derive(Debug, Clone)]
pub enum SchemaFieldType {
    /// String type.
    String,
    /// Boolean type.
    Boolean,
    /// Null type.
    Null,
    /// Number type with optional decimal precision.
    Number {
        /// Fixed decimal places.
        decimal_places: Option<u8>,
    },
    /// Integer type.
    Integer,
    /// Decimal type with optional precision.
    Decimal {
        /// Fixed decimal places.
        decimal_places: Option<u8>,
    },
    /// Currency type with optional precision.
    Currency {
        /// Fixed decimal places.
        decimal_places: Option<u8>,
    },
    /// Date type.
    Date,
    /// Timestamp type.
    Timestamp,
    /// Time type.
    Time,
    /// Duration type.
    Duration,
    /// Percent type.
    Percent,
    /// Enumeration of allowed string values.
    Enum(Vec<String>),
    /// Union of multiple possible types.
    Union(Vec<SchemaFieldType>),
    /// Reference to another path.
    Reference(String),
    /// Binary data type.
    Binary,
    /// Reference to a named type definition.
    TypeRef(String),
}

/// A constraint on a field.
#[derive(Debug, Clone)]
pub enum SchemaConstraint {
    /// Numeric or date bounds (min/max).
    Bounds {
        /// Minimum bound value.
        min: Option<String>,
        /// Maximum bound value.
        max: Option<String>,
        /// Whether the minimum bound is exclusive.
        min_exclusive: bool,
        /// Whether the maximum bound is exclusive.
        max_exclusive: bool,
    },
    /// Pattern (regex) constraint.
    Pattern(String),
    /// Enum constraint (allowed values).
    Enum(Vec<String>),
    /// Unique constraint within arrays.
    Unique,
    /// Size constraint for binary data.
    Size {
        /// Minimum size in bytes.
        min: Option<u64>,
        /// Maximum size in bytes.
        max: Option<u64>,
    },
    /// Format constraint (email, url, uuid, ssn, etc.).
    Format(String),
}

/// An array definition in a schema.
#[derive(Debug, Clone)]
pub struct SchemaArray {
    /// Array name.
    pub name: String,
    /// Type of each array item.
    pub item_type: SchemaFieldType,
    /// Minimum number of items.
    pub min_items: Option<usize>,
    /// Maximum number of items.
    pub max_items: Option<usize>,
    /// Whether items must be unique.
    pub unique: bool,
}

/// An object-level constraint.
#[derive(Debug, Clone)]
pub enum SchemaObjectConstraint {
    /// Invariant expression.
    Invariant(String),
    /// Cardinality constraint (exactly N, at most N, etc.).
    Cardinality {
        /// Fields in the cardinality group.
        fields: Vec<String>,
        /// Minimum number of fields that must be present.
        min: Option<usize>,
        /// Maximum number of fields that may be present.
        max: Option<usize>,
    },
}

/// Result of validating a document against a schema.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the document is valid.
    pub valid: bool,
    /// Validation errors found.
    pub errors: Vec<crate::types::errors::ValidationError>,
    /// Validation warnings (non-blocking).
    pub warnings: Vec<ValidationWarning>,
}

/// A non-blocking validation warning.
#[derive(Debug, Clone)]
pub struct ValidationWarning {
    /// Document path where the warning occurred.
    pub path: String,
    /// Warning message.
    pub message: String,
}

impl ValidationResult {
    /// Create a valid result with no errors.
    pub fn valid() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Create an invalid result with the given errors.
    pub fn invalid(errors: Vec<crate::types::errors::ValidationError>) -> Self {
        Self {
            valid: false,
            errors,
            warnings: Vec::new(),
        }
    }
}
