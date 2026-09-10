//! Where a `$ref` written at a given place is expected to point.
//!
//! Keyed off the innermost node names: walk outwards from the cursor and stop
//! at the first hit. When nothing matches we return `None` and the
//! caller offers no completion, rather than guessing a container.

use crate::detect::OpenApiVersion;
use crate::pointer::last_segments;

/// How many node names above the cursor we are willing to consider.
const LOOKBACK: usize = 3;

pub fn ref_target_pointer(version: OpenApiVersion, node_path: &str) -> Option<String> {
    let mapping = match version {
        OpenApiVersion::V2 => &[
            ("schema", "/definitions"),
            ("items", "/definitions"),
            ("properties", "/definitions"),
            ("parameters", "/parameters"),
            ("responses", "/responses"),
        ][..],
        OpenApiVersion::V3 | OpenApiVersion::V3_1 => &[
            ("schema", "/components/schemas"),
            ("items", "/components/schemas"),
            ("properties", "/components/schemas"),
            ("additionalProperties", "/components/schemas"),
            ("allOf", "/components/schemas"),
            ("oneOf", "/components/schemas"),
            ("anyOf", "/components/schemas"),
            ("not", "/components/schemas"),
            ("responses", "/components/responses"),
            ("parameters", "/components/parameters"),
            ("examples", "/components/examples"),
            ("requestBody", "/components/requestBodies"),
            ("callbacks", "/components/callbacks"),
            ("headers", "/components/headers"),
            ("links", "/components/links"),
            ("securitySchemes", "/components/securitySchemes"),
            ("pathItems", "/components/pathItems"),
        ][..],
        OpenApiVersion::Unknown => return None,
    };

    // Innermost name wins; array indices and the `$ref` key itself are skipped.
    for segment in last_segments(node_path, LOOKBACK).iter().rev() {
        if segment == "$ref" || segment.parse::<usize>().is_ok() {
            continue;
        }
        if let Some((_, pointer)) = mapping.iter().find(|(key, _)| key == segment) {
            return Some((*pointer).to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_containers() {
        assert_eq!(
            ref_target_pointer(OpenApiVersion::V3, "/paths/~1pets/get/responses/200/content/application~1json/schema").as_deref(),
            Some("/components/schemas")
        );
        assert_eq!(
            ref_target_pointer(OpenApiVersion::V3, "/components/schemas/Pet/properties/tag").as_deref(),
            Some("/components/schemas")
        );
        assert_eq!(
            ref_target_pointer(OpenApiVersion::V2, "/paths/~1pets/get/schema").as_deref(),
            Some("/definitions")
        );
    }

    #[test]
    fn unknown_position_offers_nothing() {
        assert_eq!(ref_target_pointer(OpenApiVersion::V3, "/info/contact"), None);
        assert_eq!(ref_target_pointer(OpenApiVersion::V3, ""), None);
        assert_eq!(
            ref_target_pointer(OpenApiVersion::Unknown, "/components/schemas/Pet/properties"),
            None
        );
    }
}
