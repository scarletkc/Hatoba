//! Argument checking for deferred MCP tool calls against a tool's JSON Schema (AI-45).

use std::cell::Cell;

use serde_json::{Map, Value};

use super::schema::local_target;

/// Problems [`check_arguments`] reports at most.
const MAX_PROBLEMS: usize = 10;
/// Nesting and `$ref` steps [`check_arguments`] follows.
const MAX_DEPTH: usize = 32;
/// Schemas [`check_arguments`] checks a value against at most, the forms of every combination it
/// tries included, so a schema whose combinations refer back to themselves cannot make it run for
/// long; what it has not checked by then is left to the server.
const MAX_STEPS: usize = 10_000;

/// The result of a call whose arguments do not match the tool's input schema: what does not
/// match and the whole schema.
#[must_use]
pub fn mismatch_result(name: &str, problems: &[String], schema: &Value) -> String {
    let mut out =
        format!("The call did not run: its arguments do not match the input schema of {name}.\n");
    for problem in problems {
        out.push_str(&format!("- {problem}\n"));
    }
    out.push_str(&format!(
        "Call it again with arguments that match this schema:\n{}",
        serde_json::to_string(schema).unwrap_or_default()
    ));
    out
}

/// What in `arguments` does not match `schema`, at most ten problems; none when it matches. Checks
/// types, required properties, properties the schema rules out, enumerations and constants, and
/// the combinations (`allOf`, `anyOf`, `oneOf`) and local `$ref`s of those, and leaves every other
/// keyword to the server, which validates what it receives anyway.
#[must_use]
pub fn check_arguments(schema: &Value, arguments: &Map<String, Value>) -> Vec<String> {
    let steps = Cell::new(0);
    let mut checker = Checker {
        root: schema,
        steps: &steps,
        limit: MAX_PROBLEMS,
        problems: Vec::new(),
    };
    checker.check(schema, &Value::Object(arguments.clone()), "", 0);
    checker.problems.truncate(MAX_PROBLEMS);
    checker.problems
}

struct Checker<'s> {
    root: &'s Value,
    /// The schemas checked so far, shared with the probes of combinations (see [`MAX_STEPS`]).
    steps: &'s Cell<usize>,
    /// The problems after which the check stops.
    limit: usize,
    problems: Vec<String>,
}

/// How a place in the arguments is named in a problem.
fn place(path: &str) -> String {
    if path.is_empty() {
        "The arguments".to_owned()
    } else {
        format!("`{path}`")
    }
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "an integer",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Whether two values are equal as JSON Schema compares them: numbers by their value, so `1.0`
/// is `1`, as it is an integer.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            x == y
                || x.as_f64()
                    .zip(y.as_f64())
                    .is_some_and(|(x, y)| x.total_cmp(&y).is_eq())
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

fn is_type(value: &Value, kind: &str) -> bool {
    match kind {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "number" => value.is_number(),
        "integer" => match value {
            Value::Number(n) => {
                n.is_i64() || n.is_u64() || n.as_f64().is_some_and(|f| f.fract() == 0.0)
            }
            _ => false,
        },
        // A type this check does not know is left to the server.
        _ => true,
    }
}

impl Checker<'_> {
    fn problem(&mut self, text: String) {
        if !self.problems.contains(&text) {
            self.problems.push(text);
        }
    }

    /// Whether `value` matches `schema`, without recording why not. A probe that runs out of
    /// steps matches, since what it did not check is left to the server.
    fn matches(&self, schema: &Value, value: &Value, depth: usize) -> bool {
        let mut probe = Checker {
            root: self.root,
            steps: self.steps,
            limit: 1,
            problems: Vec::new(),
        };
        probe.check(schema, value, "", depth);
        probe.problems.is_empty()
    }

    fn check(&mut self, schema: &Value, value: &Value, path: &str, depth: usize) {
        if depth > MAX_DEPTH || self.problems.len() >= self.limit || self.steps.get() >= MAX_STEPS {
            return;
        }
        self.steps.set(self.steps.get() + 1);
        let schema = match schema {
            Value::Bool(true) => return,
            Value::Bool(false) => {
                self.problem(format!("{} is not allowed.", place(path)));
                return;
            }
            Value::Object(schema) => schema,
            _ => return,
        };
        // OpenAPI's `nullable`, which some servers write (and schema.rs keeps), allows null
        // besides what the rest of the schema allows.
        if value.is_null() && schema.get("nullable").and_then(Value::as_bool) == Some(true) {
            return;
        }
        if let Some(target) = schema
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|r| local_target(self.root, r))
        {
            self.check(target, value, path, depth + 1);
        }
        if let Some(types) = schema.get("type") {
            let kinds: Vec<&str> = match types {
                Value::String(kind) => vec![kind.as_str()],
                Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
                _ => Vec::new(),
            };
            if !kinds.is_empty() && !kinds.iter().any(|kind| is_type(value, kind)) {
                let wanted = kinds
                    .iter()
                    .map(|k| match *k {
                        "integer" | "array" | "object" => format!("an {k}"),
                        "null" => "null".to_owned(),
                        k => format!("a {k}"),
                    })
                    .collect::<Vec<_>>()
                    .join(" or ");
                self.problem(format!(
                    "{} must be {wanted}, not {}.",
                    place(path),
                    kind_of(value)
                ));
                return;
            }
        }
        if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
            && !allowed.iter().any(|v| same(v, value))
        {
            let listed = allowed
                .iter()
                .map(|v| serde_json::to_string(v).unwrap_or_default())
                .collect::<Vec<_>>()
                .join(", ");
            self.problem(format!("{} must be one of {listed}.", place(path)));
        }
        if let Some(constant) = schema.get("const")
            && !same(constant, value)
        {
            self.problem(format!(
                "{} must be {}.",
                place(path),
                serde_json::to_string(constant).unwrap_or_default()
            ));
        }
        for part in schema
            .get("allOf")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            self.check(part, value, path, depth + 1);
        }
        // `oneOf` passes as `anyOf` does, with one form or more: forms that only keywords left to
        // the server tell apart would all match here, and the server checks that one does.
        for key in ["anyOf", "oneOf"] {
            if let Some(variants) = schema.get(key).and_then(Value::as_array)
                && !variants.is_empty()
                && !variants.iter().any(|v| self.matches(v, value, depth + 1))
            {
                self.problem(format!(
                    "{} matches none of the forms the schema allows.",
                    place(path)
                ));
            }
        }
        match value {
            Value::Object(object) => self.check_object(schema, object, path, depth),
            Value::Array(items) => self.check_array(schema, items, path, depth),
            _ => {}
        }
    }

    fn check_object(
        &mut self,
        schema: &Map<String, Value>,
        object: &Map<String, Value>,
        path: &str,
        depth: usize,
    ) {
        let properties = schema.get("properties").and_then(Value::as_object);
        for name in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(name) = name.as_str()
                && !object.contains_key(name)
            {
                self.problem(format!("`{}` is required.", join(path, name)));
            }
        }
        for (name, inner) in object {
            let at = join(path, name);
            match properties.and_then(|p| p.get(name)) {
                Some(property) => self.check(property, inner, &at, depth + 1),
                // Pattern properties are left to the server.
                None if schema.contains_key("patternProperties") => {}
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => {
                        let known = properties
                            .map(|p| {
                                p.keys()
                                    .map(|k| format!("`{k}`"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        if known.is_empty() {
                            self.problem(format!("`{at}` is not a property the tool takes."));
                        } else {
                            self.problem(format!(
                                "`{at}` is not a property the tool takes; it takes {known}."
                            ));
                        }
                    }
                    Some(extra @ Value::Object(_)) => self.check(extra, inner, &at, depth + 1),
                    _ => {}
                },
            }
        }
    }

    fn check_array(
        &mut self,
        schema: &Map<String, Value>,
        items: &[Value],
        path: &str,
        depth: usize,
    ) {
        let (tuple, rest) = match (schema.get("prefixItems"), schema.get("items")) {
            (Some(Value::Array(tuple)), rest) => (tuple.as_slice(), rest),
            (None, Some(Value::Array(tuple))) => (tuple.as_slice(), schema.get("additionalItems")),
            (_, rest) => (&[][..], rest),
        };
        for (i, item) in items.iter().enumerate() {
            let at = format!("{path}[{i}]");
            match tuple.get(i) {
                Some(each) => self.check(each, item, &at, depth + 1),
                None => {
                    if let Some(each) = rest.filter(|r| !r.is_array()) {
                        self.check(each, item, &at, depth + 1);
                    }
                }
            }
        }
    }
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_owned()
    } else {
        format!("{path}.{name}")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn arguments_are_checked_against_the_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "repo": {"type": "string"},
                "count": {"type": "integer"},
                "state": {"enum": ["open", "closed"]},
                "labels": {"type": "array", "items": {"type": "string"}},
                "where": {"$ref": "#/$defs/place"},
                "id": {"anyOf": [{"type": "string"}, {"type": "integer"}]},
                "mode": {"const": "fast"},
                "maybe": {"type": ["string", "null"]}
            },
            "required": ["repo"],
            "additionalProperties": false,
            "$defs": {"place": {"type": "object", "properties": {"x": {"type": "number"}}, "required": ["x"]}}
        });
        let ok = object(json!({
            "repo": "a/b", "count": 3.0, "state": "open", "labels": ["x"],
            "where": {"x": 1.5}, "id": 7, "mode": "fast", "maybe": null
        }));
        assert_eq!(check_arguments(&schema, &ok), Vec::<String>::new());
        let bad = object(json!({
            "count": "3", "state": "merged", "labels": [1], "where": {}, "id": true,
            "mode": "slow", "colour": "red", "maybe": 4
        }));
        assert_eq!(
            check_arguments(&schema, &bad),
            [
                "`repo` is required.",
                "`colour` is not a property the tool takes; it takes `count`, `id`, `labels`, `maybe`, `mode`, `repo`, `state`, `where`.",
                "`count` must be an integer, not a string.",
                "`id` matches none of the forms the schema allows.",
                "`labels[0]` must be a string, not an integer.",
                "`maybe` must be a string or null, not an integer.",
                "`mode` must be \"fast\".",
                "`state` must be one of \"open\", \"closed\".",
                "`where.x` is required."
            ]
        );
        // Keywords this check does not know, and schemas that allow anything, pass.
        assert!(
            check_arguments(
                &json!({"type": "object", "properties": {"n": {"minimum": 5}}}),
                &object(json!({"n": 1}))
            )
            .is_empty()
        );
        assert!(check_arguments(&json!(true), &object(json!({"any": 1}))).is_empty());
        // OpenAPI's `nullable` allows null, and numbers equal by value.
        let open_api = json!({"type": "object", "properties": {
            "name": {"type": "string", "nullable": true, "enum": ["a"]},
            "size": {"type": "integer", "enum": [1, 2]},
            "step": {"const": 2}
        }});
        assert!(
            check_arguments(
                &open_api,
                &object(json!({"name": null, "size": 1.0, "step": 2.0}))
            )
            .is_empty()
        );
        assert!(check_arguments(&json!({}), &object(json!({"any": 1}))).is_empty());
        let text = mismatch_result(
            "mcp__a__b",
            &["`repo` is required.".into()],
            &json!({"type": "object"}),
        );
        assert_eq!(
            text,
            "The call did not run: its arguments do not match the input schema of mcp__a__b.\n\
             - `repo` is required.\n\
             Call it again with arguments that match this schema:\n{\"type\":\"object\"}"
        );
    }

    #[test]
    fn a_schema_that_refers_back_to_itself_is_checked_in_bounded_steps() {
        // Every form refers back to the whole schema and fails only after it, so each level tries
        // eight forms: about 8^10 checks without the bound. What is not checked passes.
        let forms: Vec<Value> = [
            "string", "number", "boolean", "null", "array", "integer", "string", "object",
        ]
        .iter()
        .map(|kind| json!({"allOf": [{"$ref": "#"}, {"type": kind}]}))
        .collect();
        let schema = json!({ "anyOf": forms });
        assert!(check_arguments(&schema, &object(json!({"a": 1}))).is_empty());
        // `oneOf` passes with more than one matching form, as the comment in `check` says.
        let either = json!({"oneOf": [{"type": "object"}, {"type": "object"}]});
        assert!(check_arguments(&either, &object(json!({}))).is_empty());
    }
}
