#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenApiVersion {
    Unknown,
    V2,
    V3,
    V3_1,
}

impl OpenApiVersion {
    pub fn is_openapi(self) -> bool {
        self != Self::Unknown
    }
}

/// Detect OAS version from a parsed YAML/JSON document.
pub fn detect(value: &yaml_serde::Value) -> OpenApiVersion {
    if value["swagger"].as_str() == Some("2.0") {
        return OpenApiVersion::V2;
    }
    let Some(openapi) = value["openapi"].as_str() else {
        return OpenApiVersion::Unknown;
    };
    if matches_version(openapi, "3.0.") {
        OpenApiVersion::V3
    } else if matches_version(openapi, "3.1.") {
        OpenApiVersion::V3_1
    } else {
        OpenApiVersion::Unknown
    }
}

/// Best-effort version sniff for text that does not parse yet (a file opened
/// mid-edit, or a key being typed). Looks for a top-level `openapi:` /
/// `swagger:` entry in the first lines, in either YAML or JSON syntax.
pub fn detect_text(text: &str) -> OpenApiVersion {
    for line in text.lines().take(SNIFF_LINES) {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        for (key, version) in [("swagger", OpenApiVersion::V2), ("openapi", OpenApiVersion::V3)] {
            let Some(value) = entry_value(trimmed, key) else {
                continue;
            };
            return match version {
                OpenApiVersion::V2 => {
                    if value == "2.0" {
                        OpenApiVersion::V2
                    } else {
                        OpenApiVersion::Unknown
                    }
                }
                _ if matches_version(&value, "3.0.") => OpenApiVersion::V3,
                _ if matches_version(&value, "3.1.") => OpenApiVersion::V3_1,
                _ => OpenApiVersion::Unknown,
            };
        }
    }
    OpenApiVersion::Unknown
}

/// How far into a file we look for the version marker.
const SNIFF_LINES: usize = 50;

/// `key: value` / `"key": value,` with the value unquoted.
fn entry_value(line: &str, key: &str) -> Option<String> {
    let rest = line
        .strip_prefix(key)
        .or_else(|| line.strip_prefix(&format!("\"{key}\"")))
        .or_else(|| line.strip_prefix(&format!("'{key}'")))?;
    let value = rest.trim_start().strip_prefix(':')?.trim();
    let value = value.trim_end_matches(',').trim();
    let value = value.trim_matches('"').trim_matches('\'').trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn matches_version(openapi: &str, prefix: &str) -> bool {
    openapi == prefix.trim_end_matches('.') || openapi.starts_with(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> yaml_serde::Value {
        yaml_serde::from_str(s).unwrap()
    }

    #[test]
    fn detects_v2() {
        assert_eq!(detect(&parse("swagger: '2.0'\ninfo: {}\n")), OpenApiVersion::V2);
    }

    #[test]
    fn detects_v3() {
        assert_eq!(
            detect(&parse("openapi: 3.0.3\ninfo: {title: t, version: '1'}\n")),
            OpenApiVersion::V3
        );
    }

    #[test]
    fn detects_v3_1() {
        assert_eq!(
            detect(&parse("openapi: 3.1.0\ninfo: {title: t, version: '1'}\n")),
            OpenApiVersion::V3_1
        );
    }

    #[test]
    fn sniffs_version_from_unparseable_text() {
        assert_eq!(detect_text("openapi: 3.0.3\ninfo:\n  title\n"), OpenApiVersion::V3);
        assert_eq!(detect_text("openapi: \"3.1.0\"\npat\n"), OpenApiVersion::V3_1);
        assert_eq!(detect_text("swagger: '2.0'\nfoo\n"), OpenApiVersion::V2);
        assert_eq!(detect_text("{\n  \"openapi\": \"3.0.0\",\n  \"info\"\n"), OpenApiVersion::V3);
        assert_eq!(detect_text("services:\n  web: {\n"), OpenApiVersion::Unknown);
        // The marker has to be an entry, not a mention.
        assert_eq!(detect_text("description: openapi is nice\n"), OpenApiVersion::Unknown);
    }

    #[test]
    fn unknown_plain_yaml() {
        assert_eq!(detect(&parse("name: docker-compose\n")), OpenApiVersion::Unknown);
    }
}
