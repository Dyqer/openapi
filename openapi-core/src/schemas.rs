//! Bundled OpenAPI JSON Schemas (see `schemas/SOURCES.md`).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use crate::detect::OpenApiVersion;

const V2: &str = include_str!("../schemas/openapi-2.0.json");
const V3_0: &str = include_str!("../schemas/openapi-3.0.json");
const V3_1: &str = include_str!("../schemas/openapi-3.1.json");

/// The JSON Schema 2020-12 dialect and its vocabularies. OAS 3.1 hands Schema
/// Objects over to it with `$dynamicRef: "#meta"`, so without these a 3.1
/// Schema Object would describe no keys at all.
const DIALECT: &[&str] = &[
    include_str!("../schemas/json-schema-2020-12/schema.json"),
    include_str!("../schemas/json-schema-2020-12/core.json"),
    include_str!("../schemas/json-schema-2020-12/applicator.json"),
    include_str!("../schemas/json-schema-2020-12/validation.json"),
    include_str!("../schemas/json-schema-2020-12/meta-data.json"),
    include_str!("../schemas/json-schema-2020-12/format-annotation.json"),
    include_str!("../schemas/json-schema-2020-12/content.json"),
    include_str!("../schemas/json-schema-2020-12/unevaluated.json"),
];

pub const DIALECT_2020_12_ID: &str = "https://json-schema.org/draft/2020-12/schema";

/// Schema describing documents of `version`, parsed once per process.
pub fn schema_for(version: OpenApiVersion) -> Option<&'static Value> {
    static SWAGGER_2: OnceLock<Option<Value>> = OnceLock::new();
    static OAS_3_0: OnceLock<Option<Value>> = OnceLock::new();
    static OAS_3_1: OnceLock<Option<Value>> = OnceLock::new();

    match version {
        OpenApiVersion::V2 => parsed(&SWAGGER_2, V2),
        OpenApiVersion::V3 => parsed(&OAS_3_0, V3_0),
        OpenApiVersion::V3_1 => parsed(&OAS_3_1, V3_1),
        OpenApiVersion::Unknown => None,
    }
}

fn parsed(cell: &'static OnceLock<Option<Value>>, raw: &str) -> Option<&'static Value> {
    cell.get_or_init(|| serde_json::from_str(raw).ok()).as_ref()
}

/// The JSON Schema dialect OAS 3.1 delegates Schema Objects to.
pub fn dialect_2020_12() -> Option<&'static Value> {
    registered(DIALECT_2020_12_ID)
}

/// Look up a bundled schema by reference. Both the absolute `$id` and the
/// form relative to the dialect base (`meta/core`, as the 2020-12 schema
/// writes it) resolve, which spares us a URL resolver.
pub fn registered(id: &str) -> Option<&'static Value> {
    static REGISTRY: OnceLock<HashMap<String, Value>> = OnceLock::new();
    let registry = REGISTRY.get_or_init(|| {
        let base = DIALECT_2020_12_ID.trim_end_matches("schema");
        let mut map = HashMap::new();
        for raw in DIALECT {
            let Ok(schema) = serde_json::from_str::<Value>(raw) else {
                continue;
            };
            let Some(id) = schema
                .get("$id")
                .and_then(Value::as_str)
                .map(|id| id.trim_end_matches('#').to_string())
            else {
                continue;
            };
            if let Some(relative) = id.strip_prefix(base) {
                map.insert(relative.to_string(), schema.clone());
            }
            map.insert(id, schema);
        }
        map
    });
    registry.get(id.trim_end_matches('#'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_schema_parses() {
        for version in [
            OpenApiVersion::V2,
            OpenApiVersion::V3,
            OpenApiVersion::V3_1,
        ] {
            let schema = schema_for(version).expect("bundled schema");
            assert!(schema.get("properties").is_some(), "{version:?}");
        }
        assert!(schema_for(OpenApiVersion::Unknown).is_none());
    }

    #[test]
    fn dialect_and_vocabularies_are_registered() {
        assert!(dialect_2020_12().is_some());
        for vocabulary in [
            "https://json-schema.org/draft/2020-12/meta/core",
            "https://json-schema.org/draft/2020-12/meta/applicator",
            "https://json-schema.org/draft/2020-12/meta/validation",
            "https://json-schema.org/draft/2020-12/meta/meta-data",
            "https://json-schema.org/draft/2020-12/meta/format-annotation",
            "https://json-schema.org/draft/2020-12/meta/content",
            "https://json-schema.org/draft/2020-12/meta/unevaluated",
        ] {
            assert!(registered(vocabulary).is_some(), "{vocabulary}");
        }
        // The dialect refers to its vocabularies with relative URIs.
        assert!(registered("meta/applicator").is_some());
    }
}
