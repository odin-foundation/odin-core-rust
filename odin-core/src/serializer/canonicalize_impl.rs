//! Canonical serialization — deterministic, byte-identical output.
//!
//! The canonical form is used for hashing, signatures, and deduplication.
//! It guarantees that semantically equivalent documents produce identical bytes.

use crate::types::document::OdinDocument;

/// Produce canonical bytes for a document.
///
/// Rules:
/// - All keys sorted alphabetically
/// - No trailing whitespace
/// - Consistent value formatting
/// - UTF-8 encoded
pub fn canonicalize(doc: &OdinDocument) -> Vec<u8> {
    let opts = super::StringifyOptions {
        pretty: false,
        indent: String::new(),
        include_metadata: true,
        sort_keys: true,
        use_tabular: false,
        canonical: true,
    };
    let text = super::stringify_impl::stringify(doc, Some(&opts));
    text.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::values::{OdinValues, OdinModifiers};
    use crate::OdinDocumentBuilder;
    use crate::Odin;

    // ── Determinism ─────────────────────────────────────────────────────────

    #[test]
    fn same_document_same_output() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .set("age", OdinValues::integer(30))
            .build()
            .unwrap();
        let a = canonicalize(&doc);
        let b = canonicalize(&doc);
        assert_eq!(a, b);
    }

    #[test]
    fn identical_documents_identical_bytes() {
        let doc1 = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::string("hello"))
            .build()
            .unwrap();
        let doc2 = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::string("hello"))
            .build()
            .unwrap();
        assert_eq!(canonicalize(&doc1), canonicalize(&doc2));
    }

    #[test]
    fn multiple_calls_stable() {
        let doc = OdinDocumentBuilder::new()
            .set("a", OdinValues::boolean(true))
            .set("b", OdinValues::null())
            .build()
            .unwrap();
        let results: Vec<Vec<u8>> = (0..5).map(|_| canonicalize(&doc)).collect();
        for r in &results[1..] {
            assert_eq!(&results[0], r);
        }
    }

    // ── Key ordering ────────────────────────────────────────────────────────

    #[test]
    fn keys_sorted_alphabetically() {
        let doc = OdinDocumentBuilder::new()
            .set("z_last", OdinValues::integer(3))
            .set("a_first", OdinValues::integer(1))
            .set("m_mid", OdinValues::integer(2))
            .build()
            .unwrap();
        let out = String::from_utf8(canonicalize(&doc)).unwrap();
        let a_pos = out.find("a_first").unwrap();
        let m_pos = out.find("m_mid").unwrap();
        let z_pos = out.find("z_last").unwrap();
        assert!(a_pos < m_pos);
        assert!(m_pos < z_pos);
    }

    #[test]
    fn different_insertion_order_same_canonical() {
        let doc1 = OdinDocumentBuilder::new()
            .set("b", OdinValues::integer(2))
            .set("a", OdinValues::integer(1))
            .build()
            .unwrap();
        let doc2 = OdinDocumentBuilder::new()
            .set("a", OdinValues::integer(1))
            .set("b", OdinValues::integer(2))
            .build()
            .unwrap();
        assert_eq!(canonicalize(&doc1), canonicalize(&doc2));
    }

    // ── UTF-8 encoding ──────────────────────────────────────────────────────

    #[test]
    fn output_is_valid_utf8() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .build()
            .unwrap();
        let bytes = canonicalize(&doc);
        assert!(std::str::from_utf8(&bytes).is_ok());
    }

    // ── Empty document ──────────────────────────────────────────────────────

    #[test]
    fn empty_document_canonical() {
        let doc = OdinDocument::empty();
        let out = canonicalize(&doc);
        assert!(out.is_empty());
    }

    // ── Metadata included ───────────────────────────────────────────────────

    #[test]
    fn canonical_includes_metadata() {
        let doc = OdinDocumentBuilder::new()
            .metadata("odin", OdinValues::string("1.0.0"))
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let out = String::from_utf8(canonicalize(&doc)).unwrap();
        // Canonical form uses $.key prefix, not {$} section
        assert!(out.contains("$.odin = \"1.0.0\""));
        assert!(!out.contains("{$}"));
    }

    // ── Modifiers in canonical form ─────────────────────────────────────────

    #[test]
    fn modifier_ordering_canonical() {
        let mods = OdinModifiers {
            required: true,
            confidential: true,
            deprecated: true,
            attr: false,
        };
        let doc = OdinDocumentBuilder::new()
            .set("field", OdinValues::string("val").with_modifiers(mods))
            .build()
            .unwrap();
        let out = String::from_utf8(canonicalize(&doc)).unwrap();
        assert!(out.contains("!*-\"val\""));
    }

    // ── Sections ────────────────────────────────────────────────────────────

    #[test]
    fn sections_sorted_in_canonical() {
        let doc = OdinDocumentBuilder::new()
            .set("Zebra.name", OdinValues::string("z"))
            .set("Alpha.name", OdinValues::string("a"))
            .build()
            .unwrap();
        let out = String::from_utf8(canonicalize(&doc)).unwrap();
        let alpha_pos = out.find("Alpha").unwrap();
        let zebra_pos = out.find("Zebra").unwrap();
        assert!(alpha_pos < zebra_pos);
    }

    #[test]
    fn fields_within_section_sorted() {
        let doc = OdinDocumentBuilder::new()
            .set("Policy.z_field", OdinValues::integer(2))
            .set("Policy.a_field", OdinValues::integer(1))
            .build()
            .unwrap();
        let out = String::from_utf8(canonicalize(&doc)).unwrap();
        let a_pos = out.find("a_field").unwrap();
        let z_pos = out.find("z_field").unwrap();
        assert!(a_pos < z_pos);
    }

    // ── Parse-canonicalize roundtrip ────────────────────────────────────────

    #[test]
    fn parse_canonicalize_roundtrip() {
        let input = "name = \"Alice\"\nage = ##30\n";
        let doc = Odin::parse(input).unwrap();
        let canonical1 = Odin::canonicalize(&doc);
        // Parse the canonical output
        let text = String::from_utf8(canonical1.clone()).unwrap();
        let doc2 = Odin::parse(&text).unwrap();
        let canonical2 = Odin::canonicalize(&doc2);
        assert_eq!(canonical1, canonical2);
    }

    #[test]
    fn roundtrip_with_metadata() {
        let input = "{$}\nodin = \"1.0.0\"\n\nname = \"Bob\"\n";
        let doc = Odin::parse(input).unwrap();
        let c1 = Odin::canonicalize(&doc);
        let text = String::from_utf8(c1.clone()).unwrap();
        let doc2 = Odin::parse(&text).unwrap();
        let c2 = Odin::canonicalize(&doc2);
        assert_eq!(c1, c2);
    }

    #[test]
    fn roundtrip_with_types() {
        let doc = OdinDocumentBuilder::new()
            .set("flag", OdinValues::boolean(true))
            .set("count", OdinValues::integer(42))
            .set("label", OdinValues::string("test"))
            .set("empty", OdinValues::null())
            .build()
            .unwrap();
        let c1 = Odin::canonicalize(&doc);
        let text = String::from_utf8(c1.clone()).unwrap();
        let doc2 = Odin::parse(&text).unwrap();
        let c2 = Odin::canonicalize(&doc2);
        assert_eq!(c1, c2);
    }

    // ── Different values produce different canonical ─────────────────────────

    #[test]
    fn different_values_different_canonical() {
        let doc1 = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let doc2 = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(2))
            .build()
            .unwrap();
        assert_ne!(canonicalize(&doc1), canonicalize(&doc2));
    }

    #[test]
    fn different_keys_different_canonical() {
        let doc1 = OdinDocumentBuilder::new()
            .set("a", OdinValues::integer(1))
            .build()
            .unwrap();
        let doc2 = OdinDocumentBuilder::new()
            .set("b", OdinValues::integer(1))
            .build()
            .unwrap();
        assert_ne!(canonicalize(&doc1), canonicalize(&doc2));
    }

    // ── Whitespace normalization ────────────────────────────────────────────

    #[test]
    fn no_trailing_whitespace() {
        let doc = OdinDocumentBuilder::new()
            .set("a", OdinValues::string("x"))
            .set("b", OdinValues::integer(1))
            .build()
            .unwrap();
        let out = String::from_utf8(canonicalize(&doc)).unwrap();
        for line in out.lines() {
            assert_eq!(line, line.trim_end(), "Line has trailing whitespace: {line:?}");
        }
    }

    // ── Multiple fields stability ───────────────────────────────────────────

    #[test]
    fn many_fields_deterministic() {
        let mut builder = OdinDocumentBuilder::new();
        for i in (0..20).rev() {
            builder = builder.set(&format!("field_{i:02}"), OdinValues::integer(i));
        }
        let doc = builder.build().unwrap();
        let c1 = canonicalize(&doc);
        let c2 = canonicalize(&doc);
        assert_eq!(c1, c2);
        // Verify sorted
        let text = String::from_utf8(c1).unwrap();
        let f00 = text.find("field_00").unwrap();
        let f19 = text.find("field_19").unwrap();
        assert!(f00 < f19);
    }
}
