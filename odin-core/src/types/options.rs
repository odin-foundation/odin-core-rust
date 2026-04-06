//! Configuration options for ODIN operations.

/// Options for parsing ODIN text.
#[derive(Debug, Clone)]
pub struct ParseOptions {
    /// Maximum nesting depth (default: 64).
    pub max_depth: usize,
    /// Maximum document size in bytes (default: 10MB).
    pub max_size: usize,
    /// Whether to allow duplicate path assignments (default: false).
    pub allow_duplicates: bool,
    /// Whether to allow empty documents (default: false).
    pub allow_empty: bool,
    /// Continue parsing on error, collecting all errors (default: false).
    /// When true, the parser will attempt to recover from errors and return
    /// a partial document along with the collected errors.
    pub continue_on_error: bool,
    /// Preserve comments in the parsed document (default: false).
    /// When true, comments are stored in `OdinDocument.comments` for round-tripping.
    pub preserve_comments: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_size: 10 * 1024 * 1024, // 10MB
            allow_duplicates: false,
            allow_empty: false,
            continue_on_error: false,
            preserve_comments: false,
        }
    }
}

/// Options for serializing ODIN documents to text.
#[derive(Debug, Clone)]
pub struct StringifyOptions {
    /// Use pretty-printing with indentation (default: true).
    pub pretty: bool,
    /// Indent string (default: "  " — two spaces).
    pub indent: String,
    /// Include metadata section in output (default: true).
    pub include_metadata: bool,
    /// Sort assignments alphabetically (default: false — preserve insertion order).
    pub sort_keys: bool,
    /// Use tabular format for eligible arrays (default: false).
    /// Arrays where all items have the same keys are rendered as aligned column tables.
    pub use_tabular: bool,
    /// Canonical mode: metadata as $.key, strip trailing zeros from numbers,
    /// enforce min 2 decimal places on currency, numeric array index sorting.
    pub canonical: bool,
}

impl Default for StringifyOptions {
    fn default() -> Self {
        Self {
            pretty: true,
            indent: "  ".to_string(),
            include_metadata: true,
            sort_keys: false,
            use_tabular: false,
            canonical: false,
        }
    }
}

/// Options for schema validation.
#[derive(Debug, Clone, Default)]
pub struct ValidateOptions {
    /// Strict mode: reject unknown fields (default: false).
    pub strict: bool,
    /// Collect all errors instead of stopping at first (default: true).
    pub collect_all: bool,
    /// Stop validation on first error (default: false).
    pub fail_fast: bool,
    /// Include W001/W002 warnings in the result (default: false).
    pub include_warnings: bool,
}
