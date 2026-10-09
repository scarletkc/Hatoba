//! Input schema cleaning (AI-30): MCP servers send any JSON Schema (often draft-07 from
//! zod-to-json-schema, sometimes draft-04 or 2020-12), and each protocol's providers reject
//! different parts of it.
//!
//! What the providers reject, and what is done about it:
//!
//! - Both: the top level must be `"type": "object"`; Anthropic refuses `oneOf`, `anyOf` and
//!   `allOf` there ("input_schema does not support oneOf, allOf, or anyOf at the top level") and
//!   OpenAI refuses `enum`, `not` and unions there too. A top-level `$ref` (zod-to-json-schema's
//!   named schemas) is resolved, `allOf` parts are merged, and union variants become one object
//!   with every variant's properties (a property the variants define differently becomes
//!   `anyOf`, or one `enum` when they are enums or constants of one type), required only when
//!   every variant requires it. `$schema` and `$id` carry nothing for the model and are dropped.
//! - Anthropic validates against JSON Schema 2020-12 and accepts the rest (`$defs`/`$ref`,
//!   `format`, `default`, `additionalProperties`, nested unions), which is kept. Draft-04 forms
//!   that 2020-12 rejects are rewritten: boolean `exclusiveMinimum`/`exclusiveMaximum` become
//!   the numeric form, and tuple `items` arrays become `prefixItems`. Remote and dangling
//!   `$ref`s, which the provider cannot resolve, are dropped.
//! - Chat Completions also reaches Gemini's OpenAI-compatible endpoint and local servers, so its
//!   schemas keep only the keywords they all accept: `type`, `description`, `title`, `enum`,
//!   `default`, `properties`, `required`, `items`, `anyOf`, the numeric, length, pattern and
//!   count limits, `nullable` and `example`. Gemini rejects `$ref`/`$defs`, `oneOf`, `allOf`,
//!   `const`, `additionalProperties`, `exclusiveMinimum`, type arrays and most `format` values,
//!   and OpenAI rejects an `array` without `items`. So local `$ref`s are inlined (recursion and
//!   size are capped), `oneOf` becomes `anyOf`, `allOf` is merged, `const` becomes a one-value
//!   `enum`, exclusive bounds become inclusive ones, `["T", "null"]` becomes `T`, `format` is
//!   kept only where Gemini accepts it (`date-time` and `enum` for strings, `float`/`double`,
//!   `int32`/`int64`) and otherwise noted in the description, `items` defaults to `{}`, `true`
//!   schemas become `{}`, properties whose schema is `false` are dropped, and `required` keeps
//!   only declared properties.
//!
//! The server still validates what it receives, so a looser schema only loses guidance.

use serde_json::{Map, Value, json};

use crate::provider::Protocol;

/// Nesting deeper than this is replaced with `{}`.
const MAX_DEPTH: usize = 32;
/// `$ref` expansions per schema (Chat Completions), so a schema cannot grow without bound.
const MAX_REF_EXPANSIONS: usize = 500;
/// `$ref` steps followed at the top level.
const MAX_TOP_REF_STEPS: usize = 8;

/// Keywords whose value is a map from names to schemas.
const SCHEMA_MAPS: [&str; 5] = [
    "properties",
    "patternProperties",
    "$defs",
    "definitions",
    "dependentSchemas",
];
/// Keywords whose value is one schema.
const SCHEMA_VALUES: [&str; 11] = [
    "additionalProperties",
    "additionalItems",
    "not",
    "if",
    "then",
    "else",
    "contains",
    "propertyNames",
    "unevaluatedProperties",
    "unevaluatedItems",
    "contentSchema",
];
/// Keywords whose value is a list of schemas.
const SCHEMA_LISTS: [&str; 4] = ["anyOf", "oneOf", "allOf", "prefixItems"];
/// Keywords Chat Completions keeps as they are.
const CHAT_PLAIN: [&str; 15] = [
    "default",
    "description",
    "enum",
    "example",
    "maxItems",
    "maxLength",
    "maxProperties",
    "maximum",
    "minItems",
    "minLength",
    "minProperties",
    "minimum",
    "nullable",
    "pattern",
    "title",
];

/// AI-30: drops the input schema keywords `protocol` does not accept and keeps a valid object
/// schema, with `"type": "object"` and `properties` at the top. Everything both protocols accept
/// is kept; see the module documentation for the rules. Deterministic for the same input
/// (object keys come out sorted).
#[must_use]
pub fn clean_schema(schema: &Value, protocol: Protocol) -> Value {
    let mut cleaner = Cleaner {
        root: schema,
        protocol,
        stack: Vec::new(),
        budget: MAX_REF_EXPANSIONS,
    };
    let root = match schema {
        Value::Object(map) => cleaner.object(map, 0),
        _ => Map::new(),
    };
    Value::Object(fix_top(root, protocol))
}

struct Cleaner<'a> {
    root: &'a Value,
    protocol: Protocol,
    /// `$ref`s being expanded (Chat Completions), to stop recursion.
    stack: Vec<String>,
    budget: usize,
}

impl Cleaner<'_> {
    fn object(&mut self, map: &Map<String, Value>, depth: usize) -> Map<String, Value> {
        if depth > MAX_DEPTH {
            return Map::new();
        }
        match self.protocol {
            Protocol::Anthropic => self.anthropic(map, depth),
            Protocol::ChatCompletions => self.chat(map, depth),
        }
    }

    /// A schema in a schema position. `None`: drop it (a `false` schema for Chat Completions).
    fn node(&mut self, value: &Value, depth: usize) -> Option<Value> {
        match value {
            Value::Object(map) => Some(Value::Object(self.object(map, depth + 1))),
            Value::Bool(accept) => match self.protocol {
                Protocol::Anthropic => Some(Value::Bool(*accept)),
                Protocol::ChatCompletions => accept.then(|| json!({})),
            },
            _ => Some(json!({})),
        }
    }

    fn node_or_empty(&mut self, value: &Value, depth: usize) -> Value {
        self.node(value, depth).unwrap_or_else(|| json!({}))
    }

    fn schema_list(&mut self, value: &Value, depth: usize) -> Vec<Value> {
        value
            .as_array()
            .map(|items| items.iter().filter_map(|i| self.node(i, depth)).collect())
            .unwrap_or_default()
    }

    fn schema_map(&mut self, value: &Value, depth: usize) -> Map<String, Value> {
        let mut out = Map::new();
        if let Value::Object(map) = value {
            for (name, schema) in map {
                if let Some(schema) = self.node(schema, depth) {
                    out.insert(name.clone(), schema);
                }
            }
        }
        out
    }

    /// Anthropic: 2020-12 as sent, minus `$schema`/`$id` and `$ref`s that cannot resolve, and
    /// with draft-04 forms rewritten.
    fn anthropic(&mut self, map: &Map<String, Value>, depth: usize) -> Map<String, Value> {
        let mut out = Map::new();
        let tuple = matches!(map.get("items"), Some(Value::Array(_)));
        for (key, value) in map {
            let cleaned = match key.as_str() {
                "$schema" | "$id" => continue,
                // Remote and dangling references cannot be resolved by the provider.
                "$ref" => match value.as_str() {
                    Some(reference) if local_target(self.root, reference).is_some() => {
                        value.clone()
                    }
                    _ => continue,
                },
                "items" if tuple => {
                    let items = Value::Array(self.schema_list(value, depth));
                    out.insert("prefixItems".to_owned(), items);
                    continue;
                }
                "prefixItems" if tuple => continue,
                "required" => match string_array(value) {
                    Some(required) => required,
                    None => continue,
                },
                k if SCHEMA_MAPS.contains(&k) => Value::Object(self.schema_map(value, depth)),
                k if SCHEMA_VALUES.contains(&k) || k == "items" => self.node_or_empty(value, depth),
                k if SCHEMA_LISTS.contains(&k) => Value::Array(self.schema_list(value, depth)),
                _ => value.clone(),
            };
            out.insert(key.clone(), cleaned);
        }
        if tuple && let Some(rest) = out.remove("additionalItems") {
            out.insert("items".to_owned(), rest);
        }
        for (exclusive, bound) in [
            ("exclusiveMinimum", "minimum"),
            ("exclusiveMaximum", "maximum"),
        ] {
            match out.get(exclusive) {
                Some(Value::Number(_)) | None => {}
                Some(Value::Bool(true)) => match out.remove(bound) {
                    Some(limit @ Value::Number(_)) => {
                        out.insert(exclusive.to_owned(), limit);
                    }
                    _ => {
                        out.remove(exclusive);
                    }
                },
                Some(_) => {
                    out.remove(exclusive);
                }
            }
        }
        out
    }

    /// Chat Completions: the keywords every provider behind it accepts.
    fn chat(&mut self, map: &Map<String, Value>, depth: usize) -> Map<String, Value> {
        if let Some(Value::String(reference)) = map.get("$ref") {
            // The target, with the keywords next to the `$ref` (such as a description) on top.
            let mut out = self.reference(reference, depth);
            let siblings: Map<String, Value> = map
                .iter()
                .filter(|(key, _)| key.as_str() != "$ref")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            if !siblings.is_empty() {
                for (key, value) in self.chat(&siblings, depth) {
                    out.insert(key, value);
                }
            }
            return out;
        }

        let mut out = Map::new();
        let mut variants = Vec::new();
        let mut all_of = Vec::new();
        for (key, value) in map {
            match key.as_str() {
                "properties" => {
                    if value.is_object() {
                        out.insert(key.clone(), Value::Object(self.schema_map(value, depth)));
                    }
                }
                "items" => {
                    let item = match value {
                        Value::Array(items) => items.first().and_then(|i| self.node(i, depth)),
                        other => self.node(other, depth),
                    };
                    out.insert(key.clone(), item.unwrap_or_else(|| json!({})));
                }
                "prefixItems" if !map.contains_key("items") => {
                    if let Some(first) = value.as_array().and_then(|items| items.first()) {
                        let item = self.node_or_empty(first, depth);
                        out.insert("items".to_owned(), item);
                    }
                }
                "anyOf" | "oneOf" => variants.extend(self.schema_list(value, depth)),
                "allOf" => all_of.extend(self.schema_list(value, depth)),
                "type" => {
                    if let Some(kind) = chat_type(value) {
                        out.insert(key.clone(), kind);
                    }
                }
                "required" => {
                    if let Some(required) = string_array(value) {
                        out.insert(key.clone(), required);
                    }
                }
                plain if CHAT_PLAIN.contains(&plain) => {
                    out.insert(key.clone(), value.clone());
                }
                // `const`, the exclusive bounds and `format` are handled below; the rest is
                // dropped.
                _ => {}
            }
        }
        if let Some(constant) = map.get("const")
            && !out.contains_key("enum")
        {
            out.insert("enum".to_owned(), json!([constant]));
        }
        for (exclusive, bound) in [
            ("exclusiveMinimum", "minimum"),
            ("exclusiveMaximum", "maximum"),
        ] {
            // The 2020-12 numeric form becomes an inclusive bound; the draft-04 boolean form
            // leaves `minimum`/`maximum` inclusive.
            if let Some(limit @ Value::Number(_)) = map.get(exclusive) {
                out.entry(bound.to_owned()).or_insert_with(|| limit.clone());
            }
        }
        for part in all_of {
            if let Value::Object(part) = part {
                merge_all_of(&mut out, part);
            }
        }
        if variants.len() == 1 {
            // `anyOf` with one variant is that variant.
            if let Some(Value::Object(only)) = variants.pop() {
                merge_all_of(&mut out, only);
            }
        } else if !variants.is_empty() {
            out.insert("anyOf".to_owned(), Value::Array(variants));
        }
        if let Some(format) = map.get("format").and_then(Value::as_str) {
            if chat_format_allowed(format, out.get("type")) {
                out.insert("format".to_owned(), json!(format));
            } else {
                note_format(&mut out, format);
            }
        }
        keep_declared_required(&mut out);
        if out.get("type").and_then(Value::as_str) == Some("array") && !out.contains_key("items") {
            out.insert("items".to_owned(), json!({}));
        }
        out
    }

    /// The cleaned target of a local `$ref` (Chat Completions). A remote, missing, recursive or
    /// over-budget reference becomes `{}` with the target's type and description when known.
    fn reference(&mut self, reference: &str, depth: usize) -> Map<String, Value> {
        let Some(Value::Object(target)) = local_target(self.root, reference) else {
            return Map::new();
        };
        if self.budget == 0 || depth > MAX_DEPTH || self.stack.iter().any(|r| r == reference) {
            let mut stub = Map::new();
            for key in ["type", "description"] {
                if let Some(value @ Value::String(_)) = target.get(key) {
                    stub.insert(key.to_owned(), value.clone());
                }
            }
            return stub;
        }
        self.budget -= 1;
        self.stack.push(reference.to_owned());
        let out = self.chat(target, depth + 1);
        self.stack.pop();
        out
    }
}

/// The target of a local `$ref` (`#` plus a JSON pointer, percent-encoded as URI fragments are).
fn local_target<'v>(root: &'v Value, reference: &str) -> Option<&'v Value> {
    let fragment = reference.strip_prefix('#')?;
    root.pointer(&decode_pointer(fragment))
}

/// Percent-decodes a URI fragment (`%20` and so on); the fragment as it is when that fails.
fn decode_pointer(fragment: &str) -> String {
    if !fragment.contains('%') {
        return fragment.to_owned();
    }
    let bytes = fragment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = bytes.get(i + 1..i + 3)
            && let Some(byte) = std::str::from_utf8(hex)
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| fragment.to_owned())
}

/// A list of strings, without the other elements; `None` when it is not a list.
fn string_array(value: &Value) -> Option<Value> {
    let items = value.as_array()?;
    let mut names: Vec<Value> = Vec::new();
    for name in items.iter().filter(|v| v.is_string()) {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    Some(Value::Array(names))
}

/// `"T"` stays; `["T", "null"]` becomes `"T"`; other type lists are dropped (any type).
fn chat_type(value: &Value) -> Option<Value> {
    match value {
        Value::String(_) => Some(value.clone()),
        Value::Array(types) => {
            let named: Vec<&str> = types
                .iter()
                .filter_map(Value::as_str)
                .filter(|t| *t != "null")
                .collect();
            match named.as_slice() {
                [only] => Some(json!(only)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The `format` values Gemini accepts for each type.
fn chat_format_allowed(format: &str, kind: Option<&Value>) -> bool {
    match kind.and_then(Value::as_str) {
        Some("string") => matches!(format, "date-time" | "enum"),
        Some("number") => matches!(format, "float" | "double"),
        Some("integer") => matches!(format, "int32" | "int64"),
        _ => false,
    }
}

/// Keeps a dropped `format` as guidance in the description.
fn note_format(out: &mut Map<String, Value>, format: &str) {
    let format: String = format
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(40)
        .collect();
    if format.is_empty() {
        return;
    }
    let description = match out.get("description").and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => format!("{} (format: {format})", text.trim_end()),
        _ => format!("Format: {format}."),
    };
    out.insert("description".to_owned(), Value::String(description));
}

/// `required` keeps only names `properties` declares (Gemini rejects others), and is removed
/// when empty.
fn keep_declared_required(out: &mut Map<String, Value>) {
    let declared: Option<Vec<String>> = out
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| props.keys().cloned().collect());
    let Some(declared) = declared else {
        return;
    };
    if let Some(Value::Array(required)) = out.get_mut("required") {
        required.retain(|name| {
            name.as_str()
                .is_some_and(|n| declared.iter().any(|d| d == n))
        });
        if required.is_empty() {
            out.remove("required");
        }
    }
}

/// Merges an `allOf` part: properties and `required` are joined, other keywords added when
/// missing.
fn merge_all_of(out: &mut Map<String, Value>, part: Map<String, Value>) {
    for (key, value) in part {
        match (key.as_str(), value) {
            ("properties", Value::Object(props)) => {
                if let Value::Object(target) = out.entry(key).or_insert_with(|| json!({})) {
                    for (name, schema) in props {
                        target.entry(name).or_insert(schema);
                    }
                }
            }
            ("required", Value::Array(names)) => {
                if let Value::Array(target) = out.entry(key).or_insert_with(|| json!([])) {
                    for name in names {
                        if !target.contains(&name) {
                            target.push(name);
                        }
                    }
                }
            }
            (_, value) => {
                out.entry(key).or_insert(value);
            }
        }
    }
}

/// Makes the top level an object schema that both protocols accept.
fn fix_top(mut root: Map<String, Value>, protocol: Protocol) -> Map<String, Value> {
    // A `$ref` at the top (Anthropic keeps `$ref`s; Chat Completions has inlined them).
    for _ in 0..MAX_TOP_REF_STEPS {
        let Some(Value::String(reference)) = root.remove("$ref") else {
            break;
        };
        if let Some(Value::Object(target)) = local_ref(&root, &reference) {
            for (key, value) in target {
                root.entry(key).or_insert(value);
            }
        }
    }
    if let Some(Value::Array(parts)) = root.remove("allOf") {
        for part in parts {
            if let Value::Object(part) = resolve_top(&root, part) {
                merge_all_of(&mut root, part);
            }
        }
    }
    let mut variants = Vec::new();
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(list)) = root.remove(key) {
            variants.extend(list);
        }
    }
    let variants: Vec<Map<String, Value>> = variants
        .into_iter()
        .filter_map(|variant| match resolve_top(&root, variant) {
            Value::Object(map) if is_object_schema(&map) => Some(map),
            _ => None,
        })
        .collect();
    if !variants.is_empty() {
        merge_variants(&mut root, &variants);
    }
    for key in ["enum", "const", "not", "if", "then", "else"] {
        root.remove(key);
    }
    root.insert("type".to_owned(), json!("object"));
    if !root.get("properties").is_some_and(Value::is_object) {
        root.insert("properties".to_owned(), json!({}));
    }
    if protocol == Protocol::ChatCompletions {
        keep_declared_required(&mut root);
    }
    root
}

/// The target of a local `$ref` within the top-level schema.
fn local_ref(root: &Map<String, Value>, reference: &str) -> Option<Value> {
    if reference == "#" {
        return None;
    }
    let document = Value::Object(root.clone());
    local_target(&document, reference).cloned()
}

/// A top-level variant or `allOf` part with its own `$ref` followed (Anthropic).
fn resolve_top(root: &Map<String, Value>, value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    for _ in 0..MAX_TOP_REF_STEPS {
        let Some(Value::String(reference)) = map.remove("$ref") else {
            break;
        };
        if let Some(Value::Object(target)) = local_ref(root, &reference) {
            for (key, value) in target {
                map.entry(key).or_insert(value);
            }
        }
    }
    Value::Object(map)
}

/// Whether a union variant describes an object (other variants, such as `null`, are ignored).
fn is_object_schema(map: &Map<String, Value>) -> bool {
    match map.get("type") {
        Some(Value::String(kind)) => kind == "object",
        Some(Value::Array(kinds)) => kinds.iter().any(|k| k == "object"),
        _ => map.contains_key("properties") || map.contains_key("required"),
    }
}

/// Folds union variants into the top level: every variant's properties, and `required` for the
/// names every variant requires.
fn merge_variants(root: &mut Map<String, Value>, variants: &[Map<String, Value>]) {
    let mut collected: Map<String, Value> = Map::new();
    for variant in variants {
        if let Some(Value::Object(props)) = variant.get("properties") {
            for (name, schema) in props {
                let entry = collected
                    .entry(name.clone())
                    .or_insert_with(|| Value::Array(Vec::new()));
                if let Value::Array(list) = entry
                    && !list.contains(schema)
                {
                    list.push(schema.clone());
                }
            }
        }
    }
    if let Value::Object(target) = root
        .entry("properties".to_owned())
        .or_insert_with(|| json!({}))
    {
        for (name, schemas) in collected {
            if let Value::Array(schemas) = schemas {
                target.entry(name).or_insert_with(|| combine(schemas));
            }
        }
    }

    let mut common: Option<Vec<Value>> = None;
    for variant in variants {
        let names: Vec<Value> = variant
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        common = Some(match common {
            None => names,
            Some(previous) => previous.into_iter().filter(|n| names.contains(n)).collect(),
        });
    }
    let common = common.unwrap_or_default();
    if !common.is_empty()
        && let Value::Array(required) = root
            .entry("required".to_owned())
            .or_insert_with(|| json!([]))
    {
        for name in common {
            if !required.contains(&name) {
                required.push(name);
            }
        }
    }
}

/// One schema for a property that union variants define differently: the schema itself when they
/// agree, one `enum` when they are all enums or constants of one type (a discriminator), else
/// `anyOf`.
fn combine(mut schemas: Vec<Value>) -> Value {
    if schemas.len() == 1 {
        return schemas.remove(0);
    }
    let values_of = |schema: &Value| -> Option<Vec<Value>> {
        let map = schema.as_object()?;
        match (map.get("enum"), map.get("const")) {
            (Some(Value::Array(values)), _) => Some(values.clone()),
            (None, Some(value)) => Some(vec![value.clone()]),
            _ => None,
        }
    };
    let same_type = schemas
        .iter()
        .all(|s| s.get("type") == schemas[0].get("type"));
    let all_values: Option<Vec<Vec<Value>>> = schemas.iter().map(values_of).collect();
    if same_type && let Some(all_values) = all_values {
        let mut merged = schemas[0].as_object().cloned().unwrap_or_default();
        merged.remove("const");
        let mut values: Vec<Value> = Vec::new();
        for value in all_values.into_iter().flatten() {
            if !values.contains(&value) {
                values.push(value);
            }
        }
        merged.insert("enum".to_owned(), Value::Array(values));
        return Value::Object(merged);
    }
    json!({ "anyOf": schemas })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: [Protocol; 2] = [Protocol::Anthropic, Protocol::ChatCompletions];

    #[test]
    fn non_objects_become_empty_object_schemas() {
        for protocol in BOTH {
            for input in [
                json!(null),
                json!("x"),
                json!([1]),
                json!({}),
                json!({"type": "string"}),
            ] {
                assert_eq!(
                    clean_schema(&input, protocol),
                    json!({"type": "object", "properties": {}}),
                    "{input} for {protocol:?}"
                );
            }
        }
    }

    /// What zod-to-json-schema emits for a typical MCP tool.
    fn zod_schema() -> Value {
        json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "url": {"type": "string", "format": "uri", "description": "Page to open"},
                "when": {"type": "string", "format": "date-time"},
                "email": {"type": "string", "format": "email"},
                "tags": {"type": "array"},
                "mode": {"const": "fast"},
                "limit": {"type": "integer", "exclusiveMinimum": 0, "default": 10},
                "note": {"type": ["string", "null"], "examples": ["hi"]},
                "extra": {"type": "object", "additionalProperties": {"type": "string"}}
            },
            "required": ["url", "ghost"],
            "additionalProperties": false
        })
    }

    #[test]
    fn anthropic_keeps_what_it_accepts() {
        let cleaned = clean_schema(&zod_schema(), Protocol::Anthropic);
        let mut expected = zod_schema();
        expected.as_object_mut().unwrap().remove("$schema");
        assert_eq!(cleaned, expected);
    }

    #[test]
    fn chat_completions_keeps_the_common_subset() {
        let cleaned = clean_schema(&zod_schema(), Protocol::ChatCompletions);
        assert_eq!(
            cleaned,
            json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "Page to open (format: uri)"},
                    "when": {"type": "string", "format": "date-time"},
                    "email": {"type": "string", "description": "Format: email."},
                    "tags": {"type": "array", "items": {}},
                    "mode": {"enum": ["fast"]},
                    "limit": {"type": "integer", "minimum": 0, "default": 10},
                    "note": {"type": "string"},
                    "extra": {"type": "object"}
                },
                "required": ["url"]
            })
        );
    }

    #[test]
    fn refs_are_inlined_for_chat_and_kept_for_anthropic() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "urn:x",
            "type": "object",
            "properties": {
                "point": {"$ref": "#/$defs/Point", "description": "Where"},
                "old": {"$ref": "#/definitions/Old"},
                "tree": {"$ref": "#/$defs/Node"},
                "remote": {"$ref": "https://example.com/schema.json"},
                "dangling": {"$ref": "#/$defs/Missing", "type": "string"},
                "spaced": {"$ref": "#/$defs/With%20Space"}
            },
            "$defs": {
                "With Space": {"type": "boolean"},
                "Point": {"type": "object", "properties": {"x": {"type": "number"}}, "required": ["x"]},
                "Node": {
                    "type": "object",
                    "description": "A node",
                    "properties": {"children": {"type": "array", "items": {"$ref": "#/$defs/Node"}}}
                }
            },
            "definitions": {"Old": {"type": "string"}}
        });

        let anthropic = clean_schema(&schema, Protocol::Anthropic);
        let mut expected = schema.clone();
        expected.as_object_mut().unwrap().remove("$schema");
        expected.as_object_mut().unwrap().remove("$id");
        expected["properties"]["remote"] = json!({});
        expected["properties"]["dangling"] = json!({"type": "string"});
        assert_eq!(anthropic, expected);

        let chat = clean_schema(&schema, Protocol::ChatCompletions);
        assert_eq!(
            chat,
            json!({
                "type": "object",
                "properties": {
                    "point": {
                        "type": "object",
                        "description": "Where",
                        "properties": {"x": {"type": "number"}},
                        "required": ["x"]
                    },
                    "old": {"type": "string"},
                    "tree": {
                        "type": "object",
                        "description": "A node",
                        "properties": {"children": {
                            "type": "array",
                            "items": {"type": "object", "description": "A node"}
                        }}
                    },
                    "remote": {},
                    "dangling": {"type": "string"},
                    "spaced": {"type": "boolean"}
                }
            })
        );
    }

    #[test]
    fn a_top_level_ref_is_resolved() {
        // zod-to-json-schema with a name.
        let schema = json!({
            "$ref": "#/definitions/Args",
            "definitions": {
                "Args": {"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]}
            },
            "$schema": "http://json-schema.org/draft-07/schema#"
        });
        let expected_props = json!({"q": {"type": "string"}});
        for protocol in BOTH {
            let cleaned = clean_schema(&schema, protocol);
            assert_eq!(cleaned["type"], "object", "{protocol:?}");
            assert_eq!(cleaned["properties"], expected_props, "{protocol:?}");
            assert_eq!(cleaned["required"], json!(["q"]), "{protocol:?}");
            assert!(cleaned.get("$ref").is_none());
        }
        assert!(
            clean_schema(&schema, Protocol::Anthropic)
                .get("definitions")
                .is_some()
        );
        assert!(
            clean_schema(&schema, Protocol::ChatCompletions)
                .get("definitions")
                .is_none()
        );
    }

    #[test]
    fn top_level_unions_become_one_object() {
        // A discriminated union, as zod emits it.
        let schema = json!({
            "anyOf": [
                {
                    "type": "object",
                    "properties": {"kind": {"type": "string", "const": "file"}, "path": {"type": "string"}},
                    "required": ["kind", "path"],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": {"kind": {"type": "string", "const": "url"}, "url": {"type": "string"}},
                    "required": ["kind", "url"],
                    "additionalProperties": false
                },
                {"type": "null"}
            ]
        });
        for protocol in BOTH {
            let cleaned = clean_schema(&schema, protocol);
            assert_eq!(
                cleaned,
                json!({
                    "type": "object",
                    "properties": {
                        "kind": {"type": "string", "enum": ["file", "url"]},
                        "path": {"type": "string"},
                        "url": {"type": "string"}
                    },
                    "required": ["kind"]
                }),
                "{protocol:?}"
            );
        }

        let mixed = json!({
            "oneOf": [
                {"properties": {"a": {"type": "string"}}},
                {"properties": {"a": {"type": "number"}}}
            ],
            "allOf": [{"properties": {"b": {"type": "boolean"}}, "required": ["b"]}],
            "enum": [1],
            "not": {"required": ["c"]}
        });
        for protocol in BOTH {
            assert_eq!(
                clean_schema(&mixed, protocol),
                json!({
                    "type": "object",
                    "properties": {
                        "a": {"anyOf": [{"type": "string"}, {"type": "number"}]},
                        "b": {"type": "boolean"}
                    },
                    "required": ["b"]
                }),
                "{protocol:?}"
            );
        }
    }

    #[test]
    fn nested_unions_and_draft_04_forms() {
        let schema = json!({
            "type": "object",
            "properties": {
                "value": {"oneOf": [{"type": "string"}, {"type": "integer"}]},
                "single": {"anyOf": [{"type": "string", "minLength": 1}]},
                "both": {"allOf": [
                    {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]},
                    {"properties": {"b": {"type": "string"}}, "required": ["b"]}
                ]},
                "pct": {"type": "number", "minimum": 0, "exclusiveMinimum": true, "maximum": 1, "exclusiveMaximum": false},
                "pair": {"type": "array", "items": [{"type": "string"}, {"type": "number"}], "additionalItems": false},
                "anything": true,
                "nothing": false
            },
            "required": ["nothing", "value"]
        });

        let anthropic = clean_schema(&schema, Protocol::Anthropic);
        let props = &anthropic["properties"];
        assert_eq!(
            props["value"],
            json!({"oneOf": [{"type": "string"}, {"type": "integer"}]})
        );
        assert_eq!(
            props["pct"],
            json!({"type": "number", "exclusiveMinimum": 0, "maximum": 1})
        );
        assert_eq!(
            props["pair"],
            json!({"type": "array", "prefixItems": [{"type": "string"}, {"type": "number"}], "items": false})
        );
        assert_eq!(props["anything"], json!(true));
        assert_eq!(props["nothing"], json!(false));
        assert_eq!(anthropic["required"], json!(["nothing", "value"]));

        let chat = clean_schema(&schema, Protocol::ChatCompletions);
        assert_eq!(
            chat,
            json!({
                "type": "object",
                "properties": {
                    "value": {"anyOf": [{"type": "string"}, {"type": "integer"}]},
                    "single": {"type": "string", "minLength": 1},
                    "both": {
                        "type": "object",
                        "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
                        "required": ["a", "b"]
                    },
                    "pct": {"type": "number", "minimum": 0, "maximum": 1},
                    "pair": {"type": "array", "items": {"type": "string"}},
                    "anything": {}
                },
                "required": ["value"]
            })
        );
    }

    #[test]
    fn property_names_that_look_like_keywords_survive() {
        let schema = json!({
            "type": "object",
            "properties": {
                "format": {"type": "string"},
                "$ref": {"type": "string"},
                "type": {"type": "string", "enum": ["a", "b"]},
                "additionalProperties": {"type": "boolean"}
            },
            "required": ["format", "$ref"]
        });
        for protocol in BOTH {
            let cleaned = clean_schema(&schema, protocol);
            assert_eq!(cleaned, schema, "{protocol:?}");
        }
    }

    #[test]
    fn recursion_and_depth_are_bounded() {
        // Every level references the root twice: without a cap this would never finish.
        let schema = json!({
            "type": "object",
            "properties": {"a": {"$ref": "#"}, "b": {"$ref": "#"}}
        });
        let cleaned = clean_schema(&schema, Protocol::ChatCompletions);
        assert_eq!(
            cleaned["properties"]["a"],
            json!({"type": "object", "properties": {"a": {"type": "object"}, "b": {"type": "object"}}})
        );

        let mut deep = json!({"type": "string"});
        for _ in 0..100 {
            deep = json!({"type": "object", "properties": {"x": deep}});
        }
        for protocol in BOTH {
            let cleaned = clean_schema(&deep, protocol);
            let text = cleaned.to_string();
            assert!(text.matches("properties").count() < 40, "{protocol:?}");
        }

        // A wide fan-out of references stays within the expansion budget.
        let mut defs = Map::new();
        for i in 0..20 {
            let next = format!("#/$defs/d{}", i + 1);
            defs.insert(
                format!("d{i}"),
                json!({"type": "object", "properties": {"l": {"$ref": next}, "r": {"$ref": next}}}),
            );
        }
        defs.insert("d20".into(), json!({"type": "string"}));
        let wide = json!({"type": "object", "properties": {"root": {"$ref": "#/$defs/d0"}}, "$defs": defs});
        let cleaned = clean_schema(&wide, Protocol::ChatCompletions).to_string();
        assert!(cleaned.len() < 200_000, "{}", cleaned.len());
    }

    #[test]
    fn cleaning_is_deterministic_and_leaves_the_input_alone() {
        let schema = zod_schema();
        let copy = schema.clone();
        for protocol in BOTH {
            let first = clean_schema(&schema, protocol).to_string();
            let second = clean_schema(&schema, protocol).to_string();
            assert_eq!(first, second);
        }
        assert_eq!(schema, copy);
    }
}
