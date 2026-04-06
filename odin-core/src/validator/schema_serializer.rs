//! Schema serializer — converts an `OdinSchemaDefinition` back to ODIN schema text.
//!
//! Enables round-tripping: parse schema → modify → serialize back to text.

use std::fmt::Write;

use crate::types::schema::{
    OdinSchemaDefinition, SchemaConstraint, SchemaField, SchemaFieldType,
    SchemaObjectConstraint, SchemaType,
};

/// Serialize an `OdinSchemaDefinition` to ODIN schema text.
pub fn serialize_schema(schema: &OdinSchemaDefinition) -> String {
    let mut output = String::new();

    // Metadata section
    output.push_str("{$}\n");
    if let Some(ref id) = schema.metadata.id {
        let _ = writeln!(output, "id = \"{id}\"");
    }
    if let Some(ref title) = schema.metadata.title {
        let _ = writeln!(output, "title = \"{title}\"");
    }
    if let Some(ref description) = schema.metadata.description {
        let _ = writeln!(output, "description = \"{description}\"");
    }
    if let Some(ref version) = schema.metadata.version {
        let _ = writeln!(output, "version = \"{version}\"");
    }
    output.push('\n');

    // Imports
    for import in &schema.imports {
        if let Some(ref alias) = import.alias {
            let _ = writeln!(output, "@import \"{}\" as {}", import.path, alias);
        } else {
            let _ = writeln!(output, "@import \"{}\"", import.path);
        }
    }
    if !schema.imports.is_empty() {
        output.push('\n');
    }

    // Type definitions
    for (name, schema_type) in &schema.types {
        serialize_type(&mut output, name, schema_type);
        output.push('\n');
    }

    // Top-level fields (organized by section)
    let mut sections: Vec<(String, Vec<(&String, &SchemaField)>)> = Vec::new();
    for (path, field) in &schema.fields {
        if let Some(dot_pos) = path.find('.') {
            let section = &path[..dot_pos];
            if let Some(idx) = sections.iter().position(|(s, _)| s == section) {
                sections[idx].1.push((path, field));
            } else {
                sections.push((section.to_string(), vec![(path, field)]));
            }
        } else {
            // Root-level field (no section)
            if sections.is_empty() || !sections[0].0.is_empty() {
                sections.insert(0, (String::new(), vec![(path, field)]));
            } else {
                sections[0].1.push((path, field));
            }
        }
    }

    for (section, fields) in &sections {
        if !section.is_empty() {
            let _ = writeln!(output, "{{{section}}}");
        }
        for (path, field) in fields {
            let field_name = if section.is_empty() {
                (*path).clone()
            } else {
                path.strip_prefix(&format!("{section}."))
                    .unwrap_or(path)
                    .to_string()
            };
            serialize_field(&mut output, &field_name, field);
        }
    }

    // Array definitions
    for (path, array_def) in &schema.arrays {
        let _ = write!(output, "{path}[] = ");
        output.push_str(&format_field_type(&array_def.item_type));
        if let Some(min) = array_def.min_items {
            if let Some(max) = array_def.max_items {
                let _ = write!(output, " :({min}..{max})");
            } else {
                let _ = write!(output, " :({min}..)");
            }
        } else if let Some(max) = array_def.max_items {
            let _ = write!(output, " :(..{max})");
        }
        if array_def.unique {
            output.push_str(" :unique");
        }
        output.push('\n');
    }

    // Object constraints
    for (path, constraints) in &schema.constraints {
        for constraint in constraints {
            match constraint {
                SchemaObjectConstraint::Invariant(expr) => {
                    let _ = writeln!(output, "; invariant at {path}: {expr}");
                }
                SchemaObjectConstraint::Cardinality { fields, min, max } => {
                    let fields_str = fields.join(", ");
                    match (min, max) {
                        (Some(1), Some(1)) => {
                            let _ = writeln!(output, ":exactly_one({fields_str}) ; at {path}");
                        }
                        (Some(1), None) => {
                            let _ = writeln!(output, ":one_of({fields_str}) ; at {path}");
                        }
                        (None, Some(1)) => {
                            let _ = writeln!(output, ":at_most_one({fields_str}) ; at {path}");
                        }
                        _ => {
                            let _ = writeln!(output, "; cardinality({fields_str}) at {path}: min={min:?} max={max:?}");
                        }
                    }
                }
            }
        }
    }

    output
}

fn serialize_type(output: &mut String, name: &str, schema_type: &SchemaType) {
    // Type header
    output.push('@');
    output.push_str(name);
    if !schema_type.parents.is_empty() {
        output.push_str(" : ");
        output.push_str(&schema_type.parents.join(" & "));
    }
    output.push('\n');

    // Type fields
    for field in &schema_type.fields {
        serialize_field(output, &field.name, field);
    }
}

fn serialize_field(output: &mut String, name: &str, field: &SchemaField) {
    output.push_str(name);
    output.push_str(" = ");

    // Type prefix
    output.push_str(&format_field_type(&field.field_type));

    // Constraints
    for constraint in &field.constraints {
        match constraint {
            SchemaConstraint::Bounds { min, max, min_exclusive, max_exclusive } => {
                let min_bracket = if *min_exclusive { "(" } else { "[" };
                let max_bracket = if *max_exclusive { ")" } else { "]" };
                match (min, max) {
                    (Some(min_val), Some(max_val)) => {
                        let _ = write!(output, " :{min_bracket}{min_val}..{max_val}{max_bracket}");
                    }
                    (Some(min_val), None) => {
                        let _ = write!(output, " :{min_bracket}{min_val}..{max_bracket}");
                    }
                    (None, Some(max_val)) => {
                        let _ = write!(output, " :{min_bracket}..{max_val}{max_bracket}");
                    }
                    (None, None) => {}
                }
            }
            SchemaConstraint::Pattern(pat) => {
                let _ = write!(output, " :pattern \"{pat}\"");
            }
            SchemaConstraint::Enum(values) => {
                let vals: Vec<String> = values.iter().map(|v| format!("\"{v}\"")).collect();
                let _ = write!(output, " :enum({})", vals.join(", "));
            }
            SchemaConstraint::Format(fmt) => {
                let _ = write!(output, " :format {fmt}");
            }
            SchemaConstraint::Unique => {
                output.push_str(" :unique");
            }
            SchemaConstraint::Size { min, max } => {
                match (min, max) {
                    (Some(min_val), Some(max_val)) => {
                        let _ = write!(output, " :size({min_val}..{max_val})");
                    }
                    (Some(min_val), None) => {
                        let _ = write!(output, " :size({min_val}..)");
                    }
                    (None, Some(max_val)) => {
                        let _ = write!(output, " :size(..{max_val})");
                    }
                    (None, None) => {}
                }
            }
        }
    }

    // Modifiers
    if field.required {
        output.push_str(" :required");
    }
    if field.confidential {
        output.push_str(" :confidential");
    }
    if field.deprecated {
        output.push_str(" :deprecated");
    }

    output.push('\n');
}

fn format_field_type(ft: &SchemaFieldType) -> String {
    match ft {
        SchemaFieldType::String => "\"\"".to_string(),
        SchemaFieldType::Boolean => "?".to_string(),
        SchemaFieldType::Null => "~".to_string(),
        SchemaFieldType::Number { .. } | SchemaFieldType::Decimal { .. } => "#".to_string(),
        SchemaFieldType::Integer => "##".to_string(),
        SchemaFieldType::Currency { .. } => "#$".to_string(),
        SchemaFieldType::Date => ":date".to_string(),
        SchemaFieldType::Timestamp => ":timestamp".to_string(),
        SchemaFieldType::Time => ":time".to_string(),
        SchemaFieldType::Duration => ":duration".to_string(),
        SchemaFieldType::Percent => "#%".to_string(),
        SchemaFieldType::Binary => "^".to_string(),
        SchemaFieldType::Enum(values) => {
            let vals: Vec<String> = values.iter().map(|v| format!("\"{v}\"")).collect();
            vals.join("|")
        }
        SchemaFieldType::Union(members) => {
            members.iter().map(format_field_type).collect::<Vec<_>>().join("|")
        }
        SchemaFieldType::Reference(name) | SchemaFieldType::TypeRef(name) => format!("@{name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::schema::*;
    use std::collections::HashMap;

    #[test]
    fn test_serialize_empty_schema() {
        let schema = OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };

        let text = serialize_schema(&schema);
        assert!(text.contains("{$}"));
    }

    #[test]
    fn test_serialize_type_with_fields() {
        let schema = OdinSchemaDefinition {
            metadata: SchemaMetadata {
                title: Some("Test Schema".to_string()),
                ..Default::default()
            },
            imports: vec![],
            types: {
                let mut types = HashMap::new();
                types.insert("Person".to_string(), SchemaType {
                    name: "Person".to_string(),
                    description: None,
                    fields: vec![
                        SchemaField {
                            name: "name".to_string(),
                            field_type: SchemaFieldType::String,
                            required: true,
                            confidential: false,
                            deprecated: false,
                            description: None,
                            constraints: vec![],
                            default_value: None,
                            conditionals: vec![],
                        },
                        SchemaField {
                            name: "age".to_string(),
                            field_type: SchemaFieldType::Integer,
                            required: false,
                            confidential: false,
                            deprecated: false,
                            description: None,
                            constraints: vec![SchemaConstraint::Bounds {
                                min: Some("0".to_string()),
                                max: Some("150".to_string()),
                                min_exclusive: false,
                                max_exclusive: false,
                            }],
                            default_value: None,
                            conditionals: vec![],
                        },
                    ],
                    parents: vec![],
                });
                types
            },
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
        };

        let text = serialize_schema(&schema);
        assert!(text.contains("@Person"));
        assert!(text.contains("name = \"\" :required"));
        assert!(text.contains("age = ## :[0..150]"));
    }

    #[test]
    fn test_roundtrip_schema() {
        let schema_text = "@PhoneNumber\n= :(10..15)\n";
        let parsed = crate::validator::schema_parser::parse_schema(schema_text).unwrap();
        let serialized = serialize_schema(&parsed);
        assert!(serialized.contains("PhoneNumber"));
    }
}
