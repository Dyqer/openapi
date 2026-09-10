//! A deliberately small JSON Schema subset, sized for the bundled OpenAPI
//! schemas (draft-04 / 2020-12).
//!
//! It answers two questions:
//!
//! * which keys / enum values are valid at a given JSON Pointer (completion), and
//! * which nodes violate the schema (diagnostics).
//!
//! Keywords we do not understand (`$dynamicRef`, `if`/`then`,
//! `unevaluatedProperties`, `dependentSchemas`, numeric and string facets) are
//! ignored rather than guessed at: completion then offers nothing and
//! validation stays quiet, which is the failure mode we want.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use regex::Regex;
use serde_json::Value;

use crate::pointer::{append_segment, parse_pointer};

/// A key that may be written at some position in the document.
#[derive(Clone, Debug)]
pub struct KeyInfo {
    pub name: String,
    pub required: bool,
    pub ty: Option<String>,
    pub description: Option<String>,
    pub deprecated: bool,
}

/// A schema violation, addressed by JSON Pointer into the document.
#[derive(Clone, Debug)]
pub struct Issue {
    pub pointer: String,
    pub message: String,
}

/// Max issues we collect for one document; keeps pathological files cheap.
const MAX_ISSUES: usize = 200;
const MAX_DEPTH: usize = 32;

pub struct Schema<'a> {
    root: &'a Value,
}

impl<'a> Schema<'a> {
    pub fn new(root: &'a Value) -> Self {
        Self { root }
    }

    /// Schemas that apply to the node at `pointer`, `$ref`s and combinators
    /// already expanded. Empty when the position is unknown to the schema.
    pub fn schemas_at(&self, instance: &Value, pointer: &str) -> Vec<&'a Value> {
        let mut current = self.applicable(self.root, Some(instance));
        let mut node = Some(instance);

        for seg in parse_pointer(pointer) {
            let child = node.and_then(|n| child_value(n, &seg));
            let mut next: Vec<&'a Value> = Vec::new();
            for schema in &current {
                for candidate in self.child_schemas(schema, &seg, node) {
                    for expanded in self.applicable(candidate, child) {
                        if !next.iter().any(|s| std::ptr::eq(*s, expanded)) {
                            next.push(expanded);
                        }
                    }
                }
            }
            if next.is_empty() {
                return Vec::new();
            }
            current = next;
            node = child;
        }
        current
    }

    /// Keys declared by `schemas`, required ones first.
    pub fn keys(&self, schemas: &[&'a Value]) -> Vec<KeyInfo> {
        let mut out: Vec<KeyInfo> = Vec::new();
        for schema in schemas {
            let required = string_array(schema.get("required"));
            let Some(props) = schema.get("properties").and_then(Value::as_object) else {
                continue;
            };
            for (name, sub) in props {
                if out.iter().any(|k| &k.name == name) {
                    continue;
                }
                out.push(KeyInfo {
                    name: name.clone(),
                    required: required.iter().any(|r| r == name),
                    ty: self
                        .field(sub, "type")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    description: self
                        .field(sub, "description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    deprecated: self
                        .field(sub, "deprecated")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                });
            }
        }
        out.sort_by(|a, b| b.required.cmp(&a.required).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// Values `schemas` allows, when they are a closed set.
    pub fn enum_values(&self, schemas: &[&'a Value]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for schema in schemas {
            let mut push = |v: &Value| {
                let text = match v {
                    Value::String(s) => s.clone(),
                    Value::Bool(b) => b.to_string(),
                    Value::Number(n) => n.to_string(),
                    _ => return,
                };
                if !out.contains(&text) {
                    out.push(text);
                }
            };
            if let Some(values) = schema.get("enum").and_then(Value::as_array) {
                values.iter().for_each(&mut push);
            }
            if let Some(value) = schema.get("const") {
                push(value);
            }
        }
        out
    }

    /// Validate a whole document.
    pub fn validate(&self, instance: &Value) -> Vec<Issue> {
        let mut out = Vec::new();
        self.check(self.root, instance, "", &mut out, 0);
        out
    }

    fn check(
        &self,
        schema: &'a Value,
        instance: &Value,
        pointer: &str,
        out: &mut Vec<Issue>,
        depth: usize,
    ) {
        // A null node is almost always a key whose value is still being typed.
        if depth > MAX_DEPTH || out.len() >= MAX_ISSUES || instance.is_null() {
            return;
        }
        if !schema.is_object() {
            return;
        }
        // In 2020-12 a `$ref` may carry sibling keywords, so the target is an
        // *additional* constraint rather than a replacement.
        for target in self.references(schema) {
            self.check(target, instance, pointer, out, depth + 1);
        }

        if let Some(ty) = schema.get("type")
            && !type_matches(ty, instance) {
                out.push(Issue {
                    pointer: pointer.to_string(),
                    message: format!("expected {}", type_name(ty)),
                });
                return;
            }
        if let Some(values) = schema.get("enum").and_then(Value::as_array)
            && !values.contains(instance) {
                out.push(Issue {
                    pointer: pointer.to_string(),
                    message: format!("value must be one of {}", join_values(values)),
                });
            }

        if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
            for sub in all_of {
                self.check(sub, instance, pointer, out, depth + 1);
            }
        }
        for keyword in ["oneOf", "anyOf"] {
            let Some(branches) = schema.get(keyword).and_then(Value::as_array) else {
                continue;
            };
            // Report the closest branch only, so a Reference Object being
            // written where a Schema Object also fits stays quiet.
            let mut best: Option<Vec<Issue>> = None;
            for branch in branches {
                let mut errs = Vec::new();
                self.check(branch, instance, pointer, &mut errs, depth + 1);
                if errs.is_empty() {
                    best = None;
                    break;
                }
                if best.as_ref().is_none_or(|b| errs.len() < b.len()) {
                    best = Some(errs);
                }
            }
            if let Some(errs) = best {
                out.extend(errs);
            }
        }

        if let Some(obj) = instance.as_object() {
            for required in string_array(schema.get("required")) {
                if !obj.contains_key(&required) {
                    out.push(Issue {
                        pointer: pointer.to_string(),
                        message: format!("missing required property `{required}`"),
                    });
                }
            }
            let props = schema.get("properties").and_then(Value::as_object);
            let patterns = schema.get("patternProperties").and_then(Value::as_object);
            for (key, value) in obj {
                let child = append_segment(pointer, key);
                let mut matched = false;
                if let Some(sub) = props.and_then(|p| p.get(key)) {
                    matched = true;
                    self.check(sub, value, &child, out, depth + 1);
                }
                for (pattern, sub) in patterns.into_iter().flatten() {
                    if pattern_matches(pattern, key) {
                        matched = true;
                        self.check(sub, value, &child, out, depth + 1);
                    }
                }
                if matched {
                    continue;
                }
                match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => out.push(Issue {
                        pointer: child,
                        message: format!("property `{key}` is not allowed here"),
                    }),
                    Some(sub) if sub.is_object() => self.check(sub, value, &child, out, depth + 1),
                    _ => {}
                }
            }
        }

        if let Some(items) = instance.as_array() {
            let schema_items = schema.get("items");
            for (i, value) in items.iter().enumerate() {
                let child = append_segment(pointer, &i.to_string());
                match schema_items {
                    // draft-04 tuple form.
                    Some(Value::Array(tuple)) => {
                        if let Some(sub) = tuple.get(i) {
                            self.check(sub, value, &child, out, depth + 1);
                        }
                    }
                    Some(sub) if sub.is_object() => self.check(sub, value, &child, out, depth + 1),
                    _ => {}
                }
            }
        }
    }

    /// Expand `$ref`, `allOf` and the branches of `oneOf`/`anyOf` that fit
    /// `instance` (all of them when nothing fits, e.g. mid-edit).
    fn applicable(&self, schema: &'a Value, instance: Option<&Value>) -> Vec<&'a Value> {
        let mut out = Vec::new();
        self.expand(schema, instance, &mut out, 0);
        out
    }

    fn expand(
        &self,
        schema: &'a Value,
        instance: Option<&Value>,
        out: &mut Vec<&'a Value>,
        depth: usize,
    ) {
        if depth > 12 {
            return;
        }
        if !schema.is_object() || out.iter().any(|s| std::ptr::eq(*s, schema)) {
            return;
        }
        out.push(schema);

        for target in self.references(schema) {
            self.expand(target, instance, out, depth + 1);
        }

        if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
            for sub in all_of {
                self.expand(sub, instance, out, depth + 1);
            }
        }
        for keyword in ["oneOf", "anyOf"] {
            let Some(branches) = schema.get(keyword).and_then(Value::as_array) else {
                continue;
            };
            let fitting: Vec<&'a Value> = match instance {
                Some(node) => branches
                    .iter()
                    .filter(|branch| {
                        let mut errs = Vec::new();
                        self.check(branch, node, "", &mut errs, 0);
                        errs.is_empty()
                    })
                    .collect(),
                None => Vec::new(),
            };
            let chosen = if fitting.is_empty() {
                branches.iter().collect()
            } else {
                fitting
            };
            for sub in chosen {
                self.expand(sub, instance, out, depth + 1);
            }
        }
    }

    /// Schemas for the child reached by `segment`.
    fn child_schemas(
        &self,
        schema: &'a Value,
        segment: &str,
        parent: Option<&Value>,
    ) -> Vec<&'a Value> {
        let mut out = Vec::new();
        if parent.is_some_and(Value::is_array) {
            if let (Some(prefix), Ok(index)) = (
                schema.get("prefixItems").and_then(Value::as_array),
                segment.parse::<usize>(),
            )
                && let Some(sub) = prefix.get(index) {
                    out.push(sub);
                }
            match schema.get("items") {
                Some(Value::Array(tuple)) => {
                    if let Ok(index) = segment.parse::<usize>() {
                        out.extend(tuple.get(index));
                    }
                }
                Some(sub) if sub.is_object() => out.push(sub),
                _ => {}
            }
            return out;
        }

        if let Some(sub) = schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|props| props.get(segment))
        {
            out.push(sub);
        }
        if let Some(patterns) = schema.get("patternProperties").and_then(Value::as_object) {
            for (pattern, sub) in patterns {
                if pattern_matches(pattern, segment) {
                    out.push(sub);
                }
            }
        }
        if out.is_empty()
            && let Some(sub) = schema.get("additionalProperties")
                && sub.is_object() {
                    out.push(sub);
                }
        out
    }

    /// Schemas `schema` pulls in by reference.
    fn references(&self, schema: &'a Value) -> Vec<&'a Value> {
        let mut out: Vec<&'a Value> = Vec::new();
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            out.extend(self.lookup(reference));
        }
        // OAS 3.1 delegates Schema Objects to the JSON Schema dialect this
        // way. Resolving `#meta` statically is enough for our purposes; other
        // dynamic anchors are left alone.
        if schema.get("$dynamicRef").and_then(Value::as_str) == Some("#meta") {
            out.extend(crate::schemas::dialect_2020_12());
        }
        out
    }

    /// Resolve `#/pointer`, or `<absolute $id>#/pointer` against the bundled
    /// schema registry.
    fn lookup(&self, reference: &str) -> Option<&'a Value> {
        let (uri, pointer) = reference.split_once('#').unwrap_or((reference, ""));
        let base: &'a Value = if uri.is_empty() {
            self.root
        } else {
            crate::schemas::registered(uri)?
        };
        if pointer.is_empty() {
            Some(base)
        } else {
            base.pointer(pointer)
        }
    }

    /// A keyword from `schema` itself, or from the first schema it references.
    fn field(&self, schema: &'a Value, name: &str) -> Option<&'a Value> {
        schema.get(name).or_else(|| {
            self.references(schema)
                .into_iter()
                .find_map(|target| target.get(name))
        })
    }
}

fn child_value<'v>(node: &'v Value, segment: &str) -> Option<&'v Value> {
    match node {
        Value::Object(map) => map.get(segment),
        Value::Array(items) => items.get(segment.parse::<usize>().ok()?),
        _ => None,
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn type_matches(ty: &Value, instance: &Value) -> bool {
    match ty {
        Value::String(name) => match name.as_str() {
            "object" => instance.is_object(),
            "array" => instance.is_array(),
            "string" => instance.is_string(),
            "boolean" => instance.is_boolean(),
            "number" => instance.is_number(),
            "integer" => instance.as_i64().is_some() || instance.as_u64().is_some(),
            "null" => instance.is_null(),
            _ => true,
        },
        Value::Array(names) => names.iter().any(|name| type_matches(name, instance)),
        _ => true,
    }
}

fn type_name(ty: &Value) -> String {
    match ty {
        Value::String(name) => name.clone(),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" or "),
        other => other.to_string(),
    }
}

fn join_values(values: &[Value]) -> String {
    values
        .iter()
        .map(|v| match v {
            Value::String(s) => format!("`{s}`"),
            other => format!("`{other}`"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `patternProperties` matching. Patterns come from the bundled schemas, so
/// they are trusted input; unparsable ones simply never match.
fn pattern_matches(pattern: &str, key: &str) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Regex>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut compiled = cache.lock().unwrap_or_else(PoisonError::into_inner);
    compiled
        .entry(pattern.to_string())
        .or_insert_with(|| Regex::new(pattern).ok())
        .as_ref()
        .is_some_and(|re| re.is_match(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Value {
        json!({
            "type": "object",
            "required": ["openapi"],
            "additionalProperties": false,
            "patternProperties": { "^x-": {} },
            "properties": {
                "openapi": { "type": "string" },
                "paths": {
                    "type": "object",
                    "patternProperties": {
                        "^/": { "$ref": "#/definitions/PathItem" }
                    },
                    "additionalProperties": false
                }
            },
            "definitions": {
                "PathItem": {
                    "type": "object",
                    "properties": {
                        "get": { "type": "object" },
                        "summary": { "type": "string" }
                    },
                    "additionalProperties": false
                }
            }
        })
    }

    #[test]
    fn keys_follow_pattern_properties_and_refs() {
        let root = schema();
        let s = Schema::new(&root);
        let doc = json!({ "openapi": "3.0.3", "paths": { "/pets": {} } });

        let names: Vec<String> = s
            .keys(&s.schemas_at(&doc, "/paths/~1pets"))
            .into_iter()
            .map(|k| k.name)
            .collect();
        assert_eq!(names, vec!["get", "summary"]);

        // Path names are user-chosen: nothing to suggest.
        assert!(s.keys(&s.schemas_at(&doc, "/paths")).is_empty());
    }

    #[test]
    fn reports_unknown_and_missing_keys() {
        let root = schema();
        let s = Schema::new(&root);
        let doc = json!({ "pathz": {}, "x-vendor": 1 });
        let messages: Vec<String> = s.validate(&doc).into_iter().map(|i| i.message).collect();
        assert!(
            messages.iter().any(|m| m.contains("`pathz` is not allowed")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|m| m.contains("missing required property `openapi`")),
            "{messages:?}"
        );
        // `x-` extensions are allowed by patternProperties.
        assert!(!messages.iter().any(|m| m.contains("x-vendor")), "{messages:?}");
    }

    #[test]
    fn half_typed_values_are_not_flagged() {
        let root = schema();
        let s = Schema::new(&root);
        let doc = json!({ "openapi": null, "paths": null });
        assert!(s.validate(&doc).is_empty());
    }
}
