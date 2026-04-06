# odin-core

[![crates.io](https://img.shields.io/crates/v/odin-core)](https://crates.io/crates/odin-core) [![License](https://img.shields.io/crates/l/odin-core)](https://github.com/odin-foundation/odin-core-rust/blob/main/LICENSE)

Official Rust SDK for [ODIN](https://odin.foundation) (Open Data Interchange Notation) — a canonical data model for transporting meaning between systems, standards, and AI.

## Install

```bash
cargo add odin-core
```

Or add to `Cargo.toml`:

```toml
[dependencies]
odin-core = "1.0"
```

**Edition 2021 — MSRV 1.75**

## Quick Start

```rust
use odin_core::Odin;

fn main() -> Result<(), odin_core::Error> {
    let doc = Odin::parse(r#"
{policy}
number = "PAP-2024-001"
effective = 2024-06-01
premium = #$747.50
active = ?true
"#)?;

    println!("{}", doc.get("policy.number")?);   // "PAP-2024-001"
    println!("{}", doc.get("policy.premium")?);   // 747.50

    let text = Odin::stringify(&doc);
    Ok(())
}
```

## Core API

| Function | Description | Example |
|----------|-------------|---------|
| `Odin::parse(text)` | Parse ODIN text into a document | `let doc = Odin::parse(src)?;` |
| `Odin::stringify(&doc)` | Serialize document to ODIN text | `let text = Odin::stringify(&doc);` |
| `Odin::canonicalize(&doc)` | Deterministic bytes for hashing/signatures | `let bytes = Odin::canonicalize(&doc);` |
| `Odin::validate(&doc, &schema)` | Validate against an ODIN schema | `let result = Odin::validate(&doc, &schema);` |
| `Odin::parse_schema(text)` | Parse a schema definition | `let schema = Odin::parse_schema(src)?;` |
| `Odin::diff(&a, &b)` | Structured diff between two documents | `let changes = Odin::diff(&a, &b);` |
| `Odin::patch(&doc, &diff)` | Apply a diff to a document | `let updated = Odin::patch(&doc, &changes)?;` |
| `Odin::parse_transform(text)` | Parse a transform specification | `let tx = Odin::parse_transform(src)?;` |
| `Odin::execute_transform(&tx, &src)` | Run a transform on data | `let out = Odin::execute_transform(&tx, &doc)?;` |
| `doc.to_json()` | Export to JSON | `let json = doc.to_json()?;` |
| `doc.to_xml()` | Export to XML | `let xml = doc.to_xml()?;` |
| `doc.to_csv()` | Export to CSV | `let csv = doc.to_csv()?;` |
| `Odin::stringify(&doc, None)` | Export to ODIN | `let odin = Odin::stringify(&doc, None)?;` |
| `Odin::builder()` | Fluent document builder | `Odin::builder().section("policy")...` |

## Schema Validation

```rust
use odin_core::Odin;

let schema = Odin::parse_schema(r#"
{policy}
!number : string
!effective : date
!premium : currency
active : boolean
"#)?;

let doc = Odin::parse(source)?;
let result = Odin::validate(&doc, &schema);

if !result.is_valid() {
    for error in result.errors() {
        eprintln!("{}", error);
    }
}
```

## Transforms

```rust
use odin_core::Odin;

let transform = Odin::parse_transform(r#"
map policy -> record
  policy.number -> record.id
  policy.premium -> record.amount
"#)?;

let result = Odin::execute_transform(&transform, &doc)?;
```

## Export

```rust
let odin = Odin::stringify(&doc, None)?; // ODIN String
let json = doc.to_json()?;              // JSON String
let xml  = doc.to_xml()?;               // XML String
let csv  = doc.to_csv()?;               // CSV String
```

## Builder

```rust
let doc = Odin::builder()
    .section("policy")
    .set("number", "PAP-2024-001")
    .set("effective", NaiveDate::from_ymd_opt(2024, 6, 1).unwrap())
    .set_currency("premium", 747.50)
    .set("active", true)
    .build()?;
```

## Testing

Tests use the built-in test framework and the shared golden test suite:

```bash
cargo test
```

## Links

- [.Odin Foundation Website](https://odin.foundation)
- [GitHub](https://github.com/odin-foundation/odin)
- [Golden Test Suite](https://github.com/odin-foundation/odin/tree/main/sdk/golden)
- [License (Apache 2.0)](https://github.com/odin-foundation/odin/blob/main/LICENSE)
