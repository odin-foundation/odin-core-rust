//! Import resolver for ODIN documents and schemas.
//!
//! Resolves `@import` directives by loading referenced files, detecting
//! circular dependencies, and building a merged type registry.
//!
//! # Example
//!
//! ```rust,no_run
//! use odin_core::resolver::{ImportResolver, ResolverOptions, FileReader};
//!
//! struct DiskReader;
//! impl FileReader for DiskReader {
//!     fn read_file(&self, path: &str) -> Result<String, String> {
//!         std::fs::read_to_string(path).map_err(|e| e.to_string())
//!     }
//!     fn resolve_path(&self, base: &str, import: &str) -> Result<String, String> {
//!         let base_dir = std::path::Path::new(base)
//!             .parent()
//!             .unwrap_or(std::path::Path::new("."));
//!         let resolved = base_dir.join(import);
//!         resolved.canonicalize()
//!             .map(|p| p.to_string_lossy().to_string())
//!             .map_err(|e| e.to_string())
//!     }
//! }
//!
//! let resolver = ImportResolver::new(Box::new(DiskReader), ResolverOptions::default());
//! ```

pub mod schema_flattener;

use std::collections::{HashMap, HashSet};
use crate::types::document::OdinDocument;
use crate::types::schema::{OdinSchemaDefinition, SchemaType};
use crate::types::errors::ParseError;

// ─────────────────────────────────────────────────────────────────────────────
// File Reader Trait
// ─────────────────────────────────────────────────────────────────────────────

/// Trait for loading files from the filesystem or other sources.
///
/// Implementations control how import paths are resolved and how file
/// content is read. This enables testing with virtual filesystems and
/// sandboxed file access.
pub trait FileReader: Send + Sync {
    /// Read the content of a file at the given absolute path.
    fn read_file(&self, path: &str) -> Result<String, String>;

    /// Resolve an import path relative to a base file path.
    ///
    /// Returns the absolute/canonical path to the imported file.
    fn resolve_path(&self, base_path: &str, import_path: &str) -> Result<String, String>;
}

// ─────────────────────────────────────────────────────────────────────────────
// Options
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for the import resolver.
#[derive(Debug, Clone)]
pub struct ResolverOptions {
    /// Maximum import nesting depth (default: 32).
    pub max_import_depth: usize,
    /// Maximum total number of imports (default: 100).
    pub max_total_imports: usize,
    /// Whether to resolve imports in schema mode (types) or document mode.
    pub schema_mode: bool,
    /// Allowed file extensions (default: `[".odin"]`).
    pub allowed_extensions: Vec<String>,
    /// Maximum file size in bytes (default: 10MB).
    pub max_file_size: usize,
}

impl Default for ResolverOptions {
    fn default() -> Self {
        Self {
            max_import_depth: 32,
            max_total_imports: 100,
            schema_mode: true,
            allowed_extensions: vec![".odin".to_string()],
            max_file_size: 10 * 1024 * 1024,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Type Registry
// ─────────────────────────────────────────────────────────────────────────────

/// Registry of types collected from imported schemas.
#[derive(Debug, Clone, Default)]
pub struct TypeRegistry {
    /// Types without namespace (local types).
    pub local_types: HashMap<String, SchemaType>,
    /// Types organized by namespace (import alias).
    pub namespaces: HashMap<String, HashMap<String, SchemaType>>,
}

impl TypeRegistry {
    /// Create an empty type registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register all types from a schema under an optional namespace.
    pub fn register_all(
        &mut self,
        types: &HashMap<String, SchemaType>,
        namespace: Option<&str>,
    ) {
        match namespace {
            Some(ns) => {
                let ns_map = self.namespaces.entry(ns.to_string()).or_default();
                for (name, schema_type) in types {
                    ns_map.insert(name.clone(), schema_type.clone());
                }
            }
            None => {
                for (name, schema_type) in types {
                    self.local_types.insert(name.clone(), schema_type.clone());
                }
            }
        }
    }

    /// Look up a type by name, searching local types first then namespaces.
    pub fn lookup(&self, name: &str) -> Option<&SchemaType> {
        // Try local first
        if let Some(t) = self.local_types.get(name) {
            return Some(t);
        }
        // Try namespaced: "namespace.TypeName"
        if let Some(dot_pos) = name.find('.') {
            let ns = &name[..dot_pos];
            let type_name = &name[dot_pos + 1..];
            if let Some(ns_map) = self.namespaces.get(ns) {
                return ns_map.get(type_name);
            }
        }
        // Search all namespaces for unqualified name
        for ns_map in self.namespaces.values() {
            if let Some(t) = ns_map.get(name) {
                return Some(t);
            }
        }
        None
    }

    /// Returns all type names (including namespaced).
    pub fn all_type_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.local_types.keys().cloned().collect();
        for (ns, ns_map) in &self.namespaces {
            for name in ns_map.keys() {
                names.push(format!("{ns}.{name}"));
            }
        }
        names
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Circular Detector
// ─────────────────────────────────────────────────────────────────────────────

/// Stack-based circular import detector.
#[derive(Debug, Default)]
struct CircularDetector {
    chain: Vec<String>,
    chain_set: HashSet<String>,
}

impl CircularDetector {
    fn new() -> Self {
        Self::default()
    }

    /// Enter a path into the chain. Returns Err if circular.
    fn enter(&mut self, path: &str) -> Result<(), String> {
        let normalized = normalize_path(path);
        if self.chain_set.contains(&normalized) {
            let cycle = self.format_cycle(&normalized);
            return Err(format!("Circular import detected: {cycle}"));
        }
        self.chain_push(normalized);
        Ok(())
    }

    /// Remove the top of the chain.
    fn exit(&mut self) {
        if let Some(path) = self.chain.pop() {
            self.chain_set.remove(&path);
        }
    }

    /// Check if a path would create a cycle.
    pub fn is_circular(&self, path: &str) -> bool {
        self.chain_set.contains(&normalize_path(path))
    }

    fn chain_push(&mut self, path: String) {
        self.chain_set.insert(path.clone());
        self.chain.push(path);
    }

    fn format_cycle(&self, path: &str) -> String {
        let mut cycle = Vec::new();
        let mut found = false;
        for p in &self.chain {
            if p == path {
                found = true;
            }
            if found {
                cycle.push(p.as_str());
            }
        }
        cycle.push(path);
        cycle.join(" -> ")
    }
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

// ─────────────────────────────────────────────────────────────────────────────
// Resolved Import Result
// ─────────────────────────────────────────────────────────────────────────────

/// Result of resolving all imports for a document.
#[derive(Debug, Clone)]
pub struct ResolvedDocument {
    /// The original document.
    pub document: OdinDocument,
    /// All resolved import paths.
    pub resolved_paths: Vec<String>,
    /// The merged type registry from all imports.
    pub type_registry: TypeRegistry,
}

/// A single resolved import with its parsed schema and alias.
#[derive(Debug, Clone)]
pub struct ResolvedImport {
    /// The resolved file path.
    pub path: String,
    /// The import alias (e.g., "types" from `@import "types.odin" as types`).
    pub alias: Option<String>,
    /// The parsed schema (if successfully parsed).
    pub schema: Option<OdinSchemaDefinition>,
}

/// Result of resolving all imports for a schema.
#[derive(Debug, Clone)]
pub struct ResolvedSchema {
    /// The original schema.
    pub schema: OdinSchemaDefinition,
    /// All resolved import paths.
    pub resolved_paths: Vec<String>,
    /// The merged type registry from all imports.
    pub type_registry: TypeRegistry,
    /// Per-import resolved schemas (path → schema + alias).
    pub imports: Vec<ResolvedImport>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Import Resolver
// ─────────────────────────────────────────────────────────────────────────────

/// Resolves `@import` directives in ODIN documents and schemas.
///
/// The resolver loads imported files, detects circular dependencies,
/// and builds a type registry from imported schemas.
pub struct ImportResolver {
    reader: Box<dyn FileReader>,
    options: ResolverOptions,
    cache: HashMap<String, CachedEntry>,
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum CachedEntry {
    Document,
    Schema(OdinSchemaDefinition),
}

impl ImportResolver {
    /// Create a new import resolver with the given file reader.
    pub fn new(reader: Box<dyn FileReader>, options: ResolverOptions) -> Self {
        Self {
            reader,
            options,
            cache: HashMap::new(),
        }
    }

    /// Resolve a document file and all its imports.
    pub fn resolve_document(
        &mut self,
        file_path: &str,
    ) -> Result<ResolvedDocument, ParseError> {
        let content = self.reader.read_file(file_path).map_err(|e| {
            ParseError::with_message(
                crate::types::errors::ParseErrorCode::UnexpectedCharacter,
                1, 1,
                &format!("Failed to read file '{file_path}': {e}"),
            )
        })?;

        let doc = crate::parser::parse(&content, None)?;
        let mut detector = CircularDetector::new();
        let mut registry = TypeRegistry::new();
        let mut resolved_paths = Vec::new();
        let mut resolved_imports = Vec::new();
        let mut total_imports = 0;

        detector.enter(file_path).map_err(|e| {
            ParseError::with_message(
                crate::types::errors::ParseErrorCode::UnexpectedCharacter,
                1, 1, &e,
            )
        })?;

        self.resolve_imports_recursive(
            file_path,
            &doc.imports,
            &mut detector,
            &mut registry,
            &mut resolved_paths,
            &mut resolved_imports,
            &mut total_imports,
            0,
        )?;

        detector.exit();

        // resolved_imports is tracked but not exposed in ResolvedDocument
        let _ = resolved_imports;

        Ok(ResolvedDocument {
            document: doc,
            resolved_paths,
            type_registry: registry,
        })
    }

    /// Resolve a schema file and all its imports.
    pub fn resolve_schema(
        &mut self,
        file_path: &str,
    ) -> Result<ResolvedSchema, ParseError> {
        let content = self.reader.read_file(file_path).map_err(|e| {
            ParseError::with_message(
                crate::types::errors::ParseErrorCode::UnexpectedCharacter,
                1, 1,
                &format!("Failed to read schema '{file_path}': {e}"),
            )
        })?;

        let schema = crate::validator::schema_parser::parse_schema(&content)?;
        let mut detector = CircularDetector::new();
        let mut registry = TypeRegistry::new();
        let mut resolved_paths = Vec::new();
        let mut resolved_imports = Vec::new();
        let mut total_imports = 0;

        detector.enter(file_path).map_err(|e| {
            ParseError::with_message(
                crate::types::errors::ParseErrorCode::UnexpectedCharacter,
                1, 1, &e,
            )
        })?;

        // Register types from this schema
        registry.register_all(&schema.types, None);

        // Resolve schema imports
        let imports: Vec<_> = schema.imports.iter().map(|i| {
            crate::types::document::OdinImport {
                path: i.path.clone(),
                alias: i.alias.clone(),
                line: 0,
            }
        }).collect();

        self.resolve_imports_recursive(
            file_path,
            &imports,
            &mut detector,
            &mut registry,
            &mut resolved_paths,
            &mut resolved_imports,
            &mut total_imports,
            0,
        )?;

        detector.exit();

        Ok(ResolvedSchema {
            schema,
            resolved_paths,
            type_registry: registry,
            imports: resolved_imports,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_imports_recursive(
        &mut self,
        base_path: &str,
        imports: &[crate::types::document::OdinImport],
        detector: &mut CircularDetector,
        registry: &mut TypeRegistry,
        resolved_paths: &mut Vec<String>,
        resolved_imports: &mut Vec<ResolvedImport>,
        total_imports: &mut usize,
        depth: usize,
    ) -> Result<(), ParseError> {
        if depth > self.options.max_import_depth {
            return Err(ParseError::with_message(
                crate::types::errors::ParseErrorCode::MaximumDepthExceeded,
                1, 1,
                &format!("Import depth {} exceeds maximum {}", depth, self.options.max_import_depth),
            ));
        }

        for import in imports {
            *total_imports += 1;
            if *total_imports > self.options.max_total_imports {
                return Err(ParseError::with_message(
                    crate::types::errors::ParseErrorCode::MaximumDocumentSizeExceeded,
                    import.line, 1,
                    &format!("Total imports {} exceeds maximum {}", total_imports, self.options.max_total_imports),
                ));
            }

            // Validate extension
            let import_path = &import.path;
            if !self.options.allowed_extensions.iter().any(|ext| import_path.ends_with(ext.as_str())) {
                return Err(ParseError::with_message(
                    crate::types::errors::ParseErrorCode::UnexpectedCharacter,
                    import.line, 1,
                    &format!("Import path '{import_path}' has disallowed extension"),
                ));
            }

            // Resolve path
            let resolved = self.reader.resolve_path(base_path, import_path).map_err(|e| {
                ParseError::with_message(
                    crate::types::errors::ParseErrorCode::UnexpectedCharacter,
                    import.line, 1,
                    &format!("Failed to resolve import '{import_path}': {e}"),
                )
            })?;

            // Check circular
            if detector.is_circular(&resolved) {
                return Err(ParseError::with_message(
                    crate::types::errors::ParseErrorCode::UnexpectedCharacter,
                    import.line, 1,
                    &format!("Circular import detected: {import_path}"),
                ));
            }

            // Check cache
            let normalized = normalize_path(&resolved);
            if let Some(cached) = self.cache.get(&normalized) {
                match cached.clone() {
                    CachedEntry::Schema(ref schema) => {
                        registry.register_all(&schema.types, import.alias.as_deref());
                        resolved_imports.push(ResolvedImport {
                            path: resolved.clone(),
                            alias: import.alias.clone(),
                            schema: Some(schema.clone()),
                        });
                    }
                    CachedEntry::Document => {
                        resolved_imports.push(ResolvedImport {
                            path: resolved.clone(),
                            alias: import.alias.clone(),
                            schema: None,
                        });
                    }
                }
                resolved_paths.push(resolved);
                continue;
            }

            // Load and parse
            let content = self.reader.read_file(&resolved).map_err(|e| {
                ParseError::with_message(
                    crate::types::errors::ParseErrorCode::UnexpectedCharacter,
                    import.line, 1,
                    &format!("Failed to read import '{resolved}': {e}"),
                )
            })?;

            if content.len() > self.options.max_file_size {
                return Err(ParseError::with_message(
                    crate::types::errors::ParseErrorCode::MaximumDocumentSizeExceeded,
                    import.line, 1,
                    &format!("Import file '{import_path}' exceeds maximum size"),
                ));
            }

            detector.enter(&resolved).map_err(|e| {
                ParseError::with_message(
                    crate::types::errors::ParseErrorCode::UnexpectedCharacter,
                    import.line, 1, &e,
                )
            })?;

            if self.options.schema_mode {
                // Parse as schema
                let schema = crate::validator::schema_parser::parse_schema(&content)?;
                registry.register_all(&schema.types, import.alias.as_deref());

                // Track this resolved import
                resolved_imports.push(ResolvedImport {
                    path: resolved.clone(),
                    alias: import.alias.clone(),
                    schema: Some(schema.clone()),
                });

                // Convert schema imports to OdinImport for recursion
                let nested_imports: Vec<_> = schema.imports.iter().map(|i| {
                    crate::types::document::OdinImport {
                        path: i.path.clone(),
                        alias: i.alias.clone(),
                        line: 0,
                    }
                }).collect();

                self.cache.insert(normalized, CachedEntry::Schema(schema));

                self.resolve_imports_recursive(
                    &resolved,
                    &nested_imports,
                    detector,
                    registry,
                    resolved_paths,
                    resolved_imports,
                    total_imports,
                    depth + 1,
                )?;
            } else {
                // Parse as document
                let doc = crate::parser::parse(&content, None)?;
                let nested_imports = doc.imports.clone();

                resolved_imports.push(ResolvedImport {
                    path: resolved.clone(),
                    alias: import.alias.clone(),
                    schema: None,
                });

                self.cache.insert(normalized, CachedEntry::Document);

                self.resolve_imports_recursive(
                    &resolved,
                    &nested_imports,
                    detector,
                    registry,
                    resolved_paths,
                    resolved_imports,
                    total_imports,
                    depth + 1,
                )?;
            }

            detector.exit();
            resolved_paths.push(resolved);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockReader {
        files: HashMap<String, String>,
    }

    impl MockReader {
        fn new() -> Self {
            Self { files: HashMap::new() }
        }

        fn add_file(&mut self, path: &str, content: &str) {
            self.files.insert(normalize_path(path), content.to_string());
        }
    }

    impl FileReader for MockReader {
        fn read_file(&self, path: &str) -> Result<String, String> {
            self.files.get(&normalize_path(path))
                .cloned()
                .ok_or_else(|| format!("File not found: {}", path))
        }

        fn resolve_path(&self, base: &str, import: &str) -> Result<String, String> {
            // Simple resolution: combine base dir + import
            let base = normalize_path(base);
            if let Some(slash) = base.rfind('/') {
                Ok(format!("{}/{}", &base[..slash], import.trim_matches('"')))
            } else {
                Ok(import.trim_matches('"').to_string())
            }
        }
    }

    #[test]
    fn test_circular_detector() {
        let mut detector = CircularDetector::new();
        assert!(detector.enter("a.odin").is_ok());
        assert!(detector.enter("b.odin").is_ok());
        assert!(detector.is_circular("a.odin"));
        assert!(detector.enter("a.odin").is_err());
        detector.exit();
        detector.exit();
        assert!(!detector.is_circular("a.odin"));
    }

    #[test]
    fn test_type_registry_local() {
        let mut registry = TypeRegistry::new();
        let mut types = HashMap::new();
        types.insert("Address".to_string(), SchemaType {
            name: "Address".to_string(),
            description: None,
            fields: vec![],
            parents: vec![],
        });
        registry.register_all(&types, None);
        assert!(registry.lookup("Address").is_some());
    }

    #[test]
    fn test_type_registry_namespaced() {
        let mut registry = TypeRegistry::new();
        let mut types = HashMap::new();
        types.insert("Address".to_string(), SchemaType {
            name: "Address".to_string(),
            description: None,
            fields: vec![],
            parents: vec![],
        });
        registry.register_all(&types, Some("types"));
        assert!(registry.lookup("types.Address").is_some());
        assert!(registry.lookup("Address").is_some()); // Falls through to namespace search
    }

    #[test]
    fn test_resolve_document_no_imports() {
        let mut reader = MockReader::new();
        reader.add_file("/test.odin", "name = \"Alice\"");

        let mut resolver = ImportResolver::new(
            Box::new(reader),
            ResolverOptions { schema_mode: false, ..Default::default() },
        );

        let result = resolver.resolve_document("/test.odin").unwrap();
        assert_eq!(result.document.get_string("name"), Some("Alice"));
        assert!(result.resolved_paths.is_empty());
    }

    #[test]
    fn test_resolve_schema_with_import() {
        let mut reader = MockReader::new();
        reader.add_file("/schema.odin", "@import \"types.odin\"\n\n{person}\nemail = :format email");
        reader.add_file("/types.odin", "@PhoneNumber\ndigits = :(10..15)");

        let mut resolver = ImportResolver::new(
            Box::new(reader),
            ResolverOptions::default(),
        );

        let result = resolver.resolve_schema("/schema.odin").unwrap();
        assert!(result.type_registry.lookup("PhoneNumber").is_some());
        assert_eq!(result.resolved_paths.len(), 1);
    }

    #[test]
    fn test_resolve_schema_with_alias() {
        let mut reader = MockReader::new();
        reader.add_file("/schema.odin", "@import \"types.odin\" as t\n\n{person}\nemail = @t.Email");
        reader.add_file("/types.odin", "@Email\n= :format email");

        let mut resolver = ImportResolver::new(
            Box::new(reader),
            ResolverOptions::default(),
        );

        let result = resolver.resolve_schema("/schema.odin").unwrap();
        assert!(result.type_registry.lookup("t.Email").is_some());
    }

    #[test]
    fn test_circular_import_detection() {
        let mut reader = MockReader::new();
        reader.add_file("/a.odin", "@import \"b.odin\"");
        reader.add_file("/b.odin", "@import \"a.odin\"");

        let mut resolver = ImportResolver::new(
            Box::new(reader),
            ResolverOptions::default(),
        );

        let result = resolver.resolve_schema("/a.odin");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("ircular"), "Expected circular error: {}", err.message);
    }

    #[test]
    fn test_max_depth_exceeded() {
        let mut reader = MockReader::new();
        // Create a chain: 0 -> 1 -> 2 -> ... -> 35
        for i in 0..35 {
            let content = format!("@import \"{}.odin\"", i + 1);
            reader.add_file(&format!("/{}.odin", i), &content);
        }
        reader.add_file("/35.odin", "name = \"end\"");

        let mut resolver = ImportResolver::new(
            Box::new(reader),
            ResolverOptions { max_import_depth: 5, ..Default::default() },
        );

        let result = resolver.resolve_schema("/0.odin");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("depth"));
    }

    #[test]
    fn test_cached_imports() {
        let mut reader = MockReader::new();
        reader.add_file("/schema.odin", "@import \"types.odin\"\n@import \"types.odin\"");
        reader.add_file("/types.odin", "@Address\nstreet = ");

        let mut resolver = ImportResolver::new(
            Box::new(reader),
            ResolverOptions::default(),
        );

        let result = resolver.resolve_schema("/schema.odin").unwrap();
        // Both imports resolve, second from cache
        assert_eq!(result.resolved_paths.len(), 2);
        assert!(result.type_registry.lookup("Address").is_some());
    }

    #[test]
    fn test_nested_imports() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"mid.odin\"");
        reader.add_file("/mid.odin", "@import \"leaf.odin\"\n@MiddleType\nfoo = ");
        reader.add_file("/leaf.odin", "@LeafType\nbar = ");

        let mut resolver = ImportResolver::new(
            Box::new(reader),
            ResolverOptions::default(),
        );

        let result = resolver.resolve_schema("/main.odin").unwrap();
        assert!(result.type_registry.lookup("MiddleType").is_some());
        assert!(result.type_registry.lookup("LeafType").is_some());
        assert_eq!(result.resolved_paths.len(), 2);
    }

    #[test]
    fn test_resolved_schema_includes_per_import_schemas() {
        let mut reader = MockReader::new();
        reader.add_file("/schema.odin", "@import \"types.odin\" as t\n\n{person}\nemail = :format email");
        reader.add_file("/types.odin", "@PhoneNumber\ndigits = :(10..15)");

        let mut resolver = ImportResolver::new(
            Box::new(reader),
            ResolverOptions::default(),
        );

        let result = resolver.resolve_schema("/schema.odin").unwrap();
        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].alias, Some("t".to_string()));
        assert!(result.imports[0].schema.is_some());
        let import_schema = result.imports[0].schema.as_ref().unwrap();
        assert!(import_schema.types.contains_key("PhoneNumber"));
    }

    #[test]
    fn test_resolve_then_flatten() {
        use super::schema_flattener::{SchemaFlattener, FlattenerOptions, flatten_schema};

        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"types.odin\" as types\n\n@Policy\naddr = @types.Address");
        reader.add_file("/types.odin", "@Address\nstreet = \"\"\ncity = \"\"");

        let mut resolver = ImportResolver::new(
            Box::new(reader),
            ResolverOptions::default(),
        );

        let resolved = resolver.resolve_schema("/main.odin").unwrap();
        let result = flatten_schema(&resolved);

        // Flattened schema should have no imports
        assert!(result.schema.imports.is_empty());

        // Both types should be present (Policy from primary, types_Address from import)
        assert!(result.schema.types.contains_key("Policy"), "Missing Policy");
        assert!(result.schema.types.contains_key("types_Address"), "Missing types_Address");

        // types_Address should have its fields
        let addr_type = result.schema.types.get("types_Address").unwrap();
        let field_names: Vec<&str> = addr_type.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(field_names.contains(&"street"));
        assert!(field_names.contains(&"city"));
    }

    #[test]
    fn test_resolve_flatten_serialize_roundtrip() {
        use super::schema_flattener::{flatten_schema, bundle_schema, FlattenerOptions};

        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"types.odin\" as types\n\n@Policy\nnumber = \"\"\naddr = @types.Address");
        reader.add_file("/types.odin", "@Address\nline1 = \"\"\ncity = \"\"");

        let mut resolver = ImportResolver::new(
            Box::new(reader),
            ResolverOptions::default(),
        );

        let resolved = resolver.resolve_schema("/main.odin").unwrap();
        let (text, warnings) = bundle_schema(&resolved, FlattenerOptions::default());

        // The serialized text should contain both types
        assert!(text.contains("Policy"), "Serialized text should contain Policy");
        assert!(text.contains("types_Address") || text.contains("Address"),
            "Serialized text should contain Address type");

        // The text should NOT contain import directives
        assert!(!text.contains("@import"), "Flattened text should not have imports");

        // Re-parse the flattened schema
        let reparsed = crate::validator::schema_parser::parse_schema(&text).unwrap();
        assert!(reparsed.imports.is_empty());
        // Types should be preserved
        assert!(!reparsed.types.is_empty(), "Re-parsed schema should have types");
    }

    // =====================================================================
    // Additional CircularDetector tests
    // =====================================================================

    #[test]
    fn test_circular_detector_empty() {
        let detector = CircularDetector::new();
        assert!(!detector.is_circular("anything.odin"));
    }

    #[test]
    fn test_circular_detector_single_entry_exit() {
        let mut detector = CircularDetector::new();
        detector.enter("a.odin").unwrap();
        assert!(detector.is_circular("a.odin"));
        detector.exit();
        assert!(!detector.is_circular("a.odin"));
    }

    #[test]
    fn test_circular_detector_normalizes_paths() {
        let mut detector = CircularDetector::new();
        detector.enter("A\\B\\C.odin").unwrap();
        assert!(detector.is_circular("a/b/c.odin"));
    }

    #[test]
    fn test_circular_detector_format_cycle() {
        let mut detector = CircularDetector::new();
        detector.enter("a.odin").unwrap();
        detector.enter("b.odin").unwrap();
        detector.enter("c.odin").unwrap();
        let err = detector.enter("a.odin").unwrap_err();
        assert!(err.contains("a.odin -> b.odin -> c.odin -> a.odin"));
    }

    #[test]
    fn test_circular_detector_deep_chain_no_false_positive() {
        let mut detector = CircularDetector::new();
        for i in 0..20 {
            detector.enter(&format!("{i}.odin")).unwrap();
        }
        assert!(!detector.is_circular("99.odin"));
        assert!(detector.is_circular("0.odin"));
        assert!(detector.is_circular("19.odin"));
    }

    #[test]
    fn test_circular_detector_exit_order() {
        let mut detector = CircularDetector::new();
        detector.enter("a.odin").unwrap();
        detector.enter("b.odin").unwrap();
        detector.enter("c.odin").unwrap();
        detector.exit();
        assert!(!detector.is_circular("c.odin"));
        assert!(detector.is_circular("a.odin"));
        assert!(detector.is_circular("b.odin"));
        detector.exit();
        assert!(!detector.is_circular("b.odin"));
        assert!(detector.is_circular("a.odin"));
    }

    #[test]
    fn test_circular_detector_re_enter_after_exit() {
        let mut detector = CircularDetector::new();
        detector.enter("a.odin").unwrap();
        detector.exit();
        detector.enter("a.odin").unwrap();
        assert!(detector.is_circular("a.odin"));
    }

    #[test]
    fn test_circular_detector_double_enter_fails() {
        let mut detector = CircularDetector::new();
        detector.enter("a.odin").unwrap();
        assert!(detector.enter("a.odin").is_err());
    }

    #[test]
    fn test_circular_detector_case_insensitive() {
        let mut detector = CircularDetector::new();
        detector.enter("File.ODIN").unwrap();
        assert!(detector.is_circular("file.odin"));
        assert!(detector.is_circular("FILE.ODIN"));
    }

    #[test]
    fn test_circular_detector_mixed_separators() {
        let mut detector = CircularDetector::new();
        detector.enter("dir/sub/file.odin").unwrap();
        assert!(detector.is_circular("dir\\sub\\file.odin"));
    }

    #[test]
    fn test_circular_detector_exit_empty_safe() {
        let mut detector = CircularDetector::new();
        detector.exit();
        assert!(!detector.is_circular("anything"));
    }

    #[test]
    fn test_circular_detector_long_cycle_message() {
        let mut detector = CircularDetector::new();
        detector.enter("first.odin").unwrap();
        detector.enter("second.odin").unwrap();
        detector.enter("third.odin").unwrap();
        detector.enter("fourth.odin").unwrap();
        let err = detector.enter("first.odin").unwrap_err();
        assert!(err.contains("first.odin"));
        assert!(err.contains("fourth.odin"));
    }

    // =====================================================================
    // Additional TypeRegistry tests
    // =====================================================================

    #[test]
    fn test_type_registry_empty_lookup() {
        let registry = TypeRegistry::new();
        assert!(registry.lookup("NonExistent").is_none());
    }

    #[test]
    fn test_type_registry_all_type_names_empty() {
        let registry = TypeRegistry::new();
        assert!(registry.all_type_names().is_empty());
    }

    #[test]
    fn test_type_registry_all_type_names_mixed() {
        let mut registry = TypeRegistry::new();
        let mut local = HashMap::new();
        local.insert("Person".to_string(), SchemaType {
            name: "Person".to_string(), description: None, fields: vec![], parents: vec![],
        });
        registry.register_all(&local, None);

        let mut ns_types = HashMap::new();
        ns_types.insert("Address".to_string(), SchemaType {
            name: "Address".to_string(), description: None, fields: vec![], parents: vec![],
        });
        registry.register_all(&ns_types, Some("types"));

        let names = registry.all_type_names();
        assert!(names.contains(&"Person".to_string()));
        assert!(names.contains(&"types.Address".to_string()));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn test_type_registry_local_overrides_on_re_register() {
        let mut registry = TypeRegistry::new();
        let mut types1 = HashMap::new();
        types1.insert("X".to_string(), SchemaType {
            name: "X".to_string(), description: Some("first".to_string()), fields: vec![], parents: vec![],
        });
        registry.register_all(&types1, None);

        let mut types2 = HashMap::new();
        types2.insert("X".to_string(), SchemaType {
            name: "X".to_string(), description: Some("second".to_string()), fields: vec![], parents: vec![],
        });
        registry.register_all(&types2, None);

        assert_eq!(registry.lookup("X").unwrap().description.as_deref(), Some("second"));
    }

    #[test]
    fn test_type_registry_unqualified_searches_namespaces() {
        let mut registry = TypeRegistry::new();
        let mut types = HashMap::new();
        types.insert("Unique".to_string(), SchemaType {
            name: "Unique".to_string(), description: None, fields: vec![], parents: vec![],
        });
        registry.register_all(&types, Some("ns1"));
        assert!(registry.lookup("Unique").is_some());
    }

    #[test]
    fn test_type_registry_qualified_wrong_namespace() {
        let mut registry = TypeRegistry::new();
        let mut types = HashMap::new();
        types.insert("Foo".to_string(), SchemaType {
            name: "Foo".to_string(), description: None, fields: vec![], parents: vec![],
        });
        registry.register_all(&types, Some("ns1"));
        assert!(registry.lookup("ns2.Foo").is_none());
    }

    #[test]
    fn test_type_registry_multiple_namespaces_qualified() {
        let mut registry = TypeRegistry::new();
        for (ns, desc) in &[("a", "from_a"), ("b", "from_b")] {
            let mut types = HashMap::new();
            types.insert("Type1".to_string(), SchemaType {
                name: "Type1".to_string(), description: Some(desc.to_string()), fields: vec![], parents: vec![],
            });
            registry.register_all(&types, Some(ns));
        }
        assert_eq!(registry.lookup("a.Type1").unwrap().description.as_deref(), Some("from_a"));
        assert_eq!(registry.lookup("b.Type1").unwrap().description.as_deref(), Some("from_b"));
    }

    #[test]
    fn test_type_registry_register_empty_map() {
        let mut registry = TypeRegistry::new();
        let empty: HashMap<String, SchemaType> = HashMap::new();
        registry.register_all(&empty, None);
        assert!(registry.all_type_names().is_empty());
    }

    #[test]
    fn test_type_registry_register_empty_map_with_ns() {
        let mut registry = TypeRegistry::new();
        let empty: HashMap<String, SchemaType> = HashMap::new();
        registry.register_all(&empty, Some("ns"));
        assert!(registry.all_type_names().is_empty());
    }

    #[test]
    fn test_type_registry_multiple_local_types() {
        let mut registry = TypeRegistry::new();
        let mut types = HashMap::new();
        for name in &["Alpha", "Beta", "Gamma"] {
            types.insert(name.to_string(), SchemaType {
                name: name.to_string(), description: None, fields: vec![], parents: vec![],
            });
        }
        registry.register_all(&types, None);
        assert_eq!(registry.all_type_names().len(), 3);
    }

    #[test]
    fn test_type_registry_same_name_diff_namespaces() {
        let mut registry = TypeRegistry::new();
        for ns in &["ns1", "ns2", "ns3"] {
            let mut types = HashMap::new();
            types.insert("Shared".to_string(), SchemaType {
                name: "Shared".to_string(), description: Some(format!("from_{ns}")), fields: vec![], parents: vec![],
            });
            registry.register_all(&types, Some(ns));
        }
        assert!(registry.lookup("ns1.Shared").is_some());
        assert!(registry.lookup("ns2.Shared").is_some());
        assert!(registry.lookup("ns3.Shared").is_some());
    }

    #[test]
    fn test_type_registry_overwrite_namespaced() {
        let mut registry = TypeRegistry::new();
        let mut t1 = HashMap::new();
        t1.insert("T".to_string(), SchemaType {
            name: "T".to_string(), description: Some("v1".to_string()), fields: vec![], parents: vec![],
        });
        registry.register_all(&t1, Some("ns"));

        let mut t2 = HashMap::new();
        t2.insert("T".to_string(), SchemaType {
            name: "T".to_string(), description: Some("v2".to_string()), fields: vec![], parents: vec![],
        });
        registry.register_all(&t2, Some("ns"));

        assert_eq!(registry.lookup("ns.T").unwrap().description.as_deref(), Some("v2"));
    }

    #[test]
    fn test_type_registry_local_shadows_namespace() {
        let mut registry = TypeRegistry::new();
        let mut ns_types = HashMap::new();
        ns_types.insert("Clash".to_string(), SchemaType {
            name: "Clash".to_string(), description: Some("namespaced".to_string()), fields: vec![], parents: vec![],
        });
        registry.register_all(&ns_types, Some("ns"));

        let mut local = HashMap::new();
        local.insert("Clash".to_string(), SchemaType {
            name: "Clash".to_string(), description: Some("local".to_string()), fields: vec![], parents: vec![],
        });
        registry.register_all(&local, None);

        assert_eq!(registry.lookup("Clash").unwrap().description.as_deref(), Some("local"));
    }

    #[test]
    fn test_type_registry_with_fields() {
        let mut registry = TypeRegistry::new();
        let mut types = HashMap::new();
        types.insert("WithFields".to_string(), SchemaType {
            name: "WithFields".to_string(),
            description: None,
            fields: vec![crate::types::schema::SchemaField {
                name: "f1".to_string(),
                field_type: crate::types::schema::SchemaFieldType::String,
                required: true, confidential: false, deprecated: false, immutable: false,
                description: None, constraints: vec![], default_value: None, conditionals: vec![],
            }],
            parents: vec![],
        });
        registry.register_all(&types, None);
        assert_eq!(registry.lookup("WithFields").unwrap().fields.len(), 1);
    }

    #[test]
    fn test_type_registry_only_namespaced() {
        let mut registry = TypeRegistry::new();
        let mut types = HashMap::new();
        types.insert("X".to_string(), SchemaType {
            name: "X".to_string(), description: None, fields: vec![], parents: vec![],
        });
        registry.register_all(&types, Some("pkg"));
        let names = registry.all_type_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"pkg.X".to_string()));
    }

    // =====================================================================
    // normalize_path tests
    // =====================================================================

    #[test]
    fn test_normalize_path_backslashes() {
        assert_eq!(normalize_path("C:\\foo\\bar.odin"), "c:/foo/bar.odin");
    }

    #[test]
    fn test_normalize_path_lowercase() {
        assert_eq!(normalize_path("/Foo/BAR.odin"), "/foo/bar.odin");
    }

    #[test]
    fn test_normalize_path_already_normalized() {
        assert_eq!(normalize_path("/foo/bar.odin"), "/foo/bar.odin");
    }

    #[test]
    fn test_normalize_path_empty() {
        assert_eq!(normalize_path(""), "");
    }

    #[test]
    fn test_normalize_path_mixed() {
        assert_eq!(normalize_path("C:\\Users\\ODIN\\File.Odin"), "c:/users/odin/file.odin");
    }

    #[test]
    fn test_normalize_path_single_filename() {
        assert_eq!(normalize_path("File.odin"), "file.odin");
    }

    // =====================================================================
    // Additional ImportResolver tests
    // =====================================================================

    #[test]
    fn test_resolve_missing_file() {
        let reader = MockReader::new();
        let mut resolver = ImportResolver::new(
            Box::new(reader),
            ResolverOptions { schema_mode: false, ..Default::default() },
        );
        let result = resolver.resolve_document("/nonexistent.odin");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("File not found"));
    }

    #[test]
    fn test_resolve_missing_import_file() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"missing.odin\"");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/main.odin");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("File not found"));
    }

    #[test]
    fn test_disallowed_extension() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"data.json\"");
        reader.add_file("/data.json", "{}");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/main.odin");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("disallowed extension"));
    }

    #[test]
    fn test_custom_allowed_extensions() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"types.schema\"");
        reader.add_file("/types.schema", "@Foo\nbar = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions {
            allowed_extensions: vec![".odin".to_string(), ".schema".to_string()],
            ..Default::default()
        });
        assert!(resolver.resolve_schema("/main.odin").is_ok());
    }

    #[test]
    fn test_max_total_imports_exceeded() {
        let mut reader = MockReader::new();
        let mut content = String::new();
        for i in 0..5 {
            content.push_str(&format!("@import \"{i}.odin\"\n"));
            reader.add_file(&format!("/{i}.odin"), &format!("@Type{i}\nf = "));
        }
        reader.add_file("/main.odin", &content);
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions {
            max_total_imports: 3, ..Default::default()
        });
        let result = resolver.resolve_schema("/main.odin");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("exceeds maximum"));
    }

    #[test]
    fn test_diamond_imports() {
        let mut reader = MockReader::new();
        reader.add_file("/a.odin", "@import \"b.odin\"\n@import \"c.odin\"");
        reader.add_file("/b.odin", "@import \"d.odin\"\n@BType\nf = ");
        reader.add_file("/c.odin", "@import \"d.odin\"\n@CType\nf = ");
        reader.add_file("/d.odin", "@DType\nval = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/a.odin").unwrap();
        assert!(result.type_registry.lookup("DType").is_some());
        assert!(result.type_registry.lookup("BType").is_some());
        assert!(result.type_registry.lookup("CType").is_some());
    }

    #[test]
    fn test_self_import_detected() {
        let mut reader = MockReader::new();
        reader.add_file("/self.odin", "@import \"self.odin\"");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/self.odin");
        assert!(result.is_err());
    }

    #[test]
    fn test_three_way_circular() {
        let mut reader = MockReader::new();
        reader.add_file("/a.odin", "@import \"b.odin\"");
        reader.add_file("/b.odin", "@import \"c.odin\"");
        reader.add_file("/c.odin", "@import \"a.odin\"");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        assert!(resolver.resolve_schema("/a.odin").is_err());
    }

    #[test]
    fn test_cache_reuse_across_calls() {
        let mut reader = MockReader::new();
        reader.add_file("/s1.odin", "@import \"shared.odin\"");
        reader.add_file("/s2.odin", "@import \"shared.odin\"");
        reader.add_file("/shared.odin", "@Shared\nval = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        assert!(resolver.resolve_schema("/s1.odin").unwrap().type_registry.lookup("Shared").is_some());
        assert!(resolver.resolve_schema("/s2.odin").unwrap().type_registry.lookup("Shared").is_some());
    }

    #[test]
    fn test_document_mode_simple() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "name = \"Alice\"\nage = ##30\n");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions {
            schema_mode: false, ..Default::default()
        });
        let result = resolver.resolve_document("/main.odin").unwrap();
        assert_eq!(result.document.get_string("name"), Some("Alice"));
    }

    #[test]
    fn test_empty_schema_resolves() {
        let mut reader = MockReader::new();
        reader.add_file("/empty.odin", "");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/empty.odin").unwrap();
        assert!(result.schema.types.is_empty());
        assert!(result.resolved_paths.is_empty());
    }

    #[test]
    fn test_resolver_options_defaults() {
        let opts = ResolverOptions::default();
        assert_eq!(opts.max_import_depth, 32);
        assert_eq!(opts.max_total_imports, 100);
        assert!(opts.schema_mode);
        assert_eq!(opts.allowed_extensions, vec![".odin".to_string()]);
        assert_eq!(opts.max_file_size, 10 * 1024 * 1024);
    }

    #[test]
    fn test_resolved_schema_imports_populated() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"a.odin\"\n@import \"b.odin\" as ns");
        reader.add_file("/a.odin", "@TypeA\nf = ");
        reader.add_file("/b.odin", "@TypeB\nf = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/main.odin").unwrap();
        assert_eq!(result.imports.len(), 2);
        assert_eq!(result.imports[0].alias, None);
        assert_eq!(result.imports[1].alias, Some("ns".to_string()));
    }

    #[test]
    fn test_deep_nested_within_limit() {
        let mut reader = MockReader::new();
        for i in 0..4 {
            reader.add_file(&format!("/{i}.odin"), &format!("@import \"{}.odin\"\n@Type{i}\nf = ", i + 1));
        }
        reader.add_file("/4.odin", "@Type4\nf = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/0.odin").unwrap();
        for i in 0..5 {
            assert!(result.type_registry.lookup(&format!("Type{i}")).is_some(), "Missing Type{i}");
        }
    }

    #[test]
    fn test_import_with_multiple_types() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"types.odin\" as t");
        reader.add_file("/types.odin", "@Foo\na = \n\n@Bar\nb = \n\n@Baz\nc = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/main.odin").unwrap();
        assert!(result.type_registry.lookup("t.Foo").is_some());
        assert!(result.type_registry.lookup("t.Bar").is_some());
        assert!(result.type_registry.lookup("t.Baz").is_some());
    }

    #[test]
    fn test_resolve_schema_no_imports_types() {
        let mut reader = MockReader::new();
        reader.add_file("/schema.odin", "@Person\nname = \"\"\nage = ##0");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/schema.odin").unwrap();
        assert!(result.resolved_paths.is_empty());
        assert!(result.imports.is_empty());
    }

    #[test]
    fn test_resolve_preserves_primary_types() {
        let mut reader = MockReader::new();
        reader.add_file("/schema.odin", "@Widget\nwidth = ##0\nheight = ##0");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/schema.odin").unwrap();
        assert!(result.schema.types.contains_key("Widget"));
    }

    #[test]
    fn test_resolve_import_chain_depth_2() {
        let mut reader = MockReader::new();
        reader.add_file("/a.odin", "@import \"b.odin\"");
        reader.add_file("/b.odin", "@import \"c.odin\"\n@TypeB\nfb = ");
        reader.add_file("/c.odin", "@TypeC\nfc = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/a.odin").unwrap();
        assert!(result.type_registry.lookup("TypeB").is_some());
        assert!(result.type_registry.lookup("TypeC").is_some());
    }

    #[test]
    fn test_resolve_depth_one_over_limit() {
        let mut reader = MockReader::new();
        for i in 0..4 {
            reader.add_file(&format!("/{i}.odin"), &format!("@import \"{}.odin\"\n@Type{i}\nf = ", i + 1));
        }
        reader.add_file("/4.odin", "@Type4\nf = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions {
            max_import_depth: 3, ..Default::default()
        });
        assert!(resolver.resolve_schema("/0.odin").is_err());
    }

    #[test]
    fn test_resolve_total_imports_at_limit() {
        let mut reader = MockReader::new();
        let mut content = String::new();
        for i in 0..3 {
            content.push_str(&format!("@import \"{i}.odin\"\n"));
            reader.add_file(&format!("/{i}.odin"), &format!("@Type{i}\nf = "));
        }
        reader.add_file("/main.odin", &content);
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions {
            max_total_imports: 3, ..Default::default()
        });
        assert!(resolver.resolve_schema("/main.odin").is_ok());
    }

    #[test]
    fn test_resolve_aliased_registry_lookup() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"lib.odin\" as mylib");
        reader.add_file("/lib.odin", "@Widget\nw = \n\n@Gadget\ng = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/main.odin").unwrap();
        assert!(result.type_registry.lookup("mylib.Widget").is_some());
        assert!(result.type_registry.lookup("mylib.Gadget").is_some());
    }

    #[test]
    fn test_resolve_disallowed_txt() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"data.txt\"");
        reader.add_file("/data.txt", "some text");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        assert!(resolver.resolve_schema("/main.odin").is_err());
    }

    #[test]
    fn test_resolve_empty_document() {
        let mut reader = MockReader::new();
        reader.add_file("/empty.odin", "");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions {
            schema_mode: false, ..Default::default()
        });
        assert!(resolver.resolve_document("/empty.odin").unwrap().resolved_paths.is_empty());
    }

    #[test]
    fn test_resolve_whitespace_schema() {
        let mut reader = MockReader::new();
        reader.add_file("/ws.odin", "   \n\n  \n");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        assert!(resolver.resolve_schema("/ws.odin").unwrap().schema.types.is_empty());
    }

    #[test]
    fn test_resolve_comments_only_schema() {
        let mut reader = MockReader::new();
        reader.add_file("/c.odin", "; comment\n; another\n");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        assert!(resolver.resolve_schema("/c.odin").unwrap().schema.types.is_empty());
    }

    #[test]
    fn test_resolve_diamond_all_registered() {
        let mut reader = MockReader::new();
        reader.add_file("/root.odin", "@import \"left.odin\"\n@import \"right.odin\"");
        reader.add_file("/left.odin", "@import \"shared.odin\"\n@Left\nl = ");
        reader.add_file("/right.odin", "@import \"shared.odin\"\n@Right\nr = ");
        reader.add_file("/shared.odin", "@Shared\ns = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/root.odin").unwrap();
        assert!(result.type_registry.lookup("Left").is_some());
        assert!(result.type_registry.lookup("Right").is_some());
        assert!(result.type_registry.lookup("Shared").is_some());
    }

    #[test]
    fn test_resolve_two_primary_types() {
        let mut reader = MockReader::new();
        reader.add_file("/schema.odin", "@Policy\nnumber = \"\"\n\n@Claim\nid = ##0");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/schema.odin").unwrap();
        assert!(result.schema.types.contains_key("Policy"));
        assert!(result.schema.types.contains_key("Claim"));
    }

    #[test]
    fn test_resolve_import_with_local_types() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"lib.odin\"\n\n@Local\nval = ");
        reader.add_file("/lib.odin", "@Remote\nrv = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/main.odin").unwrap();
        assert!(result.schema.types.contains_key("Local"));
        assert!(result.type_registry.lookup("Remote").is_some());
    }

    #[test]
    fn test_mock_reader_resolve_path_no_slash() {
        let reader = MockReader::new();
        assert_eq!(reader.resolve_path("base", "import.odin").unwrap(), "import.odin");
    }

    #[test]
    fn test_mock_reader_resolve_path_with_dir() {
        let reader = MockReader::new();
        assert_eq!(reader.resolve_path("/dir/base.odin", "other.odin").unwrap(), "/dir/other.odin");
    }

    #[test]
    fn test_mock_reader_not_found() {
        let reader = MockReader::new();
        assert!(reader.read_file("/nonexistent.odin").is_err());
    }

    #[test]
    fn test_four_level_circular() {
        let mut reader = MockReader::new();
        reader.add_file("/a.odin", "@import \"b.odin\"");
        reader.add_file("/b.odin", "@import \"c.odin\"");
        reader.add_file("/c.odin", "@import \"d.odin\"");
        reader.add_file("/d.odin", "@import \"a.odin\"");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        assert!(resolver.resolve_schema("/a.odin").is_err());
    }

    #[test]
    fn test_resolve_many_imports() {
        let mut reader = MockReader::new();
        let mut content = String::new();
        for i in 0..10 {
            content.push_str(&format!("@import \"t{i}.odin\"\n"));
            reader.add_file(&format!("/t{i}.odin"), &format!("@Type{i}\nf = "));
        }
        reader.add_file("/main.odin", &content);
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/main.odin").unwrap();
        assert_eq!(result.resolved_paths.len(), 10);
    }

    #[test]
    fn test_resolve_import_alias_in_result() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"types.odin\" as myTypes");
        reader.add_file("/types.odin", "@Addr\nline = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/main.odin").unwrap();
        assert_eq!(result.imports[0].alias, Some("myTypes".to_string()));
    }

    #[test]
    fn test_resolve_no_alias_in_result() {
        let mut reader = MockReader::new();
        reader.add_file("/main.odin", "@import \"types.odin\"");
        reader.add_file("/types.odin", "@Addr\nline = ");
        let mut resolver = ImportResolver::new(Box::new(reader), ResolverOptions::default());
        let result = resolver.resolve_schema("/main.odin").unwrap();
        assert_eq!(result.imports[0].alias, None);
    }

    #[test]
    fn test_resolve_options_custom_file_size() {
        let opts = ResolverOptions { max_file_size: 1024, ..Default::default() };
        assert_eq!(opts.max_file_size, 1024);
    }

    #[test]
    fn test_resolve_options_custom_extensions_count() {
        let opts = ResolverOptions {
            allowed_extensions: vec![".odin".to_string(), ".schema".to_string(), ".types".to_string()],
            ..Default::default()
        };
        assert_eq!(opts.allowed_extensions.len(), 3);
    }
}
