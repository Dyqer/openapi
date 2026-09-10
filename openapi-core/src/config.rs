//! User settings, as sent by the editor.
//!
//! Everything is optional and every unknown key is ignored, so an editor may
//! send whatever it likes: missing values fall back to the defaults below.

use serde::Deserialize;
use serde_json::Value as JsonValue;

/// Settings the server understands, under the `openapi` section.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Which files `$ref` completion offers.
    pub ref_files: RefFileSettings,
}

impl Settings {
    /// Read settings out of `initializationOptions` or a
    /// `workspace/didChangeConfiguration` payload, with or without the
    /// enclosing `openapi` section.
    pub fn from_json(value: &JsonValue) -> Self {
        let section = value.get("openapi").unwrap_or(value);
        serde_json::from_value(section.clone()).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RefFileSettings {
    /// Directory levels above the document to include in the scan.
    pub scan_up: usize,
    /// Directory levels below the document's own directory to descend into.
    pub scan_down: usize,
    /// Upper bound on offered files, so a huge tree stays cheap to scan.
    pub max_files: usize,
    /// Extensions to offer. Empty means "the extension of the document being
    /// edited", which keeps a YAML spec from suggesting its own JSON build
    /// output and vice versa.
    pub extensions: Vec<String>,
    /// Directory names never scanned, whatever their depth.
    pub skip_dirs: Vec<String>,
}

impl Default for RefFileSettings {
    fn default() -> Self {
        Self {
            scan_up: 6,
            scan_down: 6,
            max_files: 500,
            extensions: Vec::new(),
            skip_dirs: ["node_modules", "target", "dist", "build", "vendor"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn partial_settings_keep_the_defaults() {
        let settings = Settings::from_json(&json!({
            "openapi": { "refFiles": { "scanUp": 1, "extensions": ["yaml"] } }
        }));
        assert_eq!(settings.ref_files.scan_up, 1);
        assert_eq!(settings.ref_files.scan_down, 6);
        assert_eq!(settings.ref_files.extensions, vec!["yaml".to_string()]);
        assert!(!settings.ref_files.skip_dirs.is_empty());
    }

    #[test]
    fn unwrapped_and_unknown_payloads_are_tolerated() {
        let settings = Settings::from_json(&json!({ "refFiles": { "maxFiles": 3 } }));
        assert_eq!(settings.ref_files.max_files, 3);

        let settings = Settings::from_json(&json!({ "somethingElse": true }));
        assert_eq!(settings.ref_files.scan_up, 6);
    }
}
