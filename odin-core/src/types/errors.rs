//! Error types for ODIN operations.
//!
//! Error codes are part of the ODIN API contract and must be identical
//! across all language implementations:
//! - Parse errors: P001-P015
//! - Validation errors: V001-V013

use std::fmt;

// ─────────────────────────────────────────────────────────────────────────────
// Error Codes
// ─────────────────────────────────────────────────────────────────────────────

/// Parse error codes (P001-P015).
///
/// These are API contract — identical codes and messages across all SDKs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParseErrorCode {
    /// P001: Unexpected character encountered during parsing.
    UnexpectedCharacter,
    /// P002: Bare (unquoted) string values are not allowed.
    BareStringNotAllowed,
    /// P003: Invalid array index (non-numeric, negative, or too large).
    InvalidArrayIndex,
    /// P004: String literal was not properly terminated with a closing quote.
    UnterminatedString,
    /// P005: Invalid escape sequence in a string literal.
    InvalidEscapeSequence,
    /// P006: Invalid type prefix (e.g., `#x` where `x` is not a valid modifier).
    InvalidTypePrefix,
    /// P007: Duplicate assignment to the same path.
    DuplicatePathAssignment,
    /// P008: Invalid header syntax (e.g., malformed `{section}` or `{$}`).
    InvalidHeaderSyntax,
    /// P009: Invalid directive syntax.
    InvalidDirective,
    /// P010: Maximum nesting depth exceeded.
    MaximumDepthExceeded,
    /// P011: Maximum document size exceeded.
    MaximumDocumentSizeExceeded,
    /// P012: Invalid UTF-8 byte sequence.
    InvalidUtf8Sequence,
    /// P013: Non-contiguous array indices (gaps in array indexing).
    NonContiguousArrayIndices,
    /// P014: Empty document (no assignments).
    EmptyDocument,
    /// P015: Array index out of allowed range.
    ArrayIndexOutOfRange,
}

impl ParseErrorCode {
    /// Returns the string code (e.g., "P001").
    pub fn code(self) -> &'static str {
        match self {
            Self::UnexpectedCharacter => "P001",
            Self::BareStringNotAllowed => "P002",
            Self::InvalidArrayIndex => "P003",
            Self::UnterminatedString => "P004",
            Self::InvalidEscapeSequence => "P005",
            Self::InvalidTypePrefix => "P006",
            Self::DuplicatePathAssignment => "P007",
            Self::InvalidHeaderSyntax => "P008",
            Self::InvalidDirective => "P009",
            Self::MaximumDepthExceeded => "P010",
            Self::MaximumDocumentSizeExceeded => "P011",
            Self::InvalidUtf8Sequence => "P012",
            Self::NonContiguousArrayIndices => "P013",
            Self::EmptyDocument => "P014",
            Self::ArrayIndexOutOfRange => "P015",
        }
    }

    /// Returns the default message for this error code.
    pub fn message(self) -> &'static str {
        match self {
            Self::UnexpectedCharacter => "Unexpected character",
            Self::BareStringNotAllowed => "Strings must be quoted",
            Self::InvalidArrayIndex => "Invalid array index",
            Self::UnterminatedString => "Unterminated string",
            Self::InvalidEscapeSequence => "Invalid escape sequence",
            Self::InvalidTypePrefix => "Invalid type prefix",
            Self::DuplicatePathAssignment => "Duplicate path assignment",
            Self::InvalidHeaderSyntax => "Invalid header syntax",
            Self::InvalidDirective => "Invalid directive",
            Self::MaximumDepthExceeded => "Maximum depth exceeded",
            Self::MaximumDocumentSizeExceeded => "Maximum document size exceeded",
            Self::InvalidUtf8Sequence => "Invalid UTF-8 sequence",
            Self::NonContiguousArrayIndices => "Non-contiguous array indices",
            Self::EmptyDocument => "Empty document",
            Self::ArrayIndexOutOfRange => "Array index out of range",
        }
    }

    /// Parse from a code string like "P001".
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "P001" => Some(Self::UnexpectedCharacter),
            "P002" => Some(Self::BareStringNotAllowed),
            "P003" => Some(Self::InvalidArrayIndex),
            "P004" => Some(Self::UnterminatedString),
            "P005" => Some(Self::InvalidEscapeSequence),
            "P006" => Some(Self::InvalidTypePrefix),
            "P007" => Some(Self::DuplicatePathAssignment),
            "P008" => Some(Self::InvalidHeaderSyntax),
            "P009" => Some(Self::InvalidDirective),
            "P010" => Some(Self::MaximumDepthExceeded),
            "P011" => Some(Self::MaximumDocumentSizeExceeded),
            "P012" => Some(Self::InvalidUtf8Sequence),
            "P013" => Some(Self::NonContiguousArrayIndices),
            "P014" => Some(Self::EmptyDocument),
            "P015" => Some(Self::ArrayIndexOutOfRange),
            _ => None,
        }
    }
}

impl fmt::Display for ParseErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

/// Validation error codes (V001-V013).
///
/// These are API contract — identical codes and messages across all SDKs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValidationErrorCode {
    /// V001: Required field is missing.
    RequiredFieldMissing,
    /// V002: Value type does not match the schema.
    TypeMismatch,
    /// V003: Value is outside allowed bounds.
    ValueOutOfBounds,
    /// V004: Value does not match the required pattern.
    PatternMismatch,
    /// V005: Value is not one of the allowed enum values.
    InvalidEnumValue,
    /// V006: Array length violates min/max constraints.
    ArrayLengthViolation,
    /// V007: Unique constraint was violated (duplicate value in array).
    UniqueConstraintViolation,
    /// V008: Schema invariant was violated.
    InvariantViolation,
    /// V009: Cardinality constraint violated.
    CardinalityConstraintViolation,
    /// V010: Conditional requirement not met.
    ConditionalRequirementNotMet,
    /// V011: Unknown field in strict mode.
    UnknownField,
    /// V012: Circular reference detected.
    CircularReference,
    /// V013: Unresolved reference.
    UnresolvedReference,
}

impl ValidationErrorCode {
    /// Returns the string code (e.g., "V001").
    pub fn code(self) -> &'static str {
        match self {
            Self::RequiredFieldMissing => "V001",
            Self::TypeMismatch => "V002",
            Self::ValueOutOfBounds => "V003",
            Self::PatternMismatch => "V004",
            Self::InvalidEnumValue => "V005",
            Self::ArrayLengthViolation => "V006",
            Self::UniqueConstraintViolation => "V007",
            Self::InvariantViolation => "V008",
            Self::CardinalityConstraintViolation => "V009",
            Self::ConditionalRequirementNotMet => "V010",
            Self::UnknownField => "V011",
            Self::CircularReference => "V012",
            Self::UnresolvedReference => "V013",
        }
    }

    /// Returns the default message for this error code.
    pub fn message(self) -> &'static str {
        match self {
            Self::RequiredFieldMissing => "Required field missing",
            Self::TypeMismatch => "Type mismatch",
            Self::ValueOutOfBounds => "Value out of bounds",
            Self::PatternMismatch => "Pattern mismatch",
            Self::InvalidEnumValue => "Invalid enum value",
            Self::ArrayLengthViolation => "Array length violation",
            Self::UniqueConstraintViolation => "Unique constraint violation",
            Self::InvariantViolation => "Invariant violation",
            Self::CardinalityConstraintViolation => "Cardinality constraint violation",
            Self::ConditionalRequirementNotMet => "Conditional requirement not met",
            Self::UnknownField => "Unknown field",
            Self::CircularReference => "Circular reference",
            Self::UnresolvedReference => "Unresolved reference",
        }
    }

    /// Parse from a code string like "V001".
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "V001" => Some(Self::RequiredFieldMissing),
            "V002" => Some(Self::TypeMismatch),
            "V003" => Some(Self::ValueOutOfBounds),
            "V004" => Some(Self::PatternMismatch),
            "V005" => Some(Self::InvalidEnumValue),
            "V006" => Some(Self::ArrayLengthViolation),
            "V007" => Some(Self::UniqueConstraintViolation),
            "V008" => Some(Self::InvariantViolation),
            "V009" => Some(Self::CardinalityConstraintViolation),
            "V010" => Some(Self::ConditionalRequirementNotMet),
            "V011" => Some(Self::UnknownField),
            "V012" => Some(Self::CircularReference),
            "V013" => Some(Self::UnresolvedReference),
            _ => None,
        }
    }
}

impl fmt::Display for ValidationErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Error Types
// ─────────────────────────────────────────────────────────────────────────────

/// Broad categories of ODIN errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OdinErrorKind {
    /// Error during parsing.
    Parse,
    /// Error during validation.
    Validation,
    /// Error during patching.
    Patch,
    /// Error during transformation.
    Transform,
    /// Other error.
    Other,
}

/// Base error type for all ODIN operations.
#[derive(Debug, Clone)]
pub struct OdinError {
    /// Human-readable error message.
    pub message: String,
    /// Error code (e.g., "P001", "V003").
    pub code: String,
    /// Error category.
    pub kind: OdinErrorKind,
}

impl fmt::Display for OdinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for OdinError {}

/// Error during ODIN text parsing.
#[derive(Debug, Clone)]
pub struct ParseError {
    /// Human-readable error message.
    pub message: String,
    /// Parse error code.
    pub error_code: ParseErrorCode,
    /// Line number (1-based) where the error occurred.
    pub line: usize,
    /// Column number (1-based) where the error occurred.
    pub column: usize,
}

impl ParseError {
    /// Create a new parse error.
    pub fn new(error_code: ParseErrorCode, line: usize, column: usize) -> Self {
        let message = format!(
            "{} at line {}, column {}",
            error_code.message(),
            line,
            column
        );
        Self {
            message,
            error_code,
            line,
            column,
        }
    }

    /// Create a parse error with a custom message.
    pub fn with_message(
        error_code: ParseErrorCode,
        line: usize,
        column: usize,
        detail: &str,
    ) -> Self {
        let message = format!(
            "{}: {} at line {}, column {}",
            error_code.message(),
            detail,
            line,
            column
        );
        Self {
            message,
            error_code,
            line,
            column,
        }
    }

    /// Returns the string error code (e.g., "P001").
    pub fn code(&self) -> &str {
        self.error_code.code()
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.error_code.code(), self.message)
    }
}

impl std::error::Error for ParseError {}

/// Error during document patching.
#[derive(Debug, Clone)]
pub struct PatchError {
    /// Human-readable error message.
    pub message: String,
    /// Path where the error occurred.
    pub path: String,
}

impl PatchError {
    /// Create a new patch error.
    pub fn new(message: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            path: path.into(),
        }
    }
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Patch error at '{}': {}", self.path, self.message)
    }
}

impl std::error::Error for PatchError {}

/// A single validation error with path and details.
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Path to the field with the error.
    pub path: String,
    /// Validation error code.
    pub error_code: ValidationErrorCode,
    /// Human-readable error message.
    pub message: String,
    /// Expected value or type (for diagnostics).
    pub expected: Option<String>,
    /// Actual value or type found (for diagnostics).
    pub actual: Option<String>,
    /// Path in the schema that triggered this error.
    pub schema_path: Option<String>,
}

impl ValidationError {
    /// Create a new validation error.
    pub fn new(
        error_code: ValidationErrorCode,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            error_code,
            message: message.into(),
            expected: None,
            actual: None,
            schema_path: None,
        }
    }

    /// Returns the string error code (e.g., "V001").
    pub fn code(&self) -> &str {
        self.error_code.code()
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} at '{}'",
            self.error_code.code(),
            self.message,
            self.path
        )
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_error_codes() {
        assert_eq!(ParseErrorCode::UnexpectedCharacter.code(), "P001");
        assert_eq!(ParseErrorCode::ArrayIndexOutOfRange.code(), "P015");
        assert_eq!(
            ParseErrorCode::from_code("P007"),
            Some(ParseErrorCode::DuplicatePathAssignment)
        );
        assert_eq!(ParseErrorCode::from_code("P999"), None);
    }

    #[test]
    fn test_validation_error_codes() {
        assert_eq!(ValidationErrorCode::RequiredFieldMissing.code(), "V001");
        assert_eq!(ValidationErrorCode::UnresolvedReference.code(), "V013");
        assert_eq!(
            ValidationErrorCode::from_code("V005"),
            Some(ValidationErrorCode::InvalidEnumValue)
        );
    }

    #[test]
    fn test_parse_error_message_format() {
        let err = ParseError::new(ParseErrorCode::UnterminatedString, 5, 10);
        assert_eq!(err.code(), "P004");
        assert!(err.message.contains("line 5"));
        assert!(err.message.contains("column 10"));
    }

    #[test]
    fn test_validation_error() {
        let err = ValidationError::new(
            ValidationErrorCode::RequiredFieldMissing,
            "policy.number",
            "Field is required but was not provided",
        );
        assert_eq!(err.code(), "V001");
        assert_eq!(err.path, "policy.number");
    }
}
