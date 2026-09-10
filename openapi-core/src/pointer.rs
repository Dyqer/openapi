//! JSON Pointer (RFC 6901) helpers.

use yaml_serde::{Mapping, Value};

pub fn encode_segment(seg: &str) -> String {
    seg.replace('~', "~0").replace('/', "~1")
}

pub fn decode_segment(seg: &str) -> String {
    seg.replace("~1", "/").replace("~0", "~")
}

pub fn parse_pointer(pointer: &str) -> Vec<String> {
    if pointer.is_empty() || pointer == "#" {
        return Vec::new();
    }
    let pointer = pointer.strip_prefix('#').unwrap_or(pointer);
    if pointer.is_empty() {
        return Vec::new();
    }
    pointer
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .map(decode_segment)
        .collect()
}

pub fn join_pointer(segments: &[String]) -> String {
    if segments.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for seg in segments {
        out.push('/');
        out.push_str(&encode_segment(seg));
    }
    out
}

pub fn append_segment(pointer: &str, seg: &str) -> String {
    format!("{}/{}", pointer.trim_end_matches('/'), encode_segment(seg))
}

pub fn last_segments(pointer: &str, n: usize) -> Vec<String> {
    let segs = parse_pointer(pointer);
    let start = segs.len().saturating_sub(n);
    segs[start..].to_vec()
}

pub fn parent_pointer(pointer: &str) -> String {
    let mut segs = parse_pointer(pointer);
    segs.pop();
    join_pointer(&segs)
}

pub fn get_at_pointer<'a>(root: &'a Value, pointer: &str) -> Option<&'a Value> {
    let mut current = root;
    for seg in parse_pointer(pointer) {
        current = match current {
            Value::Mapping(map) => get_map(map, &seg)?,
            Value::Sequence(seq) => {
                let idx: usize = seg.parse().ok()?;
                seq.get(idx)?
            }
            _ => return None,
        };
    }
    Some(current)
}

fn get_map<'a>(map: &'a Mapping, key: &str) -> Option<&'a Value> {
    map.get(Value::String(key.into()))
        .or_else(|| map.iter().find_map(|(k, v)| (k.as_str() == Some(key)).then_some(v)))
}

pub fn mapping_keys(value: &Value) -> Vec<String> {
    match value {
        Value::Mapping(map) => map
            .keys()
            .filter_map(|k| k.as_str().map(ToOwned::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_path_keys() {
        assert_eq!(encode_segment("/pets"), "~1pets");
        assert_eq!(decode_segment("~1pets"), "/pets");
        assert_eq!(parse_pointer("#/components/schemas/Pet"), vec!["components", "schemas", "Pet"]);
    }

    #[test]
    fn lookup() {
        let v: Value = yaml_serde::from_str("components:\n  schemas:\n    Pet:\n      type: object\n").unwrap();
        let pet = get_at_pointer(&v, "/components/schemas/Pet").unwrap();
        assert_eq!(pet["type"].as_str(), Some("object"));
    }
}
