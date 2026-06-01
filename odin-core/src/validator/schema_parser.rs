//! Schema parser — converts ODIN schema text to `OdinSchemaDefinition`.
//!
//! Parses schema headers, type definitions, field constraints, array specs,
//! and object-level constraints.
//!
//! Supports two type definition syntaxes:
//! - Header-based: `{@TypeName}` followed by field definitions
//! - Standalone: `@TypeName` on its own line followed by field definitions
//!
//! Type inheritance: `@Child : @Parent` or `@Child : @Parent & @Other`

use crate::types::schema::{OdinSchemaDefinition, SchemaMetadata, SchemaImport, SchemaType, SchemaField, SchemaArray, SchemaObjectConstraint, SchemaFieldType, SchemaConstraint, SchemaConditional, SchemaDefault, ConditionalOperator, ConditionalValue};
use crate::types::errors::ParseError;
use std::collections::HashMap;

/// Parse an ODIN schema from text into an `OdinSchemaDefinition`.
pub fn parse_schema(input: &str) -> Result<OdinSchemaDefinition, ParseError> {
    let mut parser = SchemaParserState::new();
    parser.parse(input)?;
    Ok(parser.build())
}

struct SchemaParserState {
    metadata: SchemaMetadata,
    imports: Vec<SchemaImport>,
    types: HashMap<String, SchemaType>,
    fields: HashMap<String, SchemaField>,
    arrays: HashMap<String, SchemaArray>,
    constraints: HashMap<String, Vec<SchemaObjectConstraint>>,

    // Parser state
    current_context: ParserContext,
    current_type_name: String,
    current_type_parents: Vec<String>,
    current_type_override_bases: Vec<String>,
    current_type_fields: Vec<SchemaField>,
    current_section_path: String,

    // Relative-header context: last absolute object path, last absolute type,
    // and the sub-path of the current `{.sub}` block within that context.
    previous_header_path: String,
    previous_header_type: String,
    current_type_sub_path: String,
}

#[derive(Debug, Clone, PartialEq)]
enum ParserContext {
    None,
    Metadata,
    TypeDef,
    Section,
    ArrayDef,
}

impl SchemaParserState {
    fn new() -> Self {
        Self {
            metadata: SchemaMetadata::default(),
            imports: Vec::new(),
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
            current_context: ParserContext::None,
            current_type_name: String::new(),
            current_type_parents: Vec::new(),
            current_type_override_bases: Vec::new(),
            current_type_fields: Vec::new(),
            current_section_path: String::new(),
            previous_header_path: String::new(),
            previous_header_type: String::new(),
            current_type_sub_path: String::new(),
        }
    }

    fn parse(&mut self, input: &str) -> Result<(), ParseError> {
        for (line_num, line) in input.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with(';') {
                continue;
            }

            // Import directive
            if trimmed.starts_with("@import") {
                self.parse_import(trimmed);
                continue;
            }

            // Header line: {$}, {section}, {@TypeName}, {path[]}
            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                self.flush_current_context();
                self.parse_header(trimmed);
                continue;
            }

            // Standalone type definition: @TypeName or @TypeName : @Parent
            if trimmed.starts_with('@') && !trimmed.contains('=') {
                self.flush_current_context();
                self.parse_standalone_type(trimmed);
                continue;
            }

            // Object-level constraint: :one_of(...), :exactly_one(...), :invariant ...
            if trimmed.starts_with(':') {
                self.parse_object_constraint(trimmed);
                continue;
            }

            // Assignment or field definition: key = value
            if let Some(eq_pos) = trimmed.find('=') {
                let key = trimmed[..eq_pos].trim();
                let value = trimmed[eq_pos + 1..].trim();

                // Type/section composition: `= @a & @b` (empty key).
                if key.is_empty() {
                    if self.current_context == ParserContext::TypeDef {
                        self.parse_type_value(value);
                        continue;
                    }
                    if self.current_context == ParserContext::Section {
                        self.parse_section_composition(value);
                        continue;
                    }
                }

                self.parse_assignment(key, value, line_num + 1)?;
            }
        }

        self.flush_current_context();
        Ok(())
    }

    fn flush_current_context(&mut self) {
        if self.current_context == ParserContext::TypeDef && !self.current_type_name.is_empty() {
            let name = std::mem::take(&mut self.current_type_name);
            let fields = std::mem::take(&mut self.current_type_fields);
            let override_bases = std::mem::take(&mut self.current_type_override_bases);

            // Merge into existing type if present (for multi-line `= ...` defs)
            if let Some(existing) = self.types.get_mut(&name) {
                for f in fields {
                    existing.fields.push(f);
                }
                for b in override_bases {
                    if !existing.override_bases.contains(&b) {
                        existing.override_bases.push(b);
                    }
                }
            } else {
                self.types.insert(name.clone(), SchemaType {
                    name,
                    description: None,
                    fields,
                    parents: std::mem::take(&mut self.current_type_parents),
                    override_bases,
                });
            }
        }
        self.current_context = ParserContext::None;
    }

    fn parse_import(&mut self, line: &str) {
        let rest = line.strip_prefix("@import").unwrap_or("").trim();
        let (path, alias) = if let Some(after_quote) = rest.strip_prefix('"') {
            if let Some(end_quote) = after_quote.find('"') {
                let path = &after_quote[..end_quote];
                let after = after_quote[end_quote + 1..].trim();
                let alias = after.strip_prefix("as").map(|s| s.trim().to_string());
                (path.to_string(), alias)
            } else {
                (rest.trim_matches('"').to_string(), None)
            }
        } else {
            (rest.to_string(), None)
        };
        self.imports.push(SchemaImport { path, alias });
    }

    fn parse_header(&mut self, line: &str) {
        let inner = line[1..line.len() - 1].trim();

        if inner == "$" {
            self.current_context = ParserContext::Metadata;
            self.current_section_path.clear();
            return;
        }

        // Relative header `{.sub}`: nest under the last absolute context.
        if let Some(rel) = inner.strip_prefix('.') {
            let sub = rel.trim();
            if !self.previous_header_type.is_empty() {
                // Re-open the parent type; fields route under the sub-path.
                self.current_context = ParserContext::TypeDef;
                self.current_type_name = self.previous_header_type.clone();
                self.current_type_parents.clear();
                self.current_type_fields.clear();
                self.current_type_sub_path = sub.to_string();
            } else {
                // Object context: relative sub-blocks scope to the last path.
                self.current_context = ParserContext::Section;
                self.current_section_path = if self.previous_header_path.is_empty() {
                    sub.to_string()
                } else {
                    format!("{}.{}", self.previous_header_path, sub)
                };
                self.current_type_sub_path.clear();
            }
            return;
        }

        // Absolute header resets the relative sub-path.
        self.current_type_sub_path.clear();

        if let Some(type_rest) = inner.strip_prefix('@') {
            // Type definition: {@TypeName}
            let type_name = type_rest.trim().to_string();
            self.current_context = ParserContext::TypeDef;
            self.current_type_name = type_name.clone();
            self.current_type_fields.clear();
            self.previous_header_type = type_name;
            self.previous_header_path.clear();
            return;
        }

        // Array header: `{path[]}` or tabular `{path[] : col1, col2}`.
        if let Some(bracket) = inner.find("[]") {
            let path = inner[..bracket].trim().to_string();
            let after = inner[bracket + 2..].trim();
            let columns: Vec<String> = if let Some(cols) = after.strip_prefix(':') {
                cols.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            } else {
                Vec::new()
            };
            self.arrays.entry(path.clone()).or_insert_with(|| SchemaArray {
                name: path.clone(),
                item_type: SchemaFieldType::String,
                min_items: None,
                max_items: None,
                unique: false,
                columns: Vec::new(),
                item_fields: HashMap::new(),
            }).columns = columns;
            self.current_context = ParserContext::ArrayDef;
            self.current_section_path = path.clone();
            self.previous_header_path = path;
            self.previous_header_type.clear();
            return;
        }

        // Regular section: {path}
        self.current_context = ParserContext::Section;
        self.current_section_path = inner.to_string();
        self.previous_header_path = inner.to_string();
        self.previous_header_type.clear();
    }

    /// Parse standalone `@TypeName` or `@TypeName : @Parent [& @Other]`
    fn parse_standalone_type(&mut self, line: &str) {
        let rest = &line[1..]; // strip leading @

        // Check for inheritance: @Child : @Parent [& @Other]
        let (type_name, parents) = if let Some(colon_pos) = rest.find(" : ") {
            let name = rest[..colon_pos].trim().to_string();
            let parents_str = rest[colon_pos + 3..].trim();
            let parents: Vec<String> = parents_str
                .split('&')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            (name, parents)
        } else {
            (rest.trim().to_string(), Vec::new())
        };

        self.current_context = ParserContext::TypeDef;
        self.current_type_name = type_name;
        self.current_type_parents = parents;
        self.current_type_fields.clear();
    }

    /// Parse type-level value definition: `= :constraints` or `= ##:(bounds)`
    fn parse_type_value(&mut self, value: &str) {
        // This handles lines like:
        //   = :(10..15) :pattern "^..."
        //   = ##:(1..)
        //   = :format email
        //   = :deprecated "msg"
        //   = ""|~  (union)
        //   = #|""  (union)
        //   = @TypeA & @TypeB (intersection)
        //   = :(20)  (exact length/value)
        let value = value.trim();
        // Separate any trailing `:override` directive from the `@`-references.
        let (refs_part, is_override) = split_override(value);
        if let Some(refs) = parse_type_intersection(refs_part) {
            for r in refs {
                if is_override && !self.current_type_override_bases.contains(&r) {
                    self.current_type_override_bases.push(r.clone());
                }
                if !self.current_type_parents.contains(&r) {
                    self.current_type_parents.push(r);
                }
            }
        }
    }

    /// Parse a section-level composition `= @A & @B`, recording a `_composition`
    /// field whose `TypeRef` name carries every `&`-joined member.
    fn parse_section_composition(&mut self, value: &str) {
        let (refs_part, is_override) = split_override(value.trim());
        let Some(refs) = parse_type_intersection(refs_part) else { return };
        if refs.is_empty() {
            return;
        }
        let name = refs.join("&");
        let path = if self.current_section_path.is_empty() {
            "_composition".to_string()
        } else {
            format!("{}._composition", self.current_section_path)
        };
        let field = SchemaField {
            name: "_composition".to_string(),
            field_type: SchemaFieldType::TypeRef(name),
            required: false,
            confidential: false,
            deprecated: false,
            immutable: false,
            computed: false,
            // Mark section overrides so schema-definition checks distinguish
            // them from intersections.
            description: if is_override { Some("override".to_string()) } else { None },
            constraints: Vec::new(),
            default_value: None,
            conditionals: Vec::new(),
        };
        self.fields.insert(path, field);
    }

    fn parse_object_constraint(&mut self, line: &str) {
        let path = self.current_section_path.clone();

        if line.starts_with(":invariant") {
            let expr = line.strip_prefix(":invariant").unwrap_or("").trim().to_string();
            self.constraints.entry(path).or_default().push(
                SchemaObjectConstraint::Invariant(expr),
            );
            return;
        }

        // Cardinality constraints: :one_of(...), :exactly_one(...), :at_most_one(...)
        let (min, max, rest) = if let Some(rest) = line.strip_prefix(":exactly_one") {
            (Some(1), Some(1), rest.trim())
        } else if let Some(rest) = line.strip_prefix(":at_most_one") {
            (None, Some(1), rest.trim())
        } else if let Some(rest) = line.strip_prefix(":one_of") {
            (Some(1), None, rest.trim())
        } else if let Some(rest) = line.strip_prefix(":of") {
            let rest = rest.trim();
            if rest.starts_with('(') {
                if let Some(paren_end) = rest.find(')') {
                    let range_str = &rest[1..paren_end];
                    let fields_str = rest[paren_end + 1..].trim();
                    let (min, max) = parse_range(range_str);
                    (min, max, fields_str)
                } else {
                    (None, None, rest)
                }
            } else {
                (None, None, rest)
            }
        } else {
            return;
        };

        // Parse field list — could be (a, b, c) or a, b, c
        let fields_str = if rest.starts_with('(') {
            if let Some(paren_end) = rest.find(')') {
                &rest[1..paren_end]
            } else {
                rest
            }
        } else {
            rest
        };

        let fields: Vec<String> = fields_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if !fields.is_empty() {
            self.constraints.entry(path).or_default().push(
                SchemaObjectConstraint::Cardinality { fields, min, max },
            );
        }
    }

    fn parse_assignment(&mut self, key: &str, value: &str, _line: usize) -> Result<(), ParseError> {
        match self.current_context {
            ParserContext::Metadata => {
                let uv = unquote(value);
                match key {
                    "id" => self.metadata.id = Some(uv),
                    "title" => self.metadata.title = Some(uv),
                    "description" => self.metadata.description = Some(uv),
                    "version" => self.metadata.version = Some(uv),
                    _ => {} // odin, schema, etc. — skip for now
                }
            }
            ParserContext::TypeDef => {
                // Check for array field: items[] = @Type
                let clean_key = key.strip_suffix("[]").map(str::trim).unwrap_or(key);
                // Prefix the field name when inside a relative sub-block (e.g. term.effective).
                let field_name = if self.current_type_sub_path.is_empty() {
                    clean_key.to_string()
                } else {
                    format!("{}.{}", self.current_type_sub_path, clean_key)
                };
                let field = parse_field_def(&field_name, value);
                self.current_type_fields.push(field);
            }
            ParserContext::Section => {
                let full_path = if self.current_section_path.is_empty() {
                    key.to_string()
                } else {
                    // Check for array field: items[] = @Type
                    if let Some(field_name) = key.strip_suffix("[]") {
                        format!("{}.{}", self.current_section_path, field_name)
                    } else {
                        format!("{}.{}", self.current_section_path, key)
                    }
                };
                let clean_key = key.strip_suffix("[]").unwrap_or(key);
                let field = parse_field_def(clean_key, value);
                self.fields.insert(full_path, field);
            }
            ParserContext::ArrayDef => {
                let clean_key = key.strip_suffix("[]").unwrap_or(key);
                let full_path = format!("{}[].{}", self.current_section_path, clean_key);
                let field = parse_field_def(clean_key, value);
                if let Some(array) = self.arrays.get_mut(&self.current_section_path) {
                    array.item_fields.insert(clean_key.to_string(), field.clone());
                }
                self.fields.insert(full_path, field);
            }
            ParserContext::None => {
                let field = parse_field_def(key, value);
                self.fields.insert(key.to_string(), field);
            }
        }
        Ok(())
    }

    fn build(self) -> OdinSchemaDefinition {
        OdinSchemaDefinition {
            metadata: self.metadata,
            imports: self.imports,
            types: self.types,
            fields: self.fields,
            arrays: self.arrays,
            constraints: self.constraints,
            validation_memo: Default::default(),
        }
    }
}

/// Parse a field definition from schema text.
fn parse_field_def(name: &str, value: &str) -> SchemaField {
    let mut required = false;
    let mut confidential = false;
    let mut deprecated = false;
    let mut immutable = false;
    let mut computed = false;
    let mut constraints = Vec::new();
    let mut conditionals = Vec::new();
    let mut field_type = SchemaFieldType::String;
    let mut default_value: Option<SchemaDefault> = None;

    let name = name.trim();
    let value = value.trim();

    // Strip inline comment
    let value = if let Some(semi) = find_unquoted_semicolon(value) {
        value[..semi].trim()
    } else {
        value
    };

    // Parse directive-based constraints: :required, :optional, :format, :pattern, etc.
    let mut rest = value;

    // Check for modifier prefix (! * -)
    loop {
        if rest.starts_with('!') {
            required = true;
            rest = rest[1..].trim_start();
        } else if rest.starts_with('*') {
            confidential = true;
            rest = rest[1..].trim_start();
        } else if rest.starts_with('-') && rest.len() > 1 && !rest[1..].starts_with(|c: char| c.is_ascii_digit()) {
            deprecated = true;
            rest = rest[1..].trim_start();
        } else {
            break;
        }
    }

    // Leading `~` nullable modifier on a typed field (e.g. `~#`, `~##`).
    // A lone `~` is the null type and is handled below.
    let mut nullable_prefix = false;
    if rest.starts_with('~') && rest.len() > 1 {
        nullable_prefix = true;
        rest = rest[1..].trim_start();
    }

    // Detect type from prefix. A trailing default value (e.g. `##3`, `#$5.00`)
    // is captured here; union members (e.g. `#|~`) extend the base type.
    let mut parsed_default: Option<SchemaDefault> = None;
    if rest.starts_with("#%") {
        let after = rest[2..].trim_start();
        let (def, after) = take_numeric_default(after, "percent");
        parsed_default = def;
        field_type = SchemaFieldType::Percent;
        rest = after;
    } else if rest.starts_with("##") {
        let after = rest[2..].trim_start();
        let (def, after) = take_numeric_default(after, "integer");
        parsed_default = def;
        field_type = SchemaFieldType::Integer;
        rest = after;
    } else if rest.starts_with("#$") {
        let after = &rest[2..];
        if let Some((places, after_places)) = take_decimal_places(after) {
            field_type = SchemaFieldType::Currency { decimal_places: Some(places) };
            rest = after_places.trim_start();
        } else {
            let (def, after) = take_numeric_default(after.trim_start(), "currency");
            parsed_default = def;
            // Currency defaults to two decimal places when unspecified.
            field_type = SchemaFieldType::Currency { decimal_places: Some(2) };
            rest = after;
        }
    } else if rest.starts_with('#') && !rest.starts_with("#(") {
        let after = &rest[1..];
        if let Some((places, after_places)) = take_decimal_places(after) {
            field_type = SchemaFieldType::Decimal { decimal_places: Some(places) };
            rest = after_places.trim_start();
        } else {
            let (def, after) = take_numeric_default(after.trim_start(), "number");
            parsed_default = def;
            field_type = SchemaFieldType::Number { decimal_places: None };
            rest = after;
        }
    } else if rest.starts_with('^') {
        let after = &rest[1..];
        // Optional algorithm tag (e.g. `^sha256`); size constraint handled in the directive loop.
        let alg_end = after
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(after.len());
        rest = &after[alg_end..];
        field_type = SchemaFieldType::Binary;
    } else if rest.starts_with('?') {
        field_type = SchemaFieldType::Boolean;
        rest = rest[1..].trim_start();
    } else if rest == "~" {
        field_type = SchemaFieldType::Null;
        rest = "";
    }

    if parsed_default.is_some() {
        default_value = parsed_default;
    }

    // Bare temporal base type (e.g. `date`, `timestamp:immutable`, `date:(min..max)`).
    // A glued `:immutable`/`:computed` directive is applied; other suffixes (`:(...)`,
    // `:format`, ...) are left for the constraint loop.
    if let Some((temporal, after)) = take_temporal(rest) {
        field_type = temporal;
        rest = apply_glued_directives(after, &mut immutable, &mut computed);
    }

    // Union: a `|`-joined member list extends the base type (e.g. `#|~`).
    if rest.starts_with('|') {
        if let Some((ty, after)) = parse_union(rest, &field_type) {
            field_type = ty;
            rest = after;
        }
    } else if let Some((first, after_first)) = take_union_member(rest) {
        // Bare-word leading member (e.g. `date|timestamp`).
        if after_first.trim_start().starts_with('|') {
            if let Some((ty, after)) = parse_union(after_first.trim_start(), &first) {
                field_type = ty;
                rest = after;
            }
        }
    }

    // Check for @TypeRef
    if rest.starts_with('@') {
        let type_ref = rest.split_whitespace().next().unwrap_or(rest);
        field_type = SchemaFieldType::TypeRef(type_ref[1..].to_string());
        rest = rest[type_ref.len()..].trim_start();
    }

    // A leading `~` makes the field nullable: admit the null type alongside it.
    if nullable_prefix && !matches!(field_type, SchemaFieldType::Null) {
        field_type = match field_type {
            SchemaFieldType::Union(mut members) => {
                if !members.iter().any(|m| matches!(m, SchemaFieldType::Null)) {
                    members.insert(0, SchemaFieldType::Null);
                }
                SchemaFieldType::Union(members)
            }
            other => SchemaFieldType::Union(vec![SchemaFieldType::Null, other]),
        };
    }

    // Parse constraint directives
    let mut remaining = rest;
    loop {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }

        // A bare leading `(...)` is an enum/bounds constraint (no `:` prefix).
        if remaining.starts_with('(') {
            if let Some(paren_end) = remaining.find(')') {
                let inner = &remaining[1..paren_end];
                if inner.contains("..") {
                    let (min, max) = parse_bounds_pair(inner);
                    constraints.push(SchemaConstraint::Bounds {
                        min,
                        max,
                        min_exclusive: false,
                        max_exclusive: false,
                    });
                } else if inner.contains(',') {
                    let values: Vec<String> = inner.split(',')
                        .map(|s| s.trim().trim_matches('"').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    constraints.push(SchemaConstraint::Enum(values));
                } else {
                    let v = inner.trim();
                    if !v.is_empty() {
                        constraints.push(SchemaConstraint::Bounds {
                            min: Some(v.to_string()),
                            max: Some(v.to_string()),
                            min_exclusive: false,
                            max_exclusive: false,
                        });
                    }
                }
                remaining = remaining[paren_end + 1..].trim_start();
                continue;
            }
        }

        if let Some(after) = remaining.strip_prefix(':') {

            if let Some(rest) = after.strip_prefix("required") {
                required = true;
                remaining = rest.trim_start();
                continue;
            }
            if let Some(rest) = after.strip_prefix("optional") {
                required = false;
                remaining = rest.trim_start();
                continue;
            }
            if let Some(format_rest) = after.strip_prefix("format ") {
                let format_name = format_rest.split_whitespace().next().unwrap_or("").to_string();
                remaining = format_rest[format_rest.find(char::is_whitespace).unwrap_or(format_rest.len())..].trim_start();
                constraints.push(SchemaConstraint::Format(format_name));
                continue;
            }
            if let Some(pat_rest) = after.strip_prefix("pattern ") {
                if let Some(pattern) = extract_quoted(pat_rest) {
                    constraints.push(SchemaConstraint::Pattern(pattern.0.to_string()));
                    remaining = pat_rest[pattern.1..].trim_start();
                    continue;
                }
            }
            if let Some(rest) = after.strip_prefix("unique") {
                constraints.push(SchemaConstraint::Unique);
                remaining = rest.trim_start();
                continue;
            }
            if after.starts_with("enum(") || after.starts_with("enum (") {
                let start = if after.starts_with("enum(") { 5 } else { 6 };
                if let Some(paren_end) = after[start..].find(')') {
                    let inner = &after[start..start + paren_end];
                    let values: Vec<String> = inner.split(',')
                        .map(|s| s.trim().trim_matches('"').to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    constraints.push(SchemaConstraint::Enum(values));
                    remaining = after[start + paren_end + 1..].trim_start();
                    continue;
                }
            }
            if let Some(rest) = after.strip_prefix("computed") {
                computed = true;
                remaining = rest.trim_start();
                continue;
            }
            if let Some(rest) = after.strip_prefix("immutable") {
                immutable = true;
                remaining = rest.trim_start();
                continue;
            }
            if let Some(rest) = after.strip_prefix("deprecated") {
                deprecated = true;
                remaining = rest.trim_start();
                // Skip optional message
                if let Some(after_quote) = remaining.strip_prefix('"') {
                    if let Some(end) = after_quote.find('"') {
                        remaining = after_quote[end + 1..].trim_start();
                    }
                }
                continue;
            }
            if let Some(rest) = after.strip_prefix("override") {
                remaining = rest.trim_start();
                continue;
            }
            // Conditional: `:if field op value` / `:unless field op value`
            if let Some(cond_rest) = after.strip_prefix("if ").or_else(|| after.strip_prefix("unless ")) {
                let unless = after.starts_with("unless ");
                if let Some(cond) = parse_conditional(cond_rest, unless) {
                    conditionals.push(cond);
                }
                remaining = "";
                continue;
            }
            // Temporal type with an optionally-glued directive (e.g. `timestamp:immutable`).
            if let Some((temporal, after_temporal)) = take_temporal(after) {
                field_type = temporal;
                let after_glue = apply_glued_directives(after_temporal, &mut immutable, &mut computed);
                remaining = after_glue.trim_start();
                continue;
            }
            // Bounds/enum: :(...)
            if after.starts_with('(') {
                if let Some(paren_end) = after.find(')') {
                    let inner = &after[1..paren_end];
                    if matches!(field_type, SchemaFieldType::Binary) && !inner.contains(',') {
                        // Binary size constraint: byte-length bounds or exact length.
                        let (min, max) = if inner.contains("..") {
                            parse_bounds_pair(inner)
                        } else {
                            let v = inner.trim().to_string();
                            (Some(v.clone()), Some(v))
                        };
                        constraints.push(SchemaConstraint::Size {
                            min: min.and_then(|s| s.parse::<u64>().ok()),
                            max: max.and_then(|s| s.parse::<u64>().ok()),
                        });
                        remaining = after[paren_end + 1..].trim_start();
                        continue;
                    }
                    if inner.contains("..") {
                        let (min, max) = parse_bounds_pair(inner);
                        constraints.push(SchemaConstraint::Bounds {
                            min,
                            max,
                            min_exclusive: false,
                            max_exclusive: false,
                        });
                    } else if inner.contains(',') {
                        // Enum
                        let values: Vec<String> = inner.split(',')
                            .map(|s| s.trim().trim_matches('"').to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        constraints.push(SchemaConstraint::Enum(values));
                    } else {
                        // Single value — exact length/value
                        let v = inner.trim();
                        if !v.is_empty() {
                            constraints.push(SchemaConstraint::Bounds {
                                min: Some(v.to_string()),
                                max: Some(v.to_string()),
                                min_exclusive: false,
                                max_exclusive: false,
                            });
                        }
                    }
                    remaining = after[paren_end + 1..].trim_start();
                    continue;
                }
            }
            // Pattern: :/regex/ — a trailing :if/:unless may follow the closer.
            if let Some(after_slash) = after.strip_prefix('/') {
                if let Some(end) = after_slash.find('/') {
                    let pattern = after_slash[..end].to_string();
                    constraints.push(SchemaConstraint::Pattern(pattern));
                    remaining = after_slash[end + 1..].trim_start();
                    continue;
                }
            }
            // Unknown directive — skip
            let next_space = after.find(char::is_whitespace).unwrap_or(after.len());
            remaining = after[next_space..].trim_start();
            continue;
        }

        // Non-directive remaining text: a type-name word, or a default value.
        if !remaining.is_empty() {
            let word = remaining.split_whitespace().next().unwrap_or(remaining);
            match word.trim_matches('"') {
                "string" => field_type = SchemaFieldType::String,
                "boolean" | "bool" => field_type = SchemaFieldType::Boolean,
                "number" | "float" | "decimal" => field_type = SchemaFieldType::Number { decimal_places: None },
                "integer" | "int" => field_type = SchemaFieldType::Integer,
                "currency" => field_type = SchemaFieldType::Currency { decimal_places: None },
                "date" => field_type = SchemaFieldType::Date,
                "timestamp" => field_type = SchemaFieldType::Timestamp,
                "time" => field_type = SchemaFieldType::Time,
                "duration" => field_type = SchemaFieldType::Duration,
                "percent" => field_type = SchemaFieldType::Percent,
                "binary" => field_type = SchemaFieldType::Binary,
                "null" => field_type = SchemaFieldType::Null,
                // A trailing literal default value (e.g. `= ##:(1..5) ##3` -> "##3").
                _ => {
                    if default_value.is_none() {
                        if let Some(def) = parse_default_literal(word, &field_type) {
                            default_value = Some(def);
                        }
                    }
                }
            }
        }
        break;
    }

    SchemaField {
        name: name.to_string(),
        field_type,
        required,
        confidential,
        deprecated,
        immutable,
        computed,
        description: None,
        constraints,
        default_value,
        conditionals,
    }
}

/// Recognized temporal base-type words and their `SchemaFieldType`.
fn temporal_type(word: &str) -> Option<SchemaFieldType> {
    match word {
        "date" => Some(SchemaFieldType::Date),
        "timestamp" => Some(SchemaFieldType::Timestamp),
        "time" => Some(SchemaFieldType::Time),
        "duration" => Some(SchemaFieldType::Duration),
        _ => None,
    }
}

/// Match a leading temporal type word in a `:directive` body, returning the type
/// and the text after the base word (which may carry glued `:immutable` etc.).
fn take_temporal(after: &str) -> Option<(SchemaFieldType, &str)> {
    for kw in ["timestamp", "duration", "date", "time"] {
        if let Some(rest) = after.strip_prefix(kw) {
            // The base word must end here or be followed by `:`/whitespace, not
            // be a longer identifier (avoid matching e.g. "datetime").
            if rest.is_empty() || rest.starts_with(':') || rest.starts_with(char::is_whitespace) {
                return temporal_type(kw).map(|t| (t, rest));
            }
        }
    }
    None
}

/// Apply directives glued onto a type suffix (`:immutable`, `:computed`),
/// returning the remaining text after the consumed directives.
fn apply_glued_directives<'a>(mut suffix: &'a str, immutable: &mut bool, computed: &mut bool) -> &'a str {
    while let Some(after) = suffix.strip_prefix(':') {
        if let Some(rest) = after.strip_prefix("immutable") {
            *immutable = true;
            suffix = rest;
        } else if let Some(rest) = after.strip_prefix("computed") {
            *computed = true;
            suffix = rest;
        } else {
            break;
        }
    }
    suffix
}

/// Parse a union member list following a base type (e.g. `|~`, `|timestamp`).
/// `head` is the text immediately after the base type prefix. Returns the
/// resulting union type and the unconsumed remainder, or `None` if not a union.
fn parse_union<'a>(head: &'a str, base: &SchemaFieldType) -> Option<(SchemaFieldType, &'a str)> {
    // A union begins with `|` directly after the base, or the base itself was a
    // bare temporal word followed by `|` (handled by the caller passing `head`).
    if !head.starts_with('|') {
        return None;
    }
    let mut members = vec![base.clone()];
    let mut s = head;
    while let Some(after_bar) = s.strip_prefix('|') {
        let after_bar = after_bar.trim_start();
        let (member, rest) = take_union_member(after_bar)?;
        members.push(member);
        s = rest.trim_start();
        if !s.starts_with('|') {
            break;
        }
    }
    Some((SchemaFieldType::Union(members), s))
}

/// Take a single union member from the start of `s`, returning the member and
/// the unconsumed remainder.
fn take_union_member(s: &str) -> Option<(SchemaFieldType, &str)> {
    if let Some(rest) = s.strip_prefix("##") {
        return Some((SchemaFieldType::Integer, rest));
    }
    if let Some(rest) = s.strip_prefix("#%") {
        return Some((SchemaFieldType::Percent, rest));
    }
    if let Some(rest) = s.strip_prefix("#$") {
        return Some((SchemaFieldType::Currency { decimal_places: None }, rest));
    }
    if let Some(rest) = s.strip_prefix('#') {
        return Some((SchemaFieldType::Number { decimal_places: None }, rest));
    }
    if let Some(rest) = s.strip_prefix('~') {
        return Some((SchemaFieldType::Null, rest));
    }
    if let Some(rest) = s.strip_prefix('?') {
        return Some((SchemaFieldType::Boolean, rest));
    }
    if let Some(rest) = s.strip_prefix("\"\"").or_else(|| s.strip_prefix("''")) {
        return Some((SchemaFieldType::String, rest));
    }
    for kw in ["timestamp", "duration", "date", "time"] {
        if let Some(rest) = s.strip_prefix(kw) {
            if rest.is_empty() || rest.starts_with('|') || rest.starts_with(char::is_whitespace) {
                return temporal_type(kw).map(|t| (t, rest));
            }
        }
    }
    None
}

/// Capture a leading numeric default literal (the text after a type prefix),
/// returning the typed default and the unconsumed remainder.
/// Detect a glued decimal-places suffix (`.N`) on a numeric prefix (`#.4`, `#$.4`).
/// Returns the place count and the remaining text when present.
fn take_decimal_places(after: &str) -> Option<(u8, &str)> {
    let digits = after.strip_prefix('.')?;
    let end = digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let places = digits[..end].parse::<u8>().ok()?;
    Some((places, &digits[end..]))
}

fn take_numeric_default<'a>(after: &'a str, kind: &str) -> (Option<SchemaDefault>, &'a str) {
    // Stop at the first non-number boundary (whitespace, `:`, `|`).
    let end = after
        .find(|c: char| c.is_whitespace() || c == ':' || c == '|')
        .unwrap_or(after.len());
    let literal = &after[..end];
    if literal.is_empty() {
        return (None, after);
    }
    let def = match kind {
        "integer" => literal.parse::<i64>().ok().map(SchemaDefault::Integer),
        "currency" => literal.parse::<f64>().ok().map(SchemaDefault::Currency),
        "percent" => literal.parse::<f64>().ok().map(SchemaDefault::Percent),
        _ => literal.parse::<f64>().ok().map(SchemaDefault::Number),
    };
    if def.is_some() {
        (def, &after[end..])
    } else {
        (None, after)
    }
}

/// Parse a trailing default literal token typed to the field's declared type.
fn parse_default_literal(word: &str, field_type: &SchemaFieldType) -> Option<SchemaDefault> {
    let w = word.trim();
    if w.is_empty() {
        return None;
    }
    if let Some(rest) = w.strip_prefix("##") {
        return rest.parse::<i64>().ok().map(SchemaDefault::Integer);
    }
    if let Some(rest) = w.strip_prefix("#$") {
        return rest.parse::<f64>().ok().map(SchemaDefault::Currency);
    }
    if let Some(rest) = w.strip_prefix("#%") {
        return rest.parse::<f64>().ok().map(SchemaDefault::Percent);
    }
    if let Some(rest) = w.strip_prefix('#') {
        return rest.parse::<f64>().ok().map(SchemaDefault::Number);
    }
    if let Some(rest) = w.strip_prefix('?') {
        return Some(SchemaDefault::Bool(rest == "true"));
    }
    if w == "true" || w == "false" {
        return Some(SchemaDefault::Bool(w == "true"));
    }
    // Quoted string default (e.g. `"a"`).
    if w.len() >= 2 && w.starts_with('"') && w.ends_with('"') {
        return Some(SchemaDefault::String(w[1..w.len() - 1].to_string()));
    }
    // Bare numeric default — tag to the field's declared type.
    if let Ok(n) = w.parse::<f64>() {
        return Some(match field_type {
            SchemaFieldType::Integer => SchemaDefault::Integer(n as i64),
            SchemaFieldType::Currency { .. } => SchemaDefault::Currency(n),
            SchemaFieldType::Percent => SchemaDefault::Percent(n),
            _ => SchemaDefault::Number(n),
        });
    }
    None
}

/// Parse a `field op value` conditional body.
fn parse_conditional(body: &str, unless: bool) -> Option<SchemaConditional> {
    let body = body.trim();
    // Operators, longest first.
    for (op_str, op) in [
        ("!=", ConditionalOperator::NotEq),
        (">=", ConditionalOperator::Gte),
        ("<=", ConditionalOperator::Lte),
        (">", ConditionalOperator::Gt),
        ("<", ConditionalOperator::Lt),
        ("=", ConditionalOperator::Eq),
    ] {
        if let Some(pos) = body.find(op_str) {
            let field = body[..pos].trim().to_string();
            let raw = body[pos + op_str.len()..].trim();
            if field.is_empty() || raw.is_empty() {
                continue;
            }
            let value = parse_conditional_value(raw);
            return Some(SchemaConditional { field, operator: op, value, unless });
        }
    }
    // Shorthand `:if field` -> `field = true`.
    if !body.is_empty() {
        return Some(SchemaConditional {
            field: body.to_string(),
            operator: ConditionalOperator::Eq,
            value: ConditionalValue::Bool(true),
            unless,
        });
    }
    None
}

/// Parse a conditional comparison value (bool, number, or quoted/bare string).
fn parse_conditional_value(raw: &str) -> ConditionalValue {
    if raw == "true" || raw == "false" {
        return ConditionalValue::Bool(raw == "true");
    }
    if let Ok(n) = raw.parse::<f64>() {
        return ConditionalValue::Number(n);
    }
    ConditionalValue::String(raw.trim_matches(|c| c == '"' || c == '\'').to_string())
}

/// Parse a `min..max` bounds pair, preserving temporal/string literals.
fn parse_bounds_pair(inner: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = inner.splitn(2, "..").collect();
    let min = parts.first().map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_string);
    let max = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty()).map(str::to_string);
    (min, max)
}

/// Split a composition value into its `@`-reference part and whether it carries
/// an `:override` directive.
fn split_override(value: &str) -> (&str, bool) {
    if let Some(pos) = value.find(":override") {
        (value[..pos].trim_end(), true)
    } else {
        (value, false)
    }
}

/// Parse an intersection of type references (`@A & @B`) into bare type names.
/// Returns `None` when the value is not a `@`-reference composition.
fn parse_type_intersection(value: &str) -> Option<Vec<String>> {
    if !value.starts_with('@') {
        return None;
    }
    let refs: Vec<String> = value
        .split('&')
        .map(|s| s.trim().trim_start_matches('@').trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if refs.is_empty() {
        None
    } else {
        Some(refs)
    }
}

fn parse_range(s: &str) -> (Option<usize>, Option<usize>) {
    let parts: Vec<&str> = s.split("..").collect();
    if parts.len() != 2 {
        return (None, None);
    }
    let min = parts[0].trim().parse::<usize>().ok();
    let max = parts[1].trim().parse::<usize>().ok();
    (min, max)
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn find_unquoted_semicolon(s: &str) -> Option<usize> {
    let mut in_quote = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => in_quote = !in_quote,
            ';' if !in_quote => return Some(i),
            _ => {}
        }
    }
    None
}

/// Extract a quoted string, returning (content, `end_position_after_closing_quote`).
fn extract_quoted(s: &str) -> Option<(&str, usize)> {
    if let Some(after_quote) = s.strip_prefix('"') {
        if let Some(end) = after_quote.find('"') {
            return Some((&after_quote[..end], end + 2));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_schema() {
        let schema_text = r#"
{$}
id = "test-schema"
title = "Test Schema"

{@Address}
street = ! :(1..100)
city = !
state = ! :(2)
zip = ! :format zip

{person}
name = ! :(1..50)
email = :format email
age = :(0..150)
"#;
        let schema = parse_schema(schema_text).unwrap();
        assert_eq!(schema.metadata.id.as_deref(), Some("test-schema"));
        assert_eq!(schema.types.len(), 1);
        assert!(schema.types.contains_key("Address"));
        assert_eq!(schema.types["Address"].fields.len(), 4);

        assert!(schema.fields.contains_key("person.name"));
        assert!(schema.fields.contains_key("person.email"));
        assert!(schema.fields.contains_key("person.age"));
    }

    #[test]
    fn test_parse_standalone_type() {
        let schema_text = "@PhoneNumber\n= :(10..15) :pattern \"^abc$\"";
        let schema = parse_schema(schema_text).unwrap();
        assert!(schema.types.contains_key("PhoneNumber"));
    }

    #[test]
    fn test_parse_type_with_fields() {
        let schema_text = "@BaseAddress\nstreet = \ncity = \nstate = :(2)";
        let schema = parse_schema(schema_text).unwrap();
        assert!(schema.types.contains_key("BaseAddress"));
        assert_eq!(schema.types["BaseAddress"].fields.len(), 3);
    }

    #[test]
    fn test_parse_type_inheritance() {
        let schema_text = "@BaseEntity\nid = :required\n\n@ExtendedEntity : @BaseEntity\ndescription = :optional";
        let schema = parse_schema(schema_text).unwrap();
        assert!(schema.types.contains_key("BaseEntity"));
        assert!(schema.types.contains_key("ExtendedEntity"));
    }

    #[test]
    fn test_parse_cardinality_with_parens() {
        let schema_text = "{contact}\n:one_of(home_phone, work_phone, mobile_phone)\nhome_phone = :optional";
        let schema = parse_schema(schema_text).unwrap();
        assert!(schema.constraints.contains_key("contact"));
        let c = &schema.constraints["contact"][0];
        assert!(matches!(c, SchemaObjectConstraint::Cardinality { fields, min: Some(1), max: None } if fields.len() == 3));
    }

    #[test]
    fn test_parse_constraints() {
        let schema_text = r#"
{data}
email = :format email
pattern = :/^[A-Z]+$/
enum_field = :(red, green, blue)
bounded = :(1..100)
"#;
        let schema = parse_schema(schema_text).unwrap();

        let email = &schema.fields["data.email"];
        assert!(matches!(&email.constraints[0], SchemaConstraint::Format(f) if f == "email"));

        let pattern = &schema.fields["data.pattern"];
        assert!(matches!(&pattern.constraints[0], SchemaConstraint::Pattern(p) if p == "^[A-Z]+$"));

        let enum_field = &schema.fields["data.enum_field"];
        assert!(matches!(&enum_field.constraints[0], SchemaConstraint::Enum(vals) if vals.len() == 3));

        let bounded = &schema.fields["data.bounded"];
        assert!(matches!(&bounded.constraints[0], SchemaConstraint::Bounds { .. }));
    }

    #[test]
    fn test_parse_modifiers() {
        let schema_text = r#"
{data}
req = !
conf = *"value"
dep = -"old"
"#;
        let schema = parse_schema(schema_text).unwrap();
        assert!(schema.fields["data.req"].required);
    }

    #[test]
    fn test_parse_imports() {
        let schema_text = r#"
@import "types.odin"
@import "address.odin" as addr
"#;
        let schema = parse_schema(schema_text).unwrap();
        assert_eq!(schema.imports.len(), 2);
        assert_eq!(schema.imports[0].path, "types.odin");
        assert_eq!(schema.imports[1].alias.as_deref(), Some("addr"));
    }

    #[test]
    fn test_parse_metadata_with_odin_version() {
        let schema_text = "{$}\nodin = \"1.0.0\"\nschema = \"1.0.0\"\nid = \"com.example.policy\"\nversion = \"2024.1\"\ndescription = \"Auto policy schema\"";
        let schema = parse_schema(schema_text).unwrap();
        assert_eq!(schema.metadata.id.as_deref(), Some("com.example.policy"));
        assert_eq!(schema.metadata.version.as_deref(), Some("2024.1"));
        assert_eq!(schema.metadata.description.as_deref(), Some("Auto policy schema"));
    }

    #[test]
    fn test_parse_enum_constraint() {
        let schema_text = "{payment}\nmethod = :enum(\"card\", \"bank\", \"crypto\")";
        let schema = parse_schema(schema_text).unwrap();
        let method = &schema.fields["payment.method"];
        assert!(matches!(&method.constraints[0], SchemaConstraint::Enum(vals) if vals.len() == 3));
    }

    // ── Simple field with type ──────────────────────────────────────────

    #[test]
    fn test_simple_string_field() {
        let schema = parse_schema("{data}\nname = ").unwrap();
        assert!(matches!(schema.fields["data.name"].field_type, SchemaFieldType::String));
    }

    #[test]
    fn test_integer_field() {
        let schema = parse_schema("{data}\ncount = ##").unwrap();
        assert!(matches!(schema.fields["data.count"].field_type, SchemaFieldType::Integer));
    }

    #[test]
    fn test_number_field() {
        let schema = parse_schema("{data}\nprice = #").unwrap();
        assert!(matches!(schema.fields["data.price"].field_type, SchemaFieldType::Number { .. }));
    }

    #[test]
    fn test_boolean_field() {
        let schema = parse_schema("{data}\nactive = ?").unwrap();
        assert!(matches!(schema.fields["data.active"].field_type, SchemaFieldType::Boolean));
    }

    #[test]
    fn test_null_field() {
        let schema = parse_schema("{data}\nvalue = ~").unwrap();
        assert!(matches!(schema.fields["data.value"].field_type, SchemaFieldType::Null));
    }

    #[test]
    fn test_currency_field() {
        let schema = parse_schema("{data}\namount = #$").unwrap();
        assert!(matches!(schema.fields["data.amount"].field_type, SchemaFieldType::Currency { .. }));
    }

    #[test]
    fn test_date_field() {
        let schema = parse_schema("{data}\nborn = :date").unwrap();
        assert!(matches!(schema.fields["data.born"].field_type, SchemaFieldType::Date));
    }

    #[test]
    fn test_timestamp_field() {
        let schema = parse_schema("{data}\ncreated = :timestamp").unwrap();
        assert!(matches!(schema.fields["data.created"].field_type, SchemaFieldType::Timestamp));
    }

    #[test]
    fn test_time_field() {
        let schema = parse_schema("{data}\nstart = :time").unwrap();
        assert!(matches!(schema.fields["data.start"].field_type, SchemaFieldType::Time));
    }

    #[test]
    fn test_field_type_from_word_string() {
        let schema = parse_schema("{data}\nname = string").unwrap();
        assert!(matches!(schema.fields["data.name"].field_type, SchemaFieldType::String));
    }

    #[test]
    fn test_field_type_from_word_integer() {
        let schema = parse_schema("{data}\ncount = integer").unwrap();
        assert!(matches!(schema.fields["data.count"].field_type, SchemaFieldType::Integer));
    }

    #[test]
    fn test_field_type_from_word_boolean() {
        let schema = parse_schema("{data}\nflag = boolean").unwrap();
        assert!(matches!(schema.fields["data.flag"].field_type, SchemaFieldType::Boolean));
    }

    #[test]
    fn test_field_type_from_word_number() {
        let schema = parse_schema("{data}\nval = number").unwrap();
        assert!(matches!(schema.fields["data.val"].field_type, SchemaFieldType::Number { .. }));
    }

    #[test]
    fn test_field_type_from_word_currency() {
        let schema = parse_schema("{data}\nval = currency").unwrap();
        assert!(matches!(schema.fields["data.val"].field_type, SchemaFieldType::Currency { .. }));
    }

    #[test]
    fn test_field_type_from_word_duration() {
        let schema = parse_schema("{data}\nval = duration").unwrap();
        assert!(matches!(schema.fields["data.val"].field_type, SchemaFieldType::Duration));
    }

    #[test]
    fn test_field_type_from_word_percent() {
        let schema = parse_schema("{data}\nval = percent").unwrap();
        assert!(matches!(schema.fields["data.val"].field_type, SchemaFieldType::Percent));
    }

    #[test]
    fn test_field_type_from_word_binary() {
        let schema = parse_schema("{data}\nval = binary").unwrap();
        assert!(matches!(schema.fields["data.val"].field_type, SchemaFieldType::Binary));
    }

    // ── Modifier tests ──────────────────────────────────────────────────

    #[test]
    fn test_required_field_bang() {
        let schema = parse_schema("{data}\nname = !").unwrap();
        assert!(schema.fields["data.name"].required);
    }

    #[test]
    fn test_required_field_directive() {
        let schema = parse_schema("{data}\nname = :required").unwrap();
        assert!(schema.fields["data.name"].required);
    }

    #[test]
    fn test_optional_field_directive() {
        let schema = parse_schema("{data}\nname = :optional").unwrap();
        assert!(!schema.fields["data.name"].required);
    }

    #[test]
    fn test_deprecated_field() {
        let schema = parse_schema("{data}\nold = -\"legacy\"").unwrap();
        assert!(schema.fields["data.old"].deprecated);
    }

    #[test]
    fn test_deprecated_directive() {
        let schema = parse_schema("{data}\nold = :deprecated").unwrap();
        assert!(schema.fields["data.old"].deprecated);
    }

    #[test]
    fn test_deprecated_directive_with_message() {
        let schema = parse_schema("{data}\nold = :deprecated \"use new_field\"").unwrap();
        assert!(schema.fields["data.old"].deprecated);
    }

    #[test]
    fn test_confidential_field() {
        let schema = parse_schema("{data}\nssn = *").unwrap();
        assert!(schema.fields["data.ssn"].confidential);
    }

    #[test]
    fn test_immutable_directive_records_flag() {
        // `:immutable` after a field type — flag must round-trip.
        let schema = parse_schema("{user}\nid = !:format uuid :immutable\n").unwrap();
        assert!(schema.fields["user.id"].immutable);
    }

    #[test]
    fn test_immutable_with_currency_prefix() {
        // `!#$:immutable` — currency type with attached :immutable directive.
        let schema = parse_schema("{transaction}\namount = !#$:immutable\n").unwrap();
        assert!(schema.fields["transaction.amount"].immutable);
    }

    #[test]
    fn test_field_without_immutable_defaults_false() {
        let schema = parse_schema("{user}\nemail = !:format email\n").unwrap();
        assert!(!schema.fields["user.email"].immutable);
    }

    #[test]
    fn test_combined_modifiers_required_deprecated_confidential() {
        let schema = parse_schema("{data}\nfield = !-*").unwrap();
        let f = &schema.fields["data.field"];
        assert!(f.required);
        assert!(f.deprecated);
        assert!(f.confidential);
    }

    #[test]
    fn test_required_and_confidential() {
        let schema = parse_schema("{data}\nfield = !*").unwrap();
        let f = &schema.fields["data.field"];
        assert!(f.required);
        assert!(f.confidential);
        assert!(!f.deprecated);
    }

    #[test]
    fn test_required_and_deprecated() {
        let schema = parse_schema("{data}\nfield = !-\"old\"").unwrap();
        let f = &schema.fields["data.field"];
        assert!(f.required);
        assert!(f.deprecated);
        assert!(!f.confidential);
    }

    // ── String constraints ──────────────────────────────────────────────

    #[test]
    fn test_min_max_length_bounds() {
        let schema = parse_schema("{data}\nname = :(1..100)").unwrap();
        let field = &schema.fields["data.name"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Bounds { min: Some(m), max: Some(x), .. } if m == "1" && x == "100"));
    }

    #[test]
    fn test_exact_length_bounds() {
        let schema = parse_schema("{data}\nstate = :(2)").unwrap();
        let field = &schema.fields["data.state"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Bounds { min: Some(m), max: Some(x), .. } if m == "2" && x == "2"));
    }

    #[test]
    fn test_min_only_bounds() {
        let schema = parse_schema("{data}\nname = :(3..)").unwrap();
        let field = &schema.fields["data.name"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Bounds { min: Some(m), max: None, .. } if m == "3"));
    }

    #[test]
    fn test_max_only_bounds() {
        let schema = parse_schema("{data}\nname = :(..50)").unwrap();
        let field = &schema.fields["data.name"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Bounds { min: None, max: Some(x), .. } if x == "50"));
    }

    #[test]
    fn test_pattern_constraint_directive() {
        let schema = parse_schema("{data}\ncode = :pattern \"^[A-Z]{3}$\"").unwrap();
        let field = &schema.fields["data.code"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Pattern(p) if p == "^[A-Z]{3}$"));
    }

    #[test]
    fn test_pattern_constraint_slash_syntax() {
        let schema = parse_schema("{data}\ncode = :/^[0-9]+$/").unwrap();
        let field = &schema.fields["data.code"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Pattern(p) if p == "^[0-9]+$"));
    }

    // ── Number constraints ──────────────────────────────────────────────

    #[test]
    fn test_integer_with_bounds() {
        let schema = parse_schema("{data}\nage = ## :(0..150)").unwrap();
        let field = &schema.fields["data.age"];
        assert!(matches!(field.field_type, SchemaFieldType::Integer));
        assert!(matches!(&field.constraints[0], SchemaConstraint::Bounds { min: Some(m), max: Some(x), .. } if m == "0" && x == "150"));
    }

    #[test]
    fn test_number_with_bounds() {
        let schema = parse_schema("{data}\nrate = # :(0..100)").unwrap();
        let field = &schema.fields["data.rate"];
        assert!(matches!(field.field_type, SchemaFieldType::Number { .. }));
    }

    #[test]
    fn test_currency_with_bounds() {
        let schema = parse_schema("{data}\nprice = #$ :(0..99999)").unwrap();
        let field = &schema.fields["data.price"];
        assert!(matches!(field.field_type, SchemaFieldType::Currency { .. }));
    }

    // ── Enum ────────────────────────────────────────────────────────────

    #[test]
    fn test_enum_inline_values() {
        let schema = parse_schema("{data}\ncolor = :(red, green, blue)").unwrap();
        let field = &schema.fields["data.color"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Enum(vals) if vals == &["red", "green", "blue"]));
    }

    #[test]
    fn test_enum_directive_quoted() {
        let schema = parse_schema("{data}\nstatus = :enum(\"active\", \"inactive\")").unwrap();
        let field = &schema.fields["data.status"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Enum(vals) if vals.len() == 2));
    }

    #[test]
    fn test_enum_directive_with_space() {
        let schema = parse_schema("{data}\nstatus = :enum (\"a\", \"b\", \"c\")").unwrap();
        let field = &schema.fields["data.status"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Enum(vals) if vals.len() == 3));
    }

    // ── Type references ─────────────────────────────────────────────────

    #[test]
    fn test_type_ref_field() {
        let schema = parse_schema("{data}\naddr = @Address").unwrap();
        let field = &schema.fields["data.addr"];
        assert!(matches!(&field.field_type, SchemaFieldType::TypeRef(name) if name == "Address"));
    }

    // ── Format constraint ───────────────────────────────────────────────

    #[test]
    fn test_format_email_constraint() {
        let schema = parse_schema("{data}\nemail = :format email").unwrap();
        let field = &schema.fields["data.email"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Format(f) if f == "email"));
    }

    #[test]
    fn test_format_uuid_constraint() {
        let schema = parse_schema("{data}\nid = :format uuid").unwrap();
        let field = &schema.fields["data.id"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Format(f) if f == "uuid"));
    }

    #[test]
    fn test_format_zip_constraint() {
        let schema = parse_schema("{data}\nzip = :format zip").unwrap();
        let field = &schema.fields["data.zip"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Format(f) if f == "zip"));
    }

    // ── Unique constraint ───────────────────────────────────────────────

    #[test]
    fn test_unique_constraint() {
        let schema = parse_schema("{data}\nid = :unique").unwrap();
        let field = &schema.fields["data.id"];
        assert!(matches!(&field.constraints[0], SchemaConstraint::Unique));
    }

    // ── Nested sections ─────────────────────────────────────────────────

    #[test]
    fn test_nested_section() {
        let schema = parse_schema("{person.address}\nstreet = !\ncity = !").unwrap();
        assert!(schema.fields.contains_key("person.address.street"));
        assert!(schema.fields.contains_key("person.address.city"));
        assert!(schema.fields["person.address.street"].required);
    }

    // ── Schema metadata ─────────────────────────────────────────────────

    #[test]
    fn test_metadata_id() {
        let schema = parse_schema("{$}\nid = \"my-schema\"").unwrap();
        assert_eq!(schema.metadata.id.as_deref(), Some("my-schema"));
    }

    #[test]
    fn test_metadata_title() {
        let schema = parse_schema("{$}\ntitle = \"My Schema Title\"").unwrap();
        assert_eq!(schema.metadata.title.as_deref(), Some("My Schema Title"));
    }

    #[test]
    fn test_metadata_description() {
        let schema = parse_schema("{$}\ndescription = \"Describes things\"").unwrap();
        assert_eq!(schema.metadata.description.as_deref(), Some("Describes things"));
    }

    #[test]
    fn test_metadata_version() {
        let schema = parse_schema("{$}\nversion = \"1.0.0\"").unwrap();
        assert_eq!(schema.metadata.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn test_metadata_all_fields() {
        let schema = parse_schema("{$}\nid = \"test\"\ntitle = \"Title\"\ndescription = \"Desc\"\nversion = \"2.0\"").unwrap();
        assert_eq!(schema.metadata.id.as_deref(), Some("test"));
        assert_eq!(schema.metadata.title.as_deref(), Some("Title"));
        assert_eq!(schema.metadata.description.as_deref(), Some("Desc"));
        assert_eq!(schema.metadata.version.as_deref(), Some("2.0"));
    }

    // ── Multiple sections ───────────────────────────────────────────────

    #[test]
    fn test_multiple_sections() {
        let schema = parse_schema("{person}\nname = !\n\n{address}\nstreet = !\ncity = !").unwrap();
        assert!(schema.fields.contains_key("person.name"));
        assert!(schema.fields.contains_key("address.street"));
        assert!(schema.fields.contains_key("address.city"));
    }

    // ── Comments ────────────────────────────────────────────────────────

    #[test]
    fn test_comments_are_ignored() {
        let schema = parse_schema("; This is a comment\n{data}\n; Another comment\nname = !").unwrap();
        assert!(schema.fields.contains_key("data.name"));
        assert_eq!(schema.fields.len(), 1);
    }

    #[test]
    fn test_blank_lines_are_ignored() {
        let schema = parse_schema("\n\n{data}\n\nname = !\n\n").unwrap();
        assert!(schema.fields.contains_key("data.name"));
    }

    // ── Inline comment stripping ────────────────────────────────────────

    #[test]
    fn test_inline_comment_stripped() {
        let schema = parse_schema("{data}\nname = ! ; this is required").unwrap();
        assert!(schema.fields["data.name"].required);
    }

    // ── Array field ─────────────────────────────────────────────────────

    #[test]
    fn test_array_section() {
        let schema = parse_schema("{items[]}\nname = !\nprice = #").unwrap();
        assert!(schema.fields.contains_key("items[].name"));
        assert!(schema.fields.contains_key("items[].price"));
    }

    // ── Type inheritance ────────────────────────────────────────────────

    #[test]
    fn test_type_inheritance_single_parent() {
        let schema = parse_schema("@Base\nid = !\n\n@Child : @Base\nname = !").unwrap();
        assert!(schema.types.contains_key("Base"));
        assert!(schema.types.contains_key("Child"));
        let child = &schema.types["Child"];
        assert!(!child.parents.is_empty());
        assert!(child.parents.iter().any(|p| p.contains("Base")));
    }

    #[test]
    fn test_type_inheritance_multiple_parents() {
        let schema = parse_schema("@A\nf1 = !\n\n@B\nf2 = !\n\n@C : @A & @B\nf3 = !").unwrap();
        let c = &schema.types["C"];
        assert_eq!(c.parents.len(), 2);
    }

    // ── Top-level fields (no section) ───────────────────────────────────

    #[test]
    fn test_top_level_field_no_section() {
        let schema = parse_schema("name = !\nemail = :format email").unwrap();
        assert!(schema.fields.contains_key("name"));
        assert!(schema.fields.contains_key("email"));
        assert!(schema.fields["name"].required);
    }

    // ── Empty schema ────────────────────────────────────────────────────

    #[test]
    fn test_empty_schema() {
        let schema = parse_schema("").unwrap();
        assert!(schema.fields.is_empty());
        assert!(schema.types.is_empty());
        assert!(schema.imports.is_empty());
    }

    #[test]
    fn test_comments_only_schema() {
        let schema = parse_schema("; just a comment\n; another one").unwrap();
        assert!(schema.fields.is_empty());
    }

    // ── Object constraints ──────────────────────────────────────────────

    #[test]
    fn test_exactly_one_constraint() {
        let schema = parse_schema("{contact}\n:exactly_one(email, phone)\nemail = \nphone = ").unwrap();
        let constraints = &schema.constraints["contact"];
        assert_eq!(constraints.len(), 1);
        assert!(matches!(&constraints[0], SchemaObjectConstraint::Cardinality { min: Some(1), max: Some(1), fields } if fields.len() == 2));
    }

    #[test]
    fn test_at_most_one_constraint() {
        let schema = parse_schema("{contact}\n:at_most_one(email, phone)\nemail = \nphone = ").unwrap();
        let constraints = &schema.constraints["contact"];
        assert!(matches!(&constraints[0], SchemaObjectConstraint::Cardinality { min: None, max: Some(1), .. }));
    }

    #[test]
    fn test_invariant_constraint() {
        let schema = parse_schema("{data}\n:invariant age >= 0").unwrap();
        let constraints = &schema.constraints["data"];
        assert!(matches!(&constraints[0], SchemaObjectConstraint::Invariant(expr) if expr == "age >= 0"));
    }

    // ── Import with and without alias ───────────────────────────────────

    #[test]
    fn test_import_without_alias() {
        let schema = parse_schema("@import \"common.odin\"").unwrap();
        assert_eq!(schema.imports.len(), 1);
        assert_eq!(schema.imports[0].path, "common.odin");
        assert!(schema.imports[0].alias.is_none());
    }

    #[test]
    fn test_import_with_alias() {
        let schema = parse_schema("@import \"types.odin\" as types").unwrap();
        assert_eq!(schema.imports[0].alias.as_deref(), Some("types"));
    }

    // ── Type definition in header syntax ────────────────────────────────

    #[test]
    fn test_type_def_header_syntax() {
        let schema = parse_schema("{@Person}\nname = !\nage = ##").unwrap();
        assert!(schema.types.contains_key("Person"));
        let person = &schema.types["Person"];
        assert_eq!(person.fields.len(), 2);
        assert_eq!(person.fields[0].name, "name");
        assert_eq!(person.fields[1].name, "age");
    }

    // ── Combined type + required + bounds ───────────────────────────────

    #[test]
    fn test_required_integer_with_bounds() {
        let schema = parse_schema("{data}\nage = ! ## :(0..150)").unwrap();
        let field = &schema.fields["data.age"];
        assert!(field.required);
        assert!(matches!(field.field_type, SchemaFieldType::Integer));
        assert!(!field.constraints.is_empty());
    }

    #[test]
    fn test_confidential_string_with_format() {
        let schema = parse_schema("{data}\nssn = * :format ssn").unwrap();
        let field = &schema.fields["data.ssn"];
        assert!(field.confidential);
        assert!(matches!(&field.constraints[0], SchemaConstraint::Format(f) if f == "ssn"));
    }

    // ── Helper function tests ───────────────────────────────────────────

    #[test]
    fn test_unquote_with_quotes() {
        assert_eq!(unquote("\"hello\""), "hello");
    }

    #[test]
    fn test_unquote_without_quotes() {
        assert_eq!(unquote("hello"), "hello");
    }

    #[test]
    fn test_unquote_empty_string() {
        assert_eq!(unquote("\"\""), "");
    }

    #[test]
    fn test_find_unquoted_semicolon_outside_quotes() {
        assert_eq!(find_unquoted_semicolon("hello ; world"), Some(6));
    }

    #[test]
    fn test_find_unquoted_semicolon_inside_quotes() {
        assert_eq!(find_unquoted_semicolon("\"hello ; world\""), None);
    }

    #[test]
    fn test_find_unquoted_semicolon_none() {
        assert_eq!(find_unquoted_semicolon("hello world"), None);
    }

    #[test]
    fn test_extract_quoted_basic() {
        let result = extract_quoted("\"hello\" rest");
        assert_eq!(result, Some(("hello", 7)));
    }

    #[test]
    fn test_extract_quoted_no_quotes() {
        assert!(extract_quoted("hello").is_none());
    }

    #[test]
    fn test_parse_range_both() {
        assert_eq!(parse_range("1..100"), (Some(1), Some(100)));
    }

    #[test]
    fn test_parse_range_min_only() {
        assert_eq!(parse_range("3.."), (Some(3), None));
    }

    #[test]
    fn test_parse_range_max_only() {
        assert_eq!(parse_range("..50"), (None, Some(50)));
    }

    // ── Conformance fixes ───────────────────────────────────────────────

    #[test]
    fn test_percent_type_first_class() {
        let s = parse_schema("{root}\ntax = #%").unwrap();
        assert!(matches!(s.fields["root.tax"].field_type, SchemaFieldType::Percent));
    }

    #[test]
    fn test_typed_default_integer() {
        let s = parse_schema("{root}\na = ##3").unwrap();
        let f = &s.fields["root.a"];
        assert!(matches!(f.field_type, SchemaFieldType::Integer));
        assert_eq!(f.default_value, Some(SchemaDefault::Integer(3)));
    }

    #[test]
    fn test_typed_default_number_currency_percent() {
        let s = parse_schema("{root}\nb = #0.05\nc = #$5.00\np = #%0.15").unwrap();
        assert_eq!(s.fields["root.b"].default_value, Some(SchemaDefault::Number(0.05)));
        assert_eq!(s.fields["root.c"].default_value, Some(SchemaDefault::Currency(5.0)));
        assert_eq!(s.fields["root.p"].default_value, Some(SchemaDefault::Percent(0.15)));
    }

    #[test]
    fn test_constrained_default_after_bounds() {
        let s = parse_schema("{root}\npriority = ##:(1..5) ##3").unwrap();
        let f = &s.fields["root.priority"];
        assert_eq!(f.default_value, Some(SchemaDefault::Integer(3)));
        assert!(matches!(&f.constraints[0],
            SchemaConstraint::Bounds { min: Some(m), max: Some(x), .. } if m == "1" && x == "5"));
    }

    #[test]
    fn test_union_date_timestamp() {
        let s = parse_schema("{root}\nu = date|timestamp").unwrap();
        match &s.fields["root.u"].field_type {
            SchemaFieldType::Union(m) => {
                assert!(matches!(m[0], SchemaFieldType::Date));
                assert!(matches!(m[1], SchemaFieldType::Timestamp));
            }
            other => panic!("expected union, got {other:?}"),
        }
    }

    #[test]
    fn test_union_number_null() {
        let s = parse_schema("{root}\nn = #|~").unwrap();
        match &s.fields["root.n"].field_type {
            SchemaFieldType::Union(m) => {
                assert!(matches!(m[0], SchemaFieldType::Number { .. }));
                assert!(matches!(m[1], SchemaFieldType::Null));
            }
            other => panic!("expected union, got {other:?}"),
        }
    }

    #[test]
    fn test_temporal_bounds_preserve_dates() {
        let s = parse_schema("{root}\nd = date:(2020-06-15..2020-06-20)").unwrap();
        let f = &s.fields["root.d"];
        assert!(matches!(f.field_type, SchemaFieldType::Date));
        assert!(matches!(&f.constraints[0],
            SchemaConstraint::Bounds { min: Some(m), max: Some(x), .. }
            if m == "2020-06-15" && x == "2020-06-20"));
    }

    #[test]
    fn test_temporal_glued_immutable_keeps_type() {
        let s = parse_schema("{root}\ncreated_at = !timestamp:immutable").unwrap();
        let f = &s.fields["root.created_at"];
        assert!(matches!(f.field_type, SchemaFieldType::Timestamp));
        assert!(f.required);
        assert!(f.immutable);
    }

    #[test]
    fn test_temporal_glued_computed_keeps_type() {
        let s = parse_schema("{root}\nstamp = date:computed").unwrap();
        let f = &s.fields["root.stamp"];
        assert!(matches!(f.field_type, SchemaFieldType::Date));
        assert!(f.computed);
    }

    #[test]
    fn test_pattern_then_if_conditional() {
        let s = parse_schema("{root}\nfield = !:/^[a-z]+$/:if method = paypal").unwrap();
        let f = &s.fields["root.field"];
        assert!(f.required);
        assert!(matches!(&f.constraints[0], SchemaConstraint::Pattern(p) if p == "^[a-z]+$"));
        assert_eq!(f.conditionals.len(), 1);
        let c = &f.conditionals[0];
        assert_eq!(c.field, "method");
        assert!(matches!(c.operator, ConditionalOperator::Eq));
        assert!(matches!(&c.value, ConditionalValue::String(v) if v == "paypal"));
    }

    #[test]
    fn test_type_intersection_section_composition() {
        let s = parse_schema(
            "{@hasName}\nname = !\n\n{@hasAge}\nage = !##\n\n{customer}\n= @hasName & @hasAge",
        ).unwrap();
        let f = &s.fields["customer._composition"];
        assert!(matches!(&f.field_type, SchemaFieldType::TypeRef(n) if n == "hasName&hasAge"));
    }
}
