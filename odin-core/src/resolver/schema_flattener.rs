//! Schema flattener — merges all imported schemas into a single flat schema.
//!
//! Given a `ResolvedSchema` (from the `ImportResolver`), produces a single
//! `OdinSchemaDefinition` with no imports, all types merged and namespaced,
//! type inheritance expanded, and unused types optionally tree-shaken.

use std::collections::{HashMap, HashSet};
use crate::types::schema::{
    OdinSchemaDefinition, SchemaArray, SchemaField, SchemaFieldType, SchemaMetadata, SchemaObjectConstraint, SchemaType,
};
use super::ResolvedSchema;

// ─────────────────────────────────────────────────────────────────────────────
// Options & Result Types
// ─────────────────────────────────────────────────────────────────────────────

/// How to handle type name conflicts when merging imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum ConflictResolution {
    /// Prefix imported types with their namespace (default).
    #[default]
    Namespace,
    /// Later definitions overwrite earlier ones.
    Overwrite,
    /// Return an error on conflict.
    Error,
}


/// Options for flattening a schema.
#[derive(Debug, Clone)]
pub struct FlattenerOptions {
    /// How to handle type name conflicts (default: Namespace).
    pub conflict_resolution: ConflictResolution,
    /// Whether to tree-shake unused types (default: true).
    pub tree_shake: bool,
    /// Whether to inline type references (default: false).
    pub inline_type_references: bool,
    /// Custom metadata to override the primary schema's metadata.
    pub metadata: Option<SchemaMetadata>,
}

impl Default for FlattenerOptions {
    fn default() -> Self {
        Self {
            conflict_resolution: ConflictResolution::Namespace,
            tree_shake: true,
            inline_type_references: false,
            metadata: None,
        }
    }
}

/// Result of flattening a schema.
#[derive(Debug, Clone)]
pub struct FlattenedResult {
    /// The flattened schema with all imports merged and no import directives.
    pub schema: OdinSchemaDefinition,
    /// All source files that were merged.
    pub source_files: Vec<String>,
    /// Warnings generated during flattening.
    pub warnings: Vec<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// SchemaFlattener
// ─────────────────────────────────────────────────────────────────────────────

/// Flattens ODIN schemas by resolving and merging all imports into a single
/// schema with no dependencies.
pub struct SchemaFlattener {
    options: FlattenerOptions,
    warnings: Vec<String>,
    /// Maps type names to their source namespace (for reference rewriting).
    type_source_map: HashMap<String, Option<String>>,
    /// Set of referenced type names (qualified) for tree shaking.
    referenced_types: HashSet<String>,
}

impl SchemaFlattener {
    /// Create a new flattener with the given options.
    pub fn new(options: FlattenerOptions) -> Self {
        Self {
            options,
            warnings: Vec::new(),
            type_source_map: HashMap::new(),
            referenced_types: HashSet::new(),
        }
    }

    /// Flatten an already-resolved schema.
    pub fn flatten_resolved(&mut self, resolved: &ResolvedSchema) -> FlattenedResult {
        self.warnings.clear();
        self.type_source_map.clear();
        self.referenced_types.clear();

        // 1. Build type source map
        self.build_type_source_map(resolved);

        // 2. Merge all types from imports
        let mut merged_types = self.merge_types(resolved);

        // 3. Expand type inheritance (_composition / parents)
        merged_types = self.expand_type_inheritance(&merged_types);

        // 4. Merge fields, arrays, constraints
        let mut merged_fields = self.merge_fields(resolved);
        let mut merged_arrays = self.merge_arrays(resolved);
        let mut merged_constraints = self.merge_constraints(resolved);

        // 5. Tree shake if enabled
        if self.options.tree_shake {
            self.collect_referenced_types(
                resolved,
                &merged_types,
                &merged_fields,
                &merged_arrays,
            );

            let original_count = merged_types.len();
            merged_types = self.filter_referenced_types(&merged_types);
            let removed = original_count - merged_types.len();
            if removed > 0 {
                self.warnings.push(format!("Tree shaking removed {removed} unused types"));
            }

            merged_fields = self.filter_referenced_fields(&merged_fields);
            merged_arrays = self.filter_referenced_arrays(&merged_arrays);
            merged_constraints = self.filter_referenced_constraints(&merged_constraints);
        }

        // 6. Build flattened schema
        let metadata = if let Some(ref custom) = self.options.metadata {
            custom.clone()
        } else {
            resolved.schema.metadata.clone()
        };

        let schema = OdinSchemaDefinition {
            metadata,
            imports: Vec::new(), // No imports in flattened schema
            types: merged_types,
            fields: merged_fields,
            arrays: merged_arrays,
            constraints: merged_constraints,
        };

        FlattenedResult {
            schema,
            source_files: resolved.resolved_paths.clone(),
            warnings: std::mem::take(&mut self.warnings),
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Qualified Naming
    // ─────────────────────────────────────────────────────────────────────────

    /// Build a qualified name for a type, avoiding duplication when namespace
    /// overlaps with the type name.
    ///
    /// Examples:
    /// - `("agency", Some("agency"))` → `"agency"`
    /// - `("carrier_program", Some("carrier"))` → `"carrier_program"`
    /// - `("address", Some("types"))` → `"types_address"`
    /// - `("policy.named_insured", Some("policy"))` → `"policy.named_insured"`
    fn build_qualified_name(&self, type_name: &str, namespace: Option<&str>) -> String {
        let Some(ns) = namespace else { return type_name.to_string() };

        // Avoid duplication: namespace == type name
        if ns == type_name {
            return type_name.to_string();
        }

        // Avoid duplication: type name starts with "namespace."
        if type_name.starts_with(&format!("{ns}.")) {
            return type_name.to_string();
        }

        // Avoid duplication: type name starts with "namespace_"
        if type_name.starts_with(&format!("{ns}_")) {
            return type_name.to_string();
        }

        format!("{ns}_{type_name}")
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Type Source Map
    // ─────────────────────────────────────────────────────────────────────────

    fn build_type_source_map(&mut self, resolved: &ResolvedSchema) {
        // Add types from imports with their namespace
        for imp in &resolved.imports {
            if let Some(ref schema) = imp.schema {
                for type_name in schema.types.keys() {
                    self.type_source_map.insert(type_name.clone(), imp.alias.clone());
                }
            }
        }

        // Add types from primary schema (no namespace) — overrides imports
        for type_name in resolved.schema.types.keys() {
            self.type_source_map.insert(type_name.clone(), None);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Merge Types
    // ─────────────────────────────────────────────────────────────────────────

    fn merge_types(&mut self, resolved: &ResolvedSchema) -> HashMap<String, SchemaType> {
        let mut merged = HashMap::new();

        // First, add all types from imports
        for imp in &resolved.imports {
            if let Some(ref schema) = imp.schema {
                for (type_name, schema_type) in &schema.types {
                    self.add_type(&mut merged, type_name, schema_type, imp.alias.as_deref());
                }
            }
        }

        // Then add types from primary schema (may override imports)
        for (type_name, schema_type) in &resolved.schema.types {
            self.add_type(&mut merged, type_name, schema_type, None);
        }

        merged
    }

    fn add_type(
        &mut self,
        merged: &mut HashMap<String, SchemaType>,
        type_name: &str,
        schema_type: &SchemaType,
        namespace: Option<&str>,
    ) {
        let qualified_name = self.build_qualified_name(type_name, namespace);

        // Conflict handling
        if merged.contains_key(&qualified_name) {
            match self.options.conflict_resolution {
                ConflictResolution::Error => {
                    self.warnings.push(format!("Type name conflict: {qualified_name}"));
                    return; // In Rust we collect as warning; caller can check
                }
                ConflictResolution::Overwrite => {
                    self.warnings.push(format!("Type '{qualified_name}' overwritten"));
                }
                ConflictResolution::Namespace => {
                    if namespace.is_some() {
                        self.warnings.push(format!(
                            "Type '{}' from namespace '{}' conflicts with existing type",
                            qualified_name,
                            namespace.unwrap_or("")
                        ));
                    }
                }
            }
        }

        // Rewrite type references in fields
        let updated_fields: Vec<SchemaField> = schema_type.fields.iter().map(|f| {
            self.update_field_references(f, namespace)
        }).collect();

        let final_name = if self.options.conflict_resolution == ConflictResolution::Namespace {
            qualified_name.clone()
        } else {
            type_name.to_string()
        };

        merged.insert(qualified_name, SchemaType {
            name: final_name,
            description: schema_type.description.clone(),
            fields: updated_fields,
            parents: schema_type.parents.clone(),
        });
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Type Inheritance Expansion
    // ─────────────────────────────────────────────────────────────────────────

    fn expand_type_inheritance(
        &mut self,
        types: &HashMap<String, SchemaType>,
    ) -> HashMap<String, SchemaType> {
        let mut expanded = HashMap::new();
        let mut visited = HashSet::new();

        let type_names: Vec<String> = types.keys().cloned().collect();
        for type_name in &type_names {
            let schema_type = &types[type_name];
            let expanded_type = self.expand_single_type(
                type_name,
                schema_type,
                types,
                &mut visited,
            );
            expanded.insert(type_name.clone(), expanded_type);
        }

        expanded
    }

    fn expand_single_type(
        &mut self,
        type_name: &str,
        schema_type: &SchemaType,
        all_types: &HashMap<String, SchemaType>,
        visited: &mut HashSet<String>,
    ) -> SchemaType {
        // No parents means no inheritance
        if schema_type.parents.is_empty() {
            return schema_type.clone();
        }

        // Circular check
        if visited.contains(type_name) {
            self.warnings.push(format!(
                "Circular type inheritance detected for '{type_name}'"
            ));
            return schema_type.clone();
        }
        visited.insert(type_name.to_string());

        // Collect fields from all parent types
        let mut merged_fields: Vec<SchemaField> = Vec::new();
        let mut merged_field_names: HashSet<String> = HashSet::new();

        for parent_name in &schema_type.parents {
            let qualified_parent = self.resolve_type_name(parent_name);
            if let Some(parent_type) = all_types.get(&qualified_parent) {
                let expanded_parent = self.expand_single_type(
                    &qualified_parent,
                    parent_type,
                    all_types,
                    visited,
                );
                for field in &expanded_parent.fields {
                    if !merged_field_names.contains(&field.name) {
                        merged_field_names.insert(field.name.clone());
                        merged_fields.push(field.clone());
                    }
                }
            }
        }

        // Add/override with local fields
        for field in &schema_type.fields {
            if merged_field_names.contains(&field.name) {
                // Override: remove old, add new
                self.warnings.push(format!(
                    "Field '{}' in type '{}' overrides base type field",
                    field.name, type_name
                ));
                merged_fields.retain(|f| f.name != field.name);
            }
            merged_field_names.insert(field.name.clone());
            merged_fields.push(field.clone());
        }

        visited.remove(type_name);

        SchemaType {
            name: schema_type.name.clone(),
            description: schema_type.description.clone(),
            fields: merged_fields,
            parents: schema_type.parents.clone(),
        }
    }

    fn resolve_type_name(&self, type_name: &str) -> String {
        if type_name.contains('.') {
            // Already namespaced: convert dots to underscores
            let parts: Vec<&str> = type_name.rsplitn(2, '.').collect();
            if parts.len() == 2 {
                let namespace = parts[1].replace('.', "_");
                let name = parts[0];
                return self.build_qualified_name(name, Some(&namespace));
            }
        }
        // Simple name: look up in type source map
        let namespace = self.type_source_map.get(type_name).cloned().flatten();
        self.build_qualified_name(type_name, namespace.as_deref())
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Reference Rewriting
    // ─────────────────────────────────────────────────────────────────────────

    fn update_field_references(
        &self,
        field: &SchemaField,
        _source_namespace: Option<&str>,
    ) -> SchemaField {
        let mut updated = field.clone();

        match &field.field_type {
            SchemaFieldType::Reference(target_path) => {
                updated.field_type = SchemaFieldType::Reference(
                    self.rewrite_type_reference(target_path),
                );
            }
            SchemaFieldType::TypeRef(type_name) => {
                updated.field_type = SchemaFieldType::TypeRef(
                    self.rewrite_type_reference(type_name),
                );
            }
            _ => {}
        }

        updated
    }

    fn rewrite_type_reference(&self, name: &str) -> String {
        if name.contains('.') {
            // Namespaced reference: "types.address" → "types_address"
            let parts: Vec<&str> = name.rsplitn(2, '.').collect();
            if parts.len() == 2 {
                let namespace = parts[1].replace('.', "_");
                let type_part = parts[0];
                return self.build_qualified_name(type_part, Some(&namespace));
            }
        }

        if self.options.conflict_resolution == ConflictResolution::Namespace {
            // Simple reference: look up where the type is actually defined
            if let Some(Some(ns)) = self.type_source_map.get(name) {
                return self.build_qualified_name(name, Some(ns));
            }
        }

        // Not found in map, leave as-is (might be a local type)
        name.to_string()
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Merge Fields / Arrays / Constraints
    // ─────────────────────────────────────────────────────────────────────────

    fn merge_fields(&self, resolved: &ResolvedSchema) -> HashMap<String, SchemaField> {
        let mut merged = HashMap::new();

        // Add fields from imports
        for imp in &resolved.imports {
            if let Some(ref schema) = imp.schema {
                for (path, field) in &schema.fields {
                    let qualified_path = if self.options.conflict_resolution == ConflictResolution::Namespace {
                        if let Some(ref alias) = imp.alias {
                            format!("{alias}_{path}")
                        } else {
                            path.clone()
                        }
                    } else {
                        path.clone()
                    };

                    let updated = self.update_field_references(field, imp.alias.as_deref());
                    merged.insert(qualified_path, updated);
                }
            }
        }

        // Add fields from primary schema (overrides imports)
        for (path, field) in &resolved.schema.fields {
            let updated = self.update_field_references(field, None);
            merged.insert(path.clone(), updated);
        }

        merged
    }

    fn merge_arrays(&self, resolved: &ResolvedSchema) -> HashMap<String, SchemaArray> {
        let mut merged = HashMap::new();

        for imp in &resolved.imports {
            if let Some(ref schema) = imp.schema {
                for (path, array) in &schema.arrays {
                    let qualified_path = if self.options.conflict_resolution == ConflictResolution::Namespace {
                        if let Some(ref alias) = imp.alias {
                            format!("{alias}_{path}")
                        } else {
                            path.clone()
                        }
                    } else {
                        path.clone()
                    };

                    merged.insert(qualified_path.clone(), SchemaArray {
                        name: qualified_path,
                        ..array.clone()
                    });
                }
            }
        }

        for (path, array) in &resolved.schema.arrays {
            merged.insert(path.clone(), array.clone());
        }

        merged
    }

    fn merge_constraints(
        &self,
        resolved: &ResolvedSchema,
    ) -> HashMap<String, Vec<SchemaObjectConstraint>> {
        let mut merged: HashMap<String, Vec<SchemaObjectConstraint>> = HashMap::new();

        for imp in &resolved.imports {
            if let Some(ref schema) = imp.schema {
                for (path, constraints) in &schema.constraints {
                    let qualified_path = if self.options.conflict_resolution == ConflictResolution::Namespace {
                        if let Some(ref alias) = imp.alias {
                            format!("{alias}_{path}")
                        } else {
                            path.clone()
                        }
                    } else {
                        path.clone()
                    };

                    merged.entry(qualified_path)
                        .or_default()
                        .extend(constraints.iter().cloned());
                }
            }
        }

        for (path, constraints) in &resolved.schema.constraints {
            merged.entry(path.clone())
                .or_default()
                .extend(constraints.iter().cloned());
        }

        merged
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Tree Shaking
    // ─────────────────────────────────────────────────────────────────────────

    fn collect_referenced_types(
        &mut self,
        resolved: &ResolvedSchema,
        all_types: &HashMap<String, SchemaType>,
        merged_fields: &HashMap<String, SchemaField>,
        merged_arrays: &HashMap<String, SchemaArray>,
    ) {
        let mut processed_type_paths = HashSet::new();

        // Start with all types defined in the primary schema
        for type_name in resolved.schema.types.keys() {
            self.mark_type_referenced(
                type_name,
                None,
                all_types,
                merged_fields,
                merged_arrays,
                &mut processed_type_paths,
            );
        }

        // Also include types referenced from primary schema fields
        let field_refs: Vec<SchemaField> = resolved.schema.fields.values().cloned().collect();
        for field in &field_refs {
            self.collect_type_refs_from_field(
                field,
                all_types,
                merged_fields,
                merged_arrays,
                &mut processed_type_paths,
            );
        }

        // And from primary schema arrays
        let arrays: Vec<SchemaArray> = resolved.schema.arrays.values().cloned().collect();
        for array in &arrays {
            // Arrays don't have item_fields in our Rust model, but the item_type
            // may reference a type
            self.collect_type_refs_from_field_type(
                &array.item_type,
                all_types,
                merged_fields,
                merged_arrays,
                &mut processed_type_paths,
            );
        }
    }

    fn mark_type_referenced(
        &mut self,
        type_name: &str,
        namespace: Option<&str>,
        all_types: &HashMap<String, SchemaType>,
        merged_fields: &HashMap<String, SchemaField>,
        merged_arrays: &HashMap<String, SchemaArray>,
        processed_type_paths: &mut HashSet<String>,
    ) {
        let qualified_name = self.build_qualified_name(type_name, namespace);

        if self.referenced_types.contains(&qualified_name) {
            return;
        }

        self.referenced_types.insert(qualified_name.clone());

        // Find the type and recursively mark types it references
        if let Some(schema_type) = all_types.get(&qualified_name) {
            let fields: Vec<SchemaField> = schema_type.fields.clone();
            for field in &fields {
                self.collect_type_refs_from_field(
                    field,
                    all_types,
                    merged_fields,
                    merged_arrays,
                    processed_type_paths,
                );
            }

            // If this type inherits, process base types
            let parents: Vec<String> = schema_type.parents.clone();
            for parent in &parents {
                self.process_inherited_field_sections(
                    parent,
                    all_types,
                    merged_fields,
                    merged_arrays,
                    processed_type_paths,
                );
            }
        }

        // Process field sections belonging to this type path
        self.process_field_sections_for_type(
            &qualified_name,
            all_types,
            merged_fields,
            merged_arrays,
            processed_type_paths,
        );
    }

    fn process_inherited_field_sections(
        &mut self,
        base_type_name: &str,
        all_types: &HashMap<String, SchemaType>,
        merged_fields: &HashMap<String, SchemaField>,
        merged_arrays: &HashMap<String, SchemaArray>,
        processed_type_paths: &mut HashSet<String>,
    ) {
        let mut qualified = self.resolve_type_name(base_type_name);

        // Try multiple name formats
        if !all_types.contains_key(&qualified) {
            if base_type_name.contains('_') {
                qualified = base_type_name.to_string();
            }
            if !all_types.contains_key(&qualified) {
                qualified = base_type_name.to_string();
            }
        }

        // Mark the base type as referenced
        if !self.referenced_types.contains(&qualified) {
            self.referenced_types.insert(qualified.clone());
            if let Some(base_type) = all_types.get(&qualified).cloned() {
                let fields = base_type.fields.clone();
                for field in &fields {
                    self.collect_type_refs_from_field(
                        field,
                        all_types,
                        merged_fields,
                        merged_arrays,
                        processed_type_paths,
                    );
                }
                // Recursively check if base type also inherits
                for parent in &base_type.parents {
                    self.process_inherited_field_sections(
                        parent,
                        all_types,
                        merged_fields,
                        merged_arrays,
                        processed_type_paths,
                    );
                }
            }
        }

        self.process_field_sections_for_type(
            &qualified,
            all_types,
            merged_fields,
            merged_arrays,
            processed_type_paths,
        );
    }

    fn process_field_sections_for_type(
        &mut self,
        type_path: &str,
        all_types: &HashMap<String, SchemaType>,
        merged_fields: &HashMap<String, SchemaField>,
        merged_arrays: &HashMap<String, SchemaArray>,
        processed_type_paths: &mut HashSet<String>,
    ) {
        if processed_type_paths.contains(type_path) {
            return;
        }
        processed_type_paths.insert(type_path.to_string());

        let prefix = format!("{type_path}.");

        // Find nested types that start with this type path
        let nested_type_names: Vec<String> = all_types.keys()
            .filter(|n| n.starts_with(&prefix))
            .cloned()
            .collect();

        for nested_name in nested_type_names {
            if !self.referenced_types.contains(&nested_name) {
                self.referenced_types.insert(nested_name.clone());
                if let Some(nested_type) = all_types.get(&nested_name).cloned() {
                    let fields = nested_type.fields.clone();
                    for field in &fields {
                        self.collect_type_refs_from_field(
                            field,
                            all_types,
                            merged_fields,
                            merged_arrays,
                            processed_type_paths,
                        );
                    }
                    // Check inheritance
                    for parent in &nested_type.parents {
                        self.process_inherited_field_sections(
                            parent,
                            all_types,
                            merged_fields,
                            merged_arrays,
                            processed_type_paths,
                        );
                    }
                    // Recursively process nested
                    self.process_field_sections_for_type(
                        &nested_name,
                        all_types,
                        merged_fields,
                        merged_arrays,
                        processed_type_paths,
                    );
                }
            }
        }

        // Find field paths starting with this type path
        let field_paths: Vec<(String, SchemaField)> = merged_fields.iter()
            .filter(|(p, _)| p.starts_with(&prefix) || p.as_str() == type_path)
            .map(|(p, f)| (p.clone(), f.clone()))
            .collect();

        for (_path, field) in &field_paths {
            self.collect_type_refs_from_field(
                field,
                all_types,
                merged_fields,
                merged_arrays,
                processed_type_paths,
            );
        }

        // Check arrays
        let array_paths: Vec<(String, SchemaArray)> = merged_arrays.iter()
            .filter(|(p, _)| p.starts_with(&prefix) || p.as_str() == type_path)
            .map(|(p, a)| (p.clone(), a.clone()))
            .collect();

        for (_path, array) in &array_paths {
            self.collect_type_refs_from_field_type(
                &array.item_type,
                all_types,
                merged_fields,
                merged_arrays,
                processed_type_paths,
            );
        }
    }

    fn collect_type_refs_from_field(
        &mut self,
        field: &SchemaField,
        all_types: &HashMap<String, SchemaType>,
        merged_fields: &HashMap<String, SchemaField>,
        merged_arrays: &HashMap<String, SchemaArray>,
        processed_type_paths: &mut HashSet<String>,
    ) {
        self.collect_type_refs_from_field_type(
            &field.field_type,
            all_types,
            merged_fields,
            merged_arrays,
            processed_type_paths,
        );
    }

    fn collect_type_refs_from_field_type(
        &mut self,
        field_type: &SchemaFieldType,
        all_types: &HashMap<String, SchemaType>,
        merged_fields: &HashMap<String, SchemaField>,
        merged_arrays: &HashMap<String, SchemaArray>,
        processed_type_paths: &mut HashSet<String>,
    ) {
        match field_type {
            SchemaFieldType::Reference(target) | SchemaFieldType::TypeRef(target) => {
                let mut qualified = self.resolve_type_ref_name(target);

                // Try underscore format if not found
                if !all_types.contains_key(&qualified) && target.contains('_') {
                    qualified.clone_from(target);
                }

                if !self.referenced_types.contains(&qualified) {
                    self.referenced_types.insert(qualified.clone());
                    if let Some(ref_type) = all_types.get(&qualified).cloned() {
                        let fields = ref_type.fields.clone();
                        for f in &fields {
                            self.collect_type_refs_from_field(
                                f,
                                all_types,
                                merged_fields,
                                merged_arrays,
                                processed_type_paths,
                            );
                        }
                        // Check inheritance
                        for parent in &ref_type.parents {
                            self.process_inherited_field_sections(
                                parent,
                                all_types,
                                merged_fields,
                                merged_arrays,
                                processed_type_paths,
                            );
                        }
                    }
                    self.process_field_sections_for_type(
                        &qualified,
                        all_types,
                        merged_fields,
                        merged_arrays,
                        processed_type_paths,
                    );
                }
            }
            SchemaFieldType::Union(members) => {
                for member in members {
                    self.collect_type_refs_from_field_type(
                        member,
                        all_types,
                        merged_fields,
                        merged_arrays,
                        processed_type_paths,
                    );
                }
            }
            _ => {}
        }
    }

    fn resolve_type_ref_name(&self, name: &str) -> String {
        if name.contains('.') {
            let parts: Vec<&str> = name.rsplitn(2, '.').collect();
            if parts.len() == 2 {
                let namespace = parts[1].replace('.', "_");
                let type_part = parts[0];
                return self.build_qualified_name(type_part, Some(&namespace));
            }
        }
        // Simple name
        let namespace = self.type_source_map.get(name).cloned().flatten();
        self.build_qualified_name(name, namespace.as_deref())
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Tree Shaking Filters
    // ─────────────────────────────────────────────────────────────────────────

    fn filter_referenced_types(
        &self,
        types: &HashMap<String, SchemaType>,
    ) -> HashMap<String, SchemaType> {
        types.iter()
            .filter(|(name, _)| self.referenced_types.contains(*name))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn filter_referenced_fields(
        &self,
        fields: &HashMap<String, SchemaField>,
    ) -> HashMap<String, SchemaField> {
        fields.iter()
            .filter(|(path, _)| self.is_type_path_referenced(path))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn filter_referenced_arrays(
        &self,
        arrays: &HashMap<String, SchemaArray>,
    ) -> HashMap<String, SchemaArray> {
        arrays.iter()
            .filter(|(path, _)| self.is_type_path_referenced(path))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn filter_referenced_constraints(
        &self,
        constraints: &HashMap<String, Vec<SchemaObjectConstraint>>,
    ) -> HashMap<String, Vec<SchemaObjectConstraint>> {
        constraints.iter()
            .filter(|(path, _)| self.is_type_path_referenced(path))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Extract the type path from a field path (first segment before any dot).
    fn get_type_path_from_field_path(field_path: &str) -> &str {
        match field_path.find('.') {
            Some(idx) => &field_path[..idx],
            None => field_path,
        }
    }

    /// Check if a type path or any parent is referenced.
    fn is_type_path_referenced(&self, path: &str) -> bool {
        let type_path = Self::get_type_path_from_field_path(path);
        if self.referenced_types.contains(type_path) {
            return true;
        }
        // Primary schema fields don't have a type prefix
        if !type_path.contains('_') {
            return true;
        }
        false
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Convenience Functions
// ─────────────────────────────────────────────────────────────────────────────

/// Flatten a resolved schema using default options.
pub fn flatten_schema(resolved: &ResolvedSchema) -> FlattenedResult {
    let mut flattener = SchemaFlattener::new(FlattenerOptions::default());
    flattener.flatten_resolved(resolved)
}

/// Flatten a resolved schema and serialize to ODIN text.
pub fn bundle_schema(
    resolved: &ResolvedSchema,
    options: FlattenerOptions,
) -> (String, Vec<String>) {
    let mut flattener = SchemaFlattener::new(options);
    let result = flattener.flatten_resolved(resolved);
    let text = crate::validator::schema_serializer::serialize_schema(&result.schema);
    (text, result.warnings)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_schema_type(name: &str, fields: Vec<SchemaField>) -> SchemaType {
        SchemaType {
            name: name.to_string(),
            description: None,
            fields,
            parents: vec![],
        }
    }

    fn make_field(name: &str, field_type: SchemaFieldType) -> SchemaField {
        SchemaField {
            name: name.to_string(),
            field_type,
            required: false,
            confidential: false,
            deprecated: false,
            immutable: false,
            description: None,
            constraints: vec![],
            default_value: None,
            conditionals: vec![],
        }
    }

    fn make_empty_resolved() -> ResolvedSchema {
        ResolvedSchema {
            schema: OdinSchemaDefinition {
                metadata: SchemaMetadata::default(),
                imports: vec![],
                types: HashMap::new(),
                fields: HashMap::new(),
                arrays: HashMap::new(),
                constraints: HashMap::new(),
            },
            resolved_paths: vec![],
            type_registry: super::super::TypeRegistry::new(),
            imports: vec![],
        }
    }

    #[test]
    fn test_flatten_no_imports() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Person".to_string(),
            make_schema_type("Person", vec![
                make_field("name", SchemaFieldType::String),
            ]),
        );

        let result = flatten_schema(&resolved);
        assert!(result.schema.imports.is_empty());
        assert!(result.schema.types.contains_key("Person"));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_flatten_empty_imports_result() {
        let resolved = make_empty_resolved();
        let result = flatten_schema(&resolved);
        assert!(result.schema.imports.is_empty());
    }

    #[test]
    fn test_flatten_merges_imported_types_with_namespace() {
        let mut resolved = make_empty_resolved();

        // Primary schema has one type that references the imported type
        resolved.schema.types.insert(
            "Policy".to_string(),
            make_schema_type("Policy", vec![
                make_field("number", SchemaFieldType::String),
                make_field("addr", SchemaFieldType::TypeRef("Address".to_string())),
            ]),
        );

        // Import has another type
        let mut imported_schema = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported_schema.types.insert(
            "Address".to_string(),
            make_schema_type("Address", vec![
                make_field("line1", SchemaFieldType::String),
            ]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/types.odin".to_string(),
            alias: Some("types".to_string()),
            schema: Some(imported_schema),
        });

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("Policy"));
        assert!(result.schema.types.contains_key("types_Address"));
        assert!(!result.schema.types.contains_key("Address")); // Should be namespaced
    }

    #[test]
    fn test_qualified_name_avoids_duplication() {
        let flattener = SchemaFlattener::new(FlattenerOptions::default());

        // namespace == type name
        assert_eq!(flattener.build_qualified_name("agency", Some("agency")), "agency");

        // type starts with namespace_
        assert_eq!(
            flattener.build_qualified_name("carrier_program", Some("carrier")),
            "carrier_program"
        );

        // type starts with namespace.
        assert_eq!(
            flattener.build_qualified_name("policy.named_insured", Some("policy")),
            "policy.named_insured"
        );

        // normal case
        assert_eq!(
            flattener.build_qualified_name("address", Some("types")),
            "types_address"
        );

        // no namespace
        assert_eq!(flattener.build_qualified_name("coverage", None), "coverage");
    }

    #[test]
    fn test_tree_shaking_removes_unused() {
        let mut resolved = make_empty_resolved();

        // Primary schema has a type that references ImportedUsed
        resolved.schema.types.insert(
            "Policy".to_string(),
            make_schema_type("Policy", vec![
                make_field("addr", SchemaFieldType::TypeRef("types_Address".to_string())),
            ]),
        );

        // Import has two types: Address (referenced) and Unused (not referenced)
        let mut imported_schema = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported_schema.types.insert(
            "Address".to_string(),
            make_schema_type("Address", vec![
                make_field("line1", SchemaFieldType::String),
            ]),
        );
        imported_schema.types.insert(
            "Unused".to_string(),
            make_schema_type("Unused", vec![
                make_field("x", SchemaFieldType::Integer),
            ]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/types.odin".to_string(),
            alias: Some("types".to_string()),
            schema: Some(imported_schema),
        });

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("Policy"));
        assert!(result.schema.types.contains_key("types_Address"));
        assert!(!result.schema.types.contains_key("types_Unused"));
        assert!(result.warnings.iter().any(|w| w.contains("Tree shaking removed")));
    }

    #[test]
    fn test_tree_shaking_preserves_transitive_refs() {
        let mut resolved = make_empty_resolved();

        // Policy -> types_Address -> types_Country (transitive chain)
        resolved.schema.types.insert(
            "Policy".to_string(),
            make_schema_type("Policy", vec![
                make_field("addr", SchemaFieldType::TypeRef("types_Address".to_string())),
            ]),
        );

        let mut imported_schema = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported_schema.types.insert(
            "Address".to_string(),
            make_schema_type("Address", vec![
                make_field("country", SchemaFieldType::TypeRef("types_Country".to_string())),
            ]),
        );
        imported_schema.types.insert(
            "Country".to_string(),
            make_schema_type("Country", vec![
                make_field("code", SchemaFieldType::String),
            ]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/types.odin".to_string(),
            alias: Some("types".to_string()),
            schema: Some(imported_schema),
        });

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("types_Address"));
        assert!(result.schema.types.contains_key("types_Country"));
    }

    #[test]
    fn test_no_tree_shaking_keeps_all() {
        let mut resolved = make_empty_resolved();

        resolved.schema.types.insert(
            "Policy".to_string(),
            make_schema_type("Policy", vec![]),
        );

        let mut imported_schema = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported_schema.types.insert(
            "Unused".to_string(),
            make_schema_type("Unused", vec![]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/types.odin".to_string(),
            alias: Some("types".to_string()),
            schema: Some(imported_schema),
        });

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            tree_shake: false,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        assert!(result.schema.types.contains_key("types_Unused"));
    }

    #[test]
    fn test_type_inheritance_expansion() {
        let mut resolved = make_empty_resolved();

        // Base type
        resolved.schema.types.insert(
            "Base".to_string(),
            make_schema_type("Base", vec![
                make_field("id", SchemaFieldType::Integer),
                make_field("created", SchemaFieldType::Timestamp),
            ]),
        );

        // Child type inheriting from Base
        let mut child = make_schema_type("Child", vec![
            make_field("name", SchemaFieldType::String),
        ]);
        child.parents = vec!["Base".to_string()];
        resolved.schema.types.insert("Child".to_string(), child);

        let result = flatten_schema(&resolved);
        let child_type = result.schema.types.get("Child").unwrap();
        let field_names: Vec<&str> = child_type.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(field_names.contains(&"id"));
        assert!(field_names.contains(&"created"));
        assert!(field_names.contains(&"name"));
    }

    #[test]
    fn test_circular_inheritance_warning() {
        let mut resolved = make_empty_resolved();

        let mut type_a = make_schema_type("A", vec![]);
        type_a.parents = vec!["B".to_string()];
        let mut type_b = make_schema_type("B", vec![]);
        type_b.parents = vec!["A".to_string()];

        resolved.schema.types.insert("A".to_string(), type_a);
        resolved.schema.types.insert("B".to_string(), type_b);

        let result = flatten_schema(&resolved);
        assert!(result.warnings.iter().any(|w| w.contains("Circular type inheritance")));
    }

    #[test]
    fn test_conflict_resolution_overwrite() {
        let mut resolved = make_empty_resolved();

        resolved.schema.types.insert(
            "Address".to_string(),
            make_schema_type("Address", vec![
                make_field("line1", SchemaFieldType::String),
            ]),
        );

        let mut imported_schema = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported_schema.types.insert(
            "Address".to_string(),
            make_schema_type("Address", vec![
                make_field("street", SchemaFieldType::String),
            ]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/types.odin".to_string(),
            alias: Some("types".to_string()),
            schema: Some(imported_schema),
        });

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            conflict_resolution: ConflictResolution::Overwrite,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);

        // Primary schema's Address should win (added last)
        let addr = result.schema.types.get("Address").unwrap();
        assert!(addr.fields.iter().any(|f| f.name == "line1"));
    }

    #[test]
    fn test_bundle_schema() {
        let mut resolved = make_empty_resolved();
        resolved.schema.metadata.title = Some("Test".to_string());
        resolved.schema.types.insert(
            "Person".to_string(),
            make_schema_type("Person", vec![
                make_field("name", SchemaFieldType::String),
            ]),
        );

        let (text, warnings) = bundle_schema(&resolved, FlattenerOptions::default());
        assert!(text.contains("{$}"));
        assert!(text.contains("Person"));
        assert!(warnings.is_empty());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Empty / minimal schemas
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_flatten_completely_empty() {
        let resolved = make_empty_resolved();
        let result = flatten_schema(&resolved);
        assert!(result.schema.types.is_empty());
        assert!(result.schema.fields.is_empty());
        assert!(result.schema.arrays.is_empty());
        assert!(result.schema.constraints.is_empty());
        assert!(result.schema.imports.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_flatten_preserves_metadata() {
        let mut resolved = make_empty_resolved();
        resolved.schema.metadata.title = Some("My Schema".to_string());
        resolved.schema.metadata.version = Some("2.0".to_string());
        resolved.schema.metadata.description = Some("A description".to_string());

        let result = flatten_schema(&resolved);
        assert_eq!(result.schema.metadata.title.as_deref(), Some("My Schema"));
        assert_eq!(result.schema.metadata.version.as_deref(), Some("2.0"));
        assert_eq!(result.schema.metadata.description.as_deref(), Some("A description"));
    }

    #[test]
    fn test_flatten_custom_metadata_override() {
        let mut resolved = make_empty_resolved();
        resolved.schema.metadata.title = Some("Original".to_string());

        let custom_meta = SchemaMetadata {
            id: None,
            title: Some("Overridden".to_string()),
            description: None,
            version: Some("3.0".to_string()),
        };

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            metadata: Some(custom_meta),
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        assert_eq!(result.schema.metadata.title.as_deref(), Some("Overridden"));
        assert_eq!(result.schema.metadata.version.as_deref(), Some("3.0"));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Qualified naming
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_qualified_name_no_namespace() {
        let flattener = SchemaFlattener::new(FlattenerOptions::default());
        assert_eq!(flattener.build_qualified_name("MyType", None), "MyType");
    }

    #[test]
    fn test_qualified_name_namespace_equals_type() {
        let flattener = SchemaFlattener::new(FlattenerOptions::default());
        assert_eq!(flattener.build_qualified_name("policy", Some("policy")), "policy");
    }

    #[test]
    fn test_qualified_name_type_starts_with_namespace_underscore() {
        let flattener = SchemaFlattener::new(FlattenerOptions::default());
        assert_eq!(
            flattener.build_qualified_name("types_address", Some("types")),
            "types_address"
        );
    }

    #[test]
    fn test_qualified_name_type_starts_with_namespace_dot() {
        let flattener = SchemaFlattener::new(FlattenerOptions::default());
        assert_eq!(
            flattener.build_qualified_name("ns.sub_type", Some("ns")),
            "ns.sub_type"
        );
    }

    #[test]
    fn test_qualified_name_normal_prefixing() {
        let flattener = SchemaFlattener::new(FlattenerOptions::default());
        assert_eq!(
            flattener.build_qualified_name("Widget", Some("lib")),
            "lib_Widget"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Merge types from imports
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_merge_types_no_imports() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Solo".to_string(),
            make_schema_type("Solo", vec![make_field("x", SchemaFieldType::String)]),
        );

        let result = flatten_schema(&resolved);
        assert_eq!(result.schema.types.len(), 1);
        assert!(result.schema.types.contains_key("Solo"));
    }

    #[test]
    fn test_merge_types_from_multiple_imports() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Main".to_string(),
            make_schema_type("Main", vec![
                make_field("a_ref", SchemaFieldType::TypeRef("a_TypeA".to_string())),
                make_field("b_ref", SchemaFieldType::TypeRef("b_TypeB".to_string())),
            ]),
        );

        let mut schema_a = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        schema_a.types.insert(
            "TypeA".to_string(),
            make_schema_type("TypeA", vec![make_field("fa", SchemaFieldType::Integer)]),
        );

        let mut schema_b = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        schema_b.types.insert(
            "TypeB".to_string(),
            make_schema_type("TypeB", vec![make_field("fb", SchemaFieldType::Boolean)]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/a.odin".to_string(),
            alias: Some("a".to_string()),
            schema: Some(schema_a),
        });
        resolved.imports.push(super::super::ResolvedImport {
            path: "/b.odin".to_string(),
            alias: Some("b".to_string()),
            schema: Some(schema_b),
        });

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("Main"));
        assert!(result.schema.types.contains_key("a_TypeA"));
        assert!(result.schema.types.contains_key("b_TypeB"));
    }

    #[test]
    fn test_merge_types_import_no_alias() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Main".to_string(),
            make_schema_type("Main", vec![
                make_field("ref", SchemaFieldType::TypeRef("Imported".to_string())),
            ]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert(
            "Imported".to_string(),
            make_schema_type("Imported", vec![make_field("val", SchemaFieldType::String)]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/imp.odin".to_string(),
            alias: None,
            schema: Some(imported),
        });

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("Imported"));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Conflict resolution
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_conflict_resolution_error_mode() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Dup".to_string(),
            make_schema_type("Dup", vec![make_field("a", SchemaFieldType::String)]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert(
            "Dup".to_string(),
            make_schema_type("Dup", vec![make_field("b", SchemaFieldType::Integer)]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/imp.odin".to_string(),
            alias: None,
            schema: Some(imported),
        });

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            conflict_resolution: ConflictResolution::Error,
            tree_shake: false,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        assert!(result.warnings.iter().any(|w| w.contains("conflict")));
    }

    #[test]
    fn test_conflict_resolution_namespace_avoids_collision() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Address".to_string(),
            make_schema_type("Address", vec![make_field("local", SchemaFieldType::String)]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert(
            "Address".to_string(),
            make_schema_type("Address", vec![make_field("imported", SchemaFieldType::String)]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/types.odin".to_string(),
            alias: Some("ext".to_string()),
            schema: Some(imported),
        });

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            conflict_resolution: ConflictResolution::Namespace,
            tree_shake: false,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        assert!(result.schema.types.contains_key("Address"));
        assert!(result.schema.types.contains_key("ext_Address"));
    }

    #[test]
    fn test_inheritance_child_overrides_parent_field() {
        let mut resolved = make_empty_resolved();

        resolved.schema.types.insert(
            "Base".to_string(),
            make_schema_type("Base", vec![
                make_field("name", SchemaFieldType::String),
                make_field("age", SchemaFieldType::Integer),
            ]),
        );

        let mut child = make_schema_type("Child", vec![
            make_field("name", SchemaFieldType::String),
            make_field("email", SchemaFieldType::String),
        ]);
        child.parents = vec!["Base".to_string()];
        resolved.schema.types.insert("Child".to_string(), child);

        let result = flatten_schema(&resolved);
        let child_type = result.schema.types.get("Child").unwrap();
        let field_names: Vec<&str> = child_type.fields.iter().map(|f| f.name.as_str()).collect();

        assert!(field_names.contains(&"age"));
        assert!(field_names.contains(&"name"));
        assert!(field_names.contains(&"email"));
        assert_eq!(child_type.fields.len(), 3);
        assert!(result.warnings.iter().any(|w| w.contains("overrides")));
    }

    #[test]
    fn test_multi_level_inheritance() {
        let mut resolved = make_empty_resolved();

        resolved.schema.types.insert(
            "A".to_string(),
            make_schema_type("A", vec![make_field("a_field", SchemaFieldType::String)]),
        );

        let mut b = make_schema_type("B", vec![make_field("b_field", SchemaFieldType::Integer)]);
        b.parents = vec!["A".to_string()];
        resolved.schema.types.insert("B".to_string(), b);

        let mut c = make_schema_type("C", vec![make_field("c_field", SchemaFieldType::Boolean)]);
        c.parents = vec!["B".to_string()];
        resolved.schema.types.insert("C".to_string(), c);

        let result = flatten_schema(&resolved);
        let c_type = result.schema.types.get("C").unwrap();
        let field_names: Vec<&str> = c_type.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(field_names.contains(&"a_field"));
        assert!(field_names.contains(&"b_field"));
        assert!(field_names.contains(&"c_field"));
    }

    #[test]
    fn test_multiple_parents() {
        let mut resolved = make_empty_resolved();

        resolved.schema.types.insert(
            "Timestamped".to_string(),
            make_schema_type("Timestamped", vec![
                make_field("created_at", SchemaFieldType::Timestamp),
            ]),
        );
        resolved.schema.types.insert(
            "Named".to_string(),
            make_schema_type("Named", vec![
                make_field("name", SchemaFieldType::String),
            ]),
        );

        let mut entity = make_schema_type("Entity", vec![
            make_field("id", SchemaFieldType::Integer),
        ]);
        entity.parents = vec!["Timestamped".to_string(), "Named".to_string()];
        resolved.schema.types.insert("Entity".to_string(), entity);

        let result = flatten_schema(&resolved);
        let entity_type = result.schema.types.get("Entity").unwrap();
        let field_names: Vec<&str> = entity_type.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(field_names.contains(&"created_at"));
        assert!(field_names.contains(&"name"));
        assert!(field_names.contains(&"id"));
    }

    #[test]
    fn test_circular_inheritance_does_not_hang() {
        let mut resolved = make_empty_resolved();

        let mut x = make_schema_type("X", vec![make_field("fx", SchemaFieldType::String)]);
        x.parents = vec!["Y".to_string()];
        let mut y = make_schema_type("Y", vec![make_field("fy", SchemaFieldType::String)]);
        y.parents = vec!["X".to_string()];

        resolved.schema.types.insert("X".to_string(), x);
        resolved.schema.types.insert("Y".to_string(), y);

        let result = flatten_schema(&resolved);
        assert!(result.warnings.iter().any(|w| w.contains("Circular type inheritance")));
    }

    #[test]
    fn test_tree_shake_keeps_primary_types() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Standalone".to_string(),
            make_schema_type("Standalone", vec![make_field("x", SchemaFieldType::String)]),
        );

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("Standalone"));
    }

    #[test]
    fn test_tree_shake_removes_unreferenced_import() {
        let mut resolved = make_empty_resolved();

        resolved.schema.types.insert(
            "LocalOnly".to_string(),
            make_schema_type("LocalOnly", vec![make_field("f", SchemaFieldType::String)]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert(
            "Orphan".to_string(),
            make_schema_type("Orphan", vec![make_field("o", SchemaFieldType::Integer)]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/orphan.odin".to_string(),
            alias: Some("ext".to_string()),
            schema: Some(imported),
        });

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("LocalOnly"));
        assert!(!result.schema.types.contains_key("ext_Orphan"));
        assert!(result.warnings.iter().any(|w| w.contains("Tree shaking removed")));
    }

    #[test]
    fn test_tree_shake_preserves_chain_of_refs() {
        let mut resolved = make_empty_resolved();

        resolved.schema.types.insert(
            "A".to_string(),
            make_schema_type("A", vec![
                make_field("b", SchemaFieldType::TypeRef("lib_B".to_string())),
            ]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert(
            "B".to_string(),
            make_schema_type("B", vec![
                make_field("c", SchemaFieldType::TypeRef("lib_C".to_string())),
            ]),
        );
        imported.types.insert(
            "C".to_string(),
            make_schema_type("C", vec![make_field("val", SchemaFieldType::String)]),
        );
        imported.types.insert(
            "Unrelated".to_string(),
            make_schema_type("Unrelated", vec![make_field("u", SchemaFieldType::String)]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/lib.odin".to_string(),
            alias: Some("lib".to_string()),
            schema: Some(imported),
        });

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("A"));
        assert!(result.schema.types.contains_key("lib_B"));
        assert!(result.schema.types.contains_key("lib_C"));
        assert!(!result.schema.types.contains_key("lib_Unrelated"));
    }

    #[test]
    fn test_tree_shake_disabled_keeps_everything() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Used".to_string(),
            make_schema_type("Used", vec![]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert("A".to_string(), make_schema_type("A", vec![]));
        imported.types.insert("B".to_string(), make_schema_type("B", vec![]));
        imported.types.insert("C".to_string(), make_schema_type("C", vec![]));

        resolved.imports.push(super::super::ResolvedImport {
            path: "/lib.odin".to_string(),
            alias: Some("lib".to_string()),
            schema: Some(imported),
        });

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            tree_shake: false,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        assert!(result.schema.types.contains_key("lib_A"));
        assert!(result.schema.types.contains_key("lib_B"));
        assert!(result.schema.types.contains_key("lib_C"));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Reference rewriting
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_type_ref_rewritten_to_namespace() {
        let mut resolved = make_empty_resolved();

        resolved.schema.types.insert(
            "Policy".to_string(),
            make_schema_type("Policy", vec![
                make_field("addr", SchemaFieldType::TypeRef("types.Address".to_string())),
            ]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert(
            "Address".to_string(),
            make_schema_type("Address", vec![make_field("line", SchemaFieldType::String)]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/types.odin".to_string(),
            alias: Some("types".to_string()),
            schema: Some(imported),
        });

        let result = flatten_schema(&resolved);
        let policy = result.schema.types.get("Policy").unwrap();
        let addr_field = policy.fields.iter().find(|f| f.name == "addr").unwrap();
        match &addr_field.field_type {
            SchemaFieldType::TypeRef(name) => assert_eq!(name, "types_Address"),
            other => panic!("Expected TypeRef, got {:?}", other),
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Merge fields / arrays / constraints
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_merge_fields_from_import() {
        let mut resolved = make_empty_resolved();

        resolved.schema.fields.insert(
            "person.name".to_string(),
            make_field("name", SchemaFieldType::String),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.fields.insert(
            "addr.line1".to_string(),
            make_field("line1", SchemaFieldType::String),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/types.odin".to_string(),
            alias: Some("t".to_string()),
            schema: Some(imported),
        });

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            tree_shake: false,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        assert!(result.schema.fields.contains_key("person.name"));
        assert!(result.schema.fields.contains_key("t_addr.line1"));
    }

    #[test]
    fn test_merge_arrays_from_import() {
        let mut resolved = make_empty_resolved();

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.arrays.insert(
            "items".to_string(),
            SchemaArray {
                name: "items".to_string(),
                item_type: SchemaFieldType::String,
                min_items: Some(1),
                max_items: Some(10),
                unique: false,
            },
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/lib.odin".to_string(),
            alias: Some("lib".to_string()),
            schema: Some(imported),
        });

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            tree_shake: false,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        assert!(result.schema.arrays.contains_key("lib_items"));
        let arr = result.schema.arrays.get("lib_items").unwrap();
        assert_eq!(arr.min_items, Some(1));
        assert_eq!(arr.max_items, Some(10));
    }

    #[test]
    fn test_primary_fields_override_import_fields() {
        let mut resolved = make_empty_resolved();

        resolved.schema.fields.insert(
            "shared_path".to_string(),
            make_field("shared_path", SchemaFieldType::Integer),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.fields.insert(
            "shared_path".to_string(),
            make_field("shared_path", SchemaFieldType::String),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/imp.odin".to_string(),
            alias: None,
            schema: Some(imported),
        });

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            tree_shake: false,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        let field = result.schema.fields.get("shared_path").unwrap();
        assert!(matches!(field.field_type, SchemaFieldType::Integer));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Bundle schema
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_bundle_empty_schema() {
        let resolved = make_empty_resolved();
        let (text, warnings) = bundle_schema(&resolved, FlattenerOptions::default());
        assert!(text.contains("{$}"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_bundle_with_import_produces_no_import_directive() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Main".to_string(),
            make_schema_type("Main", vec![
                make_field("ref", SchemaFieldType::TypeRef("ext_Imported".to_string())),
            ]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert(
            "Imported".to_string(),
            make_schema_type("Imported", vec![make_field("v", SchemaFieldType::String)]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/ext.odin".to_string(),
            alias: Some("ext".to_string()),
            schema: Some(imported),
        });

        let (text, _warnings) = bundle_schema(&resolved, FlattenerOptions::default());
        assert!(!text.contains("@import"));
        assert!(text.contains("Main"));
        assert!(text.contains("ext_Imported") || text.contains("Imported"));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Source files tracking
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_source_files_populated() {
        let mut resolved = make_empty_resolved();
        resolved.resolved_paths = vec![
            "/a.odin".to_string(),
            "/b.odin".to_string(),
        ];

        let result = flatten_schema(&resolved);
        assert_eq!(result.source_files.len(), 2);
        assert_eq!(result.source_files[0], "/a.odin");
        assert_eq!(result.source_files[1], "/b.odin");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // FlattenerOptions defaults
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_flattener_options_defaults() {
        let opts = FlattenerOptions::default();
        assert_eq!(opts.conflict_resolution, ConflictResolution::Namespace);
        assert!(opts.tree_shake);
        assert!(!opts.inline_type_references);
        assert!(opts.metadata.is_none());
    }

    #[test]
    fn test_conflict_resolution_default() {
        let cr = ConflictResolution::default();
        assert_eq!(cr, ConflictResolution::Namespace);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Diamond import through flattener
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_flatten_diamond_import_no_duplicates() {
        let mut resolved = make_empty_resolved();

        resolved.schema.types.insert(
            "Root".to_string(),
            make_schema_type("Root", vec![
                make_field("a", SchemaFieldType::TypeRef("ns1_A".to_string())),
                make_field("b", SchemaFieldType::TypeRef("ns2_B".to_string())),
            ]),
        );

        let mut imp_a = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imp_a.types.insert("A".to_string(), make_schema_type("A", vec![]));

        let mut imp_b = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imp_b.types.insert("B".to_string(), make_schema_type("B", vec![]));

        resolved.imports.push(super::super::ResolvedImport {
            path: "/a.odin".to_string(),
            alias: Some("ns1".to_string()),
            schema: Some(imp_a),
        });
        resolved.imports.push(super::super::ResolvedImport {
            path: "/b.odin".to_string(),
            alias: Some("ns2".to_string()),
            schema: Some(imp_b),
        });

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("Root"));
        assert!(result.schema.types.contains_key("ns1_A"));
        assert!(result.schema.types.contains_key("ns2_B"));
        assert_eq!(result.schema.types.len(), 3);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Edge cases: import with no schema
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_import_with_no_schema_ignored() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Main".to_string(),
            make_schema_type("Main", vec![]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/doc.odin".to_string(),
            alias: Some("doc".to_string()),
            schema: None,
        });

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("Main"));
        assert_eq!(result.schema.types.len(), 1);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Additional flatten tests — field types, modifiers, edge cases
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_flatten_single_type_single_field() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Simple".to_string(),
            make_schema_type("Simple", vec![
                make_field("val", SchemaFieldType::Integer),
            ]),
        );
        let result = flatten_schema(&resolved);
        assert_eq!(result.schema.types.len(), 1);
        let t = result.schema.types.get("Simple").unwrap();
        assert_eq!(t.fields.len(), 1);
        assert_eq!(t.fields[0].name, "val");
    }

    #[test]
    fn test_flatten_preserves_field_types() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Multi".to_string(),
            make_schema_type("Multi", vec![
                make_field("s", SchemaFieldType::String),
                make_field("i", SchemaFieldType::Integer),
                make_field("b", SchemaFieldType::Boolean),
                make_field("n", SchemaFieldType::Null),
                make_field("d", SchemaFieldType::Date),
                make_field("ts", SchemaFieldType::Timestamp),
            ]),
        );
        let result = flatten_schema(&resolved);
        let t = result.schema.types.get("Multi").unwrap();
        assert_eq!(t.fields.len(), 6);
        assert!(matches!(t.fields[0].field_type, SchemaFieldType::String));
        assert!(matches!(t.fields[1].field_type, SchemaFieldType::Integer));
        assert!(matches!(t.fields[2].field_type, SchemaFieldType::Boolean));
    }

    #[test]
    fn test_flatten_preserves_field_modifiers() {
        let mut resolved = make_empty_resolved();
        let mut field = make_field("secret", SchemaFieldType::String);
        field.required = true;
        field.confidential = true;
        field.deprecated = true;
        resolved.schema.types.insert(
            "Flagged".to_string(),
            make_schema_type("Flagged", vec![field]),
        );

        let result = flatten_schema(&resolved);
        let t = result.schema.types.get("Flagged").unwrap();
        let f = &t.fields[0];
        assert!(f.required);
        assert!(f.confidential);
        assert!(f.deprecated);
    }

    #[test]
    fn test_flatten_multiple_primary_types() {
        let mut resolved = make_empty_resolved();
        for name in &["Alpha", "Beta", "Gamma"] {
            resolved.schema.types.insert(
                name.to_string(),
                make_schema_type(name, vec![make_field("f", SchemaFieldType::String)]),
            );
        }

        let result = flatten_schema(&resolved);
        assert_eq!(result.schema.types.len(), 3);
        assert!(result.schema.types.contains_key("Alpha"));
        assert!(result.schema.types.contains_key("Beta"));
        assert!(result.schema.types.contains_key("Gamma"));
    }

    #[test]
    fn test_flatten_import_with_empty_types() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Main".to_string(),
            make_schema_type("Main", vec![]),
        );

        let imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };

        resolved.imports.push(super::super::ResolvedImport {
            path: "/empty.odin".to_string(),
            alias: Some("emp".to_string()),
            schema: Some(imported),
        });

        let result = flatten_schema(&resolved);
        assert_eq!(result.schema.types.len(), 1);
        assert!(result.schema.types.contains_key("Main"));
    }

    #[test]
    fn test_flatten_type_description_preserved() {
        let mut resolved = make_empty_resolved();
        let mut st = make_schema_type("Described", vec![]);
        st.description = Some("A described type".to_string());
        resolved.schema.types.insert("Described".to_string(), st);

        let result = flatten_schema(&resolved);
        let t = result.schema.types.get("Described").unwrap();
        assert_eq!(t.description.as_deref(), Some("A described type"));
    }

    #[test]
    fn test_flatten_imported_type_description_preserved() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Main".to_string(),
            make_schema_type("Main", vec![
                make_field("ref", SchemaFieldType::TypeRef("lib_Ext".to_string())),
            ]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        let mut ext = make_schema_type("Ext", vec![make_field("v", SchemaFieldType::String)]);
        ext.description = Some("External type".to_string());
        imported.types.insert("Ext".to_string(), ext);

        resolved.imports.push(super::super::ResolvedImport {
            path: "/lib.odin".to_string(),
            alias: Some("lib".to_string()),
            schema: Some(imported),
        });

        let result = flatten_schema(&resolved);
        let t = result.schema.types.get("lib_Ext").unwrap();
        assert_eq!(t.description.as_deref(), Some("External type"));
    }

    #[test]
    fn test_flatten_three_imports_all_referenced() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Root".to_string(),
            make_schema_type("Root", vec![
                make_field("a", SchemaFieldType::TypeRef("ns1_A".to_string())),
                make_field("b", SchemaFieldType::TypeRef("ns2_B".to_string())),
                make_field("c", SchemaFieldType::TypeRef("ns3_C".to_string())),
            ]),
        );

        for (ns, name) in &[("ns1", "A"), ("ns2", "B"), ("ns3", "C")] {
            let mut imp = OdinSchemaDefinition {
                metadata: SchemaMetadata::default(),
                imports: vec![],
                types: HashMap::new(),
                fields: HashMap::new(),
                arrays: HashMap::new(),
                constraints: HashMap::new(),
            };
            imp.types.insert(name.to_string(), make_schema_type(name, vec![
                make_field("val", SchemaFieldType::String),
            ]));
            resolved.imports.push(super::super::ResolvedImport {
                path: format!("/{ns}.odin"),
                alias: Some(ns.to_string()),
                schema: Some(imp),
            });
        }

        let result = flatten_schema(&resolved);
        assert_eq!(result.schema.types.len(), 4);
        assert!(result.schema.types.contains_key("ns1_A"));
        assert!(result.schema.types.contains_key("ns2_B"));
        assert!(result.schema.types.contains_key("ns3_C"));
    }

    #[test]
    fn test_flatten_preserves_primary_arrays() {
        let mut resolved = make_empty_resolved();
        resolved.schema.arrays.insert(
            "tags".to_string(),
            SchemaArray {
                name: "tags".to_string(),
                item_type: SchemaFieldType::String,
                min_items: Some(0),
                max_items: Some(50),
                unique: true,
            },
        );

        let result = flatten_schema(&resolved);
        assert!(result.schema.arrays.contains_key("tags"));
        let arr = result.schema.arrays.get("tags").unwrap();
        assert_eq!(arr.min_items, Some(0));
        assert_eq!(arr.max_items, Some(50));
        assert!(arr.unique);
    }

    #[test]
    fn test_flatten_preserves_primary_fields() {
        let mut resolved = make_empty_resolved();
        let mut field = make_field("email", SchemaFieldType::String);
        field.required = true;
        resolved.schema.fields.insert("person.email".to_string(), field);

        let result = flatten_schema(&resolved);
        assert!(result.schema.fields.contains_key("person.email"));
        let f = result.schema.fields.get("person.email").unwrap();
        assert!(f.required);
    }

    #[test]
    fn test_flatten_metadata_id_preserved() {
        let mut resolved = make_empty_resolved();
        resolved.schema.metadata.id = Some("urn:test:schema:1".to_string());

        let result = flatten_schema(&resolved);
        assert_eq!(result.schema.metadata.id.as_deref(), Some("urn:test:schema:1"));
    }

    #[test]
    fn test_flatten_no_warnings_for_clean_schema() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Clean".to_string(),
            make_schema_type("Clean", vec![make_field("x", SchemaFieldType::String)]),
        );

        let result = flatten_schema(&resolved);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_flatten_source_files_empty_when_no_imports() {
        let resolved = make_empty_resolved();
        let result = flatten_schema(&resolved);
        assert!(result.source_files.is_empty());
    }

    #[test]
    fn test_flatten_source_files_from_multiple_imports() {
        let mut resolved = make_empty_resolved();
        resolved.resolved_paths = vec![
            "/a.odin".to_string(),
            "/b.odin".to_string(),
            "/c.odin".to_string(),
        ];

        let result = flatten_schema(&resolved);
        assert_eq!(result.source_files.len(), 3);
    }

    #[test]
    fn test_bundle_schema_with_metadata() {
        let mut resolved = make_empty_resolved();
        resolved.schema.metadata.title = Some("Bundle Test".to_string());
        resolved.schema.metadata.version = Some("1.0".to_string());
        resolved.schema.types.insert(
            "X".to_string(),
            make_schema_type("X", vec![make_field("f", SchemaFieldType::String)]),
        );

        let (text, warnings) = bundle_schema(&resolved, FlattenerOptions::default());
        assert!(text.contains("{$}"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_bundle_schema_no_import_directives() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Main".to_string(),
            make_schema_type("Main", vec![make_field("val", SchemaFieldType::String)]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert(
            "Imp".to_string(),
            make_schema_type("Imp", vec![make_field("x", SchemaFieldType::Integer)]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/imp.odin".to_string(),
            alias: Some("ns".to_string()),
            schema: Some(imported),
        });

        let (text, _) = bundle_schema(&resolved, FlattenerOptions {
            tree_shake: false,
            ..Default::default()
        });
        assert!(!text.contains("@import"));
    }

    #[test]
    fn test_conflict_resolution_namespace_default_no_conflict() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Local".to_string(),
            make_schema_type("Local", vec![]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert(
            "Remote".to_string(),
            make_schema_type("Remote", vec![]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/ext.odin".to_string(),
            alias: Some("ext".to_string()),
            schema: Some(imported),
        });

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            tree_shake: false,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        assert!(result.schema.types.contains_key("Local"));
        assert!(result.schema.types.contains_key("ext_Remote"));
        assert!(!result.warnings.iter().any(|w| w.contains("conflict")));
    }

    #[test]
    fn test_inheritance_no_parent_found() {
        let mut resolved = make_empty_resolved();

        let mut orphan = make_schema_type("Orphan", vec![
            make_field("f", SchemaFieldType::String),
        ]);
        orphan.parents = vec!["NonExistent".to_string()];
        resolved.schema.types.insert("Orphan".to_string(), orphan);

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("Orphan"));
        let t = result.schema.types.get("Orphan").unwrap();
        assert_eq!(t.fields.len(), 1);
        assert_eq!(t.fields[0].name, "f");
    }

    #[test]
    fn test_inheritance_grandchild() {
        let mut resolved = make_empty_resolved();

        resolved.schema.types.insert(
            "Grand".to_string(),
            make_schema_type("Grand", vec![make_field("g", SchemaFieldType::String)]),
        );

        let mut parent = make_schema_type("Parent", vec![make_field("p", SchemaFieldType::Integer)]);
        parent.parents = vec!["Grand".to_string()];
        resolved.schema.types.insert("Parent".to_string(), parent);

        let mut child = make_schema_type("Child", vec![make_field("c", SchemaFieldType::Boolean)]);
        child.parents = vec!["Parent".to_string()];
        resolved.schema.types.insert("Child".to_string(), child);

        let result = flatten_schema(&resolved);
        let child_type = result.schema.types.get("Child").unwrap();
        let field_names: Vec<&str> = child_type.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(field_names.contains(&"g"), "Missing grandparent field");
        assert!(field_names.contains(&"p"), "Missing parent field");
        assert!(field_names.contains(&"c"), "Missing child field");
    }

    #[test]
    fn test_flatten_large_type_count() {
        let mut resolved = make_empty_resolved();
        for i in 0..20 {
            resolved.schema.types.insert(
                format!("Type{i}"),
                make_schema_type(&format!("Type{i}"), vec![
                    make_field("f", SchemaFieldType::String),
                ]),
            );
        }

        let result = flatten_schema(&resolved);
        assert_eq!(result.schema.types.len(), 20);
    }

    #[test]
    fn test_flatten_type_with_many_fields() {
        let mut resolved = make_empty_resolved();
        let fields: Vec<SchemaField> = (0..15)
            .map(|i| make_field(&format!("field_{i}"), SchemaFieldType::String))
            .collect();
        resolved.schema.types.insert(
            "Wide".to_string(),
            make_schema_type("Wide", fields),
        );

        let result = flatten_schema(&resolved);
        let t = result.schema.types.get("Wide").unwrap();
        assert_eq!(t.fields.len(), 15);
    }

    #[test]
    fn test_merge_constraints_from_import() {
        let mut resolved = make_empty_resolved();

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.constraints.insert(
            "age_range".to_string(),
            vec![SchemaObjectConstraint::Invariant("age >= 0 && age <= 150".to_string())],
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/constraints.odin".to_string(),
            alias: Some("c".to_string()),
            schema: Some(imported),
        });

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            tree_shake: false,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        assert!(result.schema.constraints.contains_key("c_age_range"));
    }

    #[test]
    fn test_flatten_preserves_primary_constraints() {
        let mut resolved = make_empty_resolved();
        resolved.schema.constraints.insert(
            "my_rule".to_string(),
            vec![SchemaObjectConstraint::Invariant("x > 0".to_string())],
        );

        // Use no tree-shaking to ensure constraints are preserved
        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            tree_shake: false,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        assert!(result.schema.constraints.contains_key("my_rule"));
        let c = result.schema.constraints.get("my_rule").unwrap();
        assert_eq!(c.len(), 1);
        assert!(matches!(&c[0], SchemaObjectConstraint::Invariant(s) if s == "x > 0"));
    }

    #[test]
    fn test_flattener_options_custom() {
        let opts = FlattenerOptions {
            conflict_resolution: ConflictResolution::Error,
            tree_shake: false,
            inline_type_references: true,
            metadata: Some(SchemaMetadata {
                id: Some("custom".to_string()),
                title: Some("Custom".to_string()),
                description: None,
                version: None,
            }),
        };
        assert_eq!(opts.conflict_resolution, ConflictResolution::Error);
        assert!(!opts.tree_shake);
        assert!(opts.inline_type_references);
        assert!(opts.metadata.is_some());
    }

    #[test]
    fn test_flatten_multiple_none_schema_imports_ignored() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Only".to_string(),
            make_schema_type("Only", vec![]),
        );

        for i in 0..5 {
            resolved.imports.push(super::super::ResolvedImport {
                path: format!("/doc{i}.odin"),
                alias: Some(format!("d{i}")),
                schema: None,
            });
        }

        let result = flatten_schema(&resolved);
        assert_eq!(result.schema.types.len(), 1);
    }

    #[test]
    fn test_flatten_import_no_alias_preserves_type_name() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Main".to_string(),
            make_schema_type("Main", vec![
                make_field("ref", SchemaFieldType::TypeRef("Helper".to_string())),
            ]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert(
            "Helper".to_string(),
            make_schema_type("Helper", vec![make_field("h", SchemaFieldType::Boolean)]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/help.odin".to_string(),
            alias: None,
            schema: Some(imported),
        });

        let result = flatten_schema(&resolved);
        assert!(result.schema.types.contains_key("Helper"));
    }

    #[test]
    fn test_flatten_overwrite_conflict_primary_wins() {
        let mut resolved = make_empty_resolved();

        resolved.schema.types.insert(
            "Dup".to_string(),
            make_schema_type("Dup", vec![make_field("primary_f", SchemaFieldType::String)]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert(
            "Dup".to_string(),
            make_schema_type("Dup", vec![make_field("imported_f", SchemaFieldType::Integer)]),
        );

        resolved.imports.push(super::super::ResolvedImport {
            path: "/imp.odin".to_string(),
            alias: None,
            schema: Some(imported),
        });

        let mut flattener = SchemaFlattener::new(FlattenerOptions {
            conflict_resolution: ConflictResolution::Overwrite,
            tree_shake: false,
            ..Default::default()
        });
        let result = flattener.flatten_resolved(&resolved);
        let dup = result.schema.types.get("Dup").unwrap();
        assert!(dup.fields.iter().any(|f| f.name == "primary_f"));
    }

    #[test]
    fn test_qualified_name_long_namespace() {
        let flattener = SchemaFlattener::new(FlattenerOptions::default());
        assert_eq!(
            flattener.build_qualified_name("Type", Some("very_long_namespace")),
            "very_long_namespace_Type"
        );
    }

    #[test]
    fn test_qualified_name_with_numbers() {
        let flattener = SchemaFlattener::new(FlattenerOptions::default());
        assert_eq!(
            flattener.build_qualified_name("Type2", Some("ns1")),
            "ns1_Type2"
        );
    }

    #[test]
    fn test_bundle_schema_with_custom_options() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "Main".to_string(),
            make_schema_type("Main", vec![make_field("x", SchemaFieldType::String)]),
        );

        let opts = FlattenerOptions {
            tree_shake: false,
            conflict_resolution: ConflictResolution::Overwrite,
            ..Default::default()
        };
        let (text, warnings) = bundle_schema(&resolved, opts);
        assert!(text.contains("Main"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_flatten_result_schema_has_no_imports() {
        let mut resolved = make_empty_resolved();
        resolved.schema.types.insert(
            "T".to_string(),
            make_schema_type("T", vec![make_field("r", SchemaFieldType::TypeRef("ns_R".to_string()))]),
        );

        let mut imported = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };
        imported.types.insert("R".to_string(), make_schema_type("R", vec![]));

        resolved.imports.push(super::super::ResolvedImport {
            path: "/r.odin".to_string(),
            alias: Some("ns".to_string()),
            schema: Some(imported),
        });

        let result = flatten_schema(&resolved);
        assert!(result.schema.imports.is_empty());
    }
}
