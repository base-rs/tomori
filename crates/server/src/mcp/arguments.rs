// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Strict arguments: a tool call naming an argument its tool does not declare is refused, before the tool does anything,
//! saying which and what the tool takes. The rule is read from each tool's own input schema, so that the two cannot
//! drift: [`strict`] marks every object that declares its properties with `"additionalProperties": false`, which
//! clients see in `tools/list`, and [`check`] refuses a property such an object does not declare. See
//! `design/mcp.md`.
//!
//! Only the names are checked here; each tool reads and checks its own values, through [`Arguments`], which, in a
//! build with debug assertions, as every test is, panics on reading an argument its schema does not declare, so that
//! no tool relies on one. This is kept apart so that a declarative parser of arguments may replace it whole.
use serde_json::{Map, Value};

/// The keys of a schema whose alternatives each describe what a value may be.
const ALTERNATIVES: [&str; 2] = ["anyOf", "oneOf"];

/// A tool's arguments, checked, as its handler reads them, with the schema that declares them.
#[derive(Clone, Copy)]
pub(super) struct Arguments<'a> {
    schema: &'a Value,
    value: &'a Value,
}

impl<'a> Arguments<'a> {
    /// The arguments `value`, which [`check`] took against `schema`.
    pub(super) fn new(schema: &'a Value, value: &'a Value) -> Self {
        Self { schema, value }
    }

    /// The argument `name`, which the schema must declare.
    pub(super) fn get(self, name: &str) -> Option<&'a Value> {
        debug_assert!(
            self.schema["properties"].get(name).is_some(),
            "a tool reads {name}, which its schema does not declare"
        );
        self.value.get(name)
    }

    /// The arguments whole, as they were given, for a tool that reads them as a model of its own.
    pub(super) fn value(self) -> &'a Value {
        self.value
    }

    /// The object argument `name`, if it is given, read as arguments of its own.
    pub(super) fn object(self, name: &str) -> Option<Self> {
        let value = self.get(name).filter(|value| !value.is_null())?;
        Some(Self::new(&self.schema["properties"][name], value))
    }

    /// `value`, an item of the list argument `name`, read as arguments of its own.
    pub(super) fn item(self, name: &str, value: &'a Value) -> Self {
        Self::new(&self.schema["properties"][name]["items"], value)
    }
}

/// The most unknown names an error lists, the rest counted.
const NAMES_LISTED: usize = 4;

/// The most characters of each unknown name an error repeats, since the caller chose it.
const NAME_CHARS: usize = 32;

/// Mark every object `schema` declares the properties of, at any depth, as taking no others.
pub(super) fn strict(schema: &mut Value) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if let Some(Value::Object(properties)) = object.get_mut("properties") {
        properties.values_mut().for_each(strict);
        object.insert("additionalProperties".to_owned(), Value::Bool(false));
    }
    if let Some(items) = object.get_mut("items") {
        strict(items);
    }
    for key in ALTERNATIVES {
        if let Some(Value::Array(choices)) = object.get_mut(key) {
            choices.iter_mut().for_each(strict);
        }
    }
}

/// Refuse `arguments` to the tool `tool` if they name an argument its input `schema` does not declare, at any depth
/// where the schema takes no others, as [`strict`] marks it: the error names the first object's unknown arguments and
/// lists those it takes. Arguments left out are none.
pub(super) fn check(tool: &str, schema: &Value, arguments: &Value) -> Result<(), String> {
    match arguments {
        Value::Null => Ok(()),
        Value::Object(_) => within(tool, schema, arguments, ""),
        _ => Err(format!(
            "The arguments of {tool} are an object, of each argument by its name."
        )),
    }
}

/// [`check`] of `value`, found at `path` within the arguments, against `schema`.
fn within(tool: &str, schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    // A value is taken by any alternative that declares its properties, and refused as the first refuses it.
    let choices: Vec<&Value> = ALTERNATIVES
        .iter()
        .filter_map(|key| schema[key].as_array())
        .flatten()
        .filter(|choice| choice.get("properties").is_some())
        .collect();
    if !choices.is_empty() {
        let mut refused = None;
        for choice in choices {
            match within(tool, choice, value, path) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    refused.get_or_insert(error);
                }
            }
        }
        return refused.map_or(Ok(()), Err);
    }
    match value {
        Value::Object(given) => {
            let Some(declared) = schema["properties"]
                .as_object()
                .filter(|_| schema["additionalProperties"] == false)
            else {
                return Ok(());
            };
            let mut unknown: Vec<&str> = given
                .keys()
                .filter(|name| !declared.contains_key(*name))
                .map(String::as_str)
                .collect();
            unknown.sort_unstable();
            if !unknown.is_empty() {
                return Err(refusal(tool, path, &unknown, declared));
            }
            for (name, value) in given {
                let path = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}.{name}")
                };
                within(tool, &declared[name], value, &path)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                within(tool, &schema["items"], item, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Why the object at `path` in the arguments of `tool` was refused, naming at most [`NAMES_LISTED`] of its `unknown`
/// names, each cut to [`NAME_CHARS`], and every one it `declared`, each in alphabetical order, whichever order JSON
/// objects keep.
fn refusal(tool: &str, path: &str, unknown: &[&str], declared: &Map<String, Value>) -> String {
    let mut named: Vec<String> = unknown
        .iter()
        .take(NAMES_LISTED)
        .map(|name| shown(name))
        .collect();
    if unknown.len() > NAMES_LISTED {
        named.push(format!("{} more", unknown.len() - NAMES_LISTED));
    }
    let named = match named.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => named.concat(),
    };
    let what = if unknown.len() == 1 {
        "is not an argument"
    } else {
        "are not arguments"
    };
    let of = if path.is_empty() {
        tool.to_owned()
    } else {
        format!("{tool}'s {path}")
    };
    let takes = if declared.is_empty() {
        "none".to_owned()
    } else {
        let mut takes: Vec<&str> = declared.keys().map(String::as_str).collect();
        takes.sort_unstable();
        takes.join(", ")
    };
    format!("{named} {what} of {of}; it takes: {takes}.")
}

/// An unknown argument's `name` as an error repeats it: bare if it is a plain name, and otherwise quoted, on one line
/// and cut to [`NAME_CHARS`] characters, since the caller chose it.
fn shown(name: &str) -> String {
    let plain = !name.is_empty()
        && name.chars().count() <= NAME_CHARS
        && name
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '_' || char == '-');
    if plain {
        name.to_owned()
    } else {
        format!(
            "{:?}",
            tomori_protocol::text::one_line(name, NAME_CHARS).trim()
        )
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> Value {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "note": { "type": "string" },
                "walls": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "attach": { "type": "object", "properties": { "to": { "type": "string" } } },
                        },
                    },
                },
                "parameters": { "type": "object" },
                "target": { "anyOf": [{ "type": "integer" }, { "type": "object", "properties": { "x": { "type": "number" } } }] },
            },
        });
        strict(&mut schema);
        schema
    }

    #[test]
    fn every_object_that_declares_its_properties_takes_no_others() {
        let schema = schema();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["walls"]["items"]["additionalProperties"],
            false
        );
        assert_eq!(
            schema["properties"]["walls"]["items"]["properties"]["attach"]["additionalProperties"],
            false
        );
        // An object of parameters chosen elsewhere declares none, and stays open.
        assert!(
            schema["properties"]["parameters"]
                .get("additionalProperties")
                .is_none()
        );
        let mut flat = json!(true);
        strict(&mut flat);
        assert_eq!(flat, true);
    }

    #[test]
    fn declared_arguments_are_taken_and_others_refused_naming_them() {
        let schema = schema();
        for arguments in [
            Value::Null,
            json!({}),
            json!({"note": "x", "walls": [{"id": "a", "attach": {"to": "b"}}], "parameters": {"anything": 1}}),
            json!({"walls": "not a list"}),
        ] {
            assert_eq!(check("tool", &schema, &arguments), Ok(()), "{arguments}");
        }
        assert_eq!(
            check(
                "tool",
                &schema,
                &json!({"note": "x", "when_present": false})
            ),
            Err(
                "when_present is not an argument of tool; it takes: note, parameters, target, walls."
                    .to_owned()
            )
        );
        assert_eq!(
            check("tool", &schema, &json!({"a": 1, "b": 2})),
            Err(
                "a and b are not arguments of tool; it takes: note, parameters, target, walls."
                    .to_owned()
            )
        );
        assert_eq!(
            check(
                "tool",
                &schema,
                &json!({"walls": [{"id": "a"}, {"id": "b", "tilts": 3}]})
            ),
            Err("tilts is not an argument of tool's walls[1]; it takes: attach, id.".to_owned())
        );
        assert_eq!(
            check(
                "tool",
                &schema,
                &json!({"walls": [{"id": "a", "attach": {"to": "b", "edge": "left", "side": 1}}]})
            ),
            Err(
                "edge and side are not arguments of tool's walls[0].attach; it takes: to."
                    .to_owned()
            )
        );
        assert_eq!(
            check("tool", &schema, &json!([1])),
            Err("The arguments of tool are an object, of each argument by its name.".to_owned())
        );
        let mut none = json!({"type": "object", "properties": {}});
        strict(&mut none);
        assert_eq!(
            check("server", &none, &json!({"verbose": true})),
            Err("verbose is not an argument of server; it takes: none.".to_owned())
        );
    }

    #[test]
    fn an_object_among_alternatives_takes_only_what_it_declares() {
        let schema = schema();
        assert_eq!(
            schema["properties"]["target"]["anyOf"][1]["additionalProperties"],
            false
        );
        assert!(
            schema["properties"]["target"]["anyOf"][0]
                .get("additionalProperties")
                .is_none()
        );
        for target in [json!(3), json!({"x": 1})] {
            assert_eq!(check("tool", &schema, &json!({ "target": target })), Ok(()));
        }
        assert_eq!(
            check("tool", &schema, &json!({"target": {"x": 1, "y": 2}})),
            Err("y is not an argument of tool's target; it takes: x.".to_owned())
        );
    }

    #[test]
    fn a_handler_reads_declared_arguments_and_those_within_them() {
        let schema = schema();
        let value = json!({"note": "x", "walls": [{"id": "a", "attach": {"to": "b"}}]});
        let arguments = Arguments::new(&schema, &value);
        assert_eq!(arguments.get("note"), Some(&json!("x")));
        assert!(arguments.object("parameters").is_none());
        let wall = arguments.item("walls", &value["walls"][0]);
        assert_eq!(wall.get("id"), Some(&json!("a")));
        let attach = wall.object("attach").unwrap();
        assert_eq!(attach.get("to"), Some(&json!("b")));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "a tool reads when_present, which its schema does not declare")]
    fn a_handler_reading_an_argument_its_schema_does_not_declare_panics_in_tests() {
        let schema = schema();
        let value = json!({});
        Arguments::new(&schema, &value).get("when_present");
    }

    #[test]
    fn the_error_stays_short_however_many_or_long_the_unknown_names() {
        let schema = schema();
        let mut arguments = Map::new();
        for index in 0..1000 {
            arguments.insert(format!("{index}{}", "x".repeat(10_000)), json!(1));
        }
        arguments.insert("odd, name; it takes\n".to_owned(), json!(1));
        let error = check("tool", &schema, &Value::Object(arguments)).unwrap_err();
        assert!(error.len() < 400, "{error}");
        assert!(
            error.contains(" and 997 more are not arguments of tool"),
            "{error}"
        );
        assert!(
            error.contains(&format!("\"0{}…\"", "x".repeat(NAME_CHARS - 1))),
            "{error}"
        );
        assert_eq!(
            check("tool", &schema, &json!({"odd, name\n": 1})),
            Err(
                "\"odd, name\" is not an argument of tool; it takes: note, parameters, target, walls."
                    .to_owned()
            )
        );
        assert_eq!(
            check("tool", &schema, &json!({"": 1})),
            Err(
                "\"\" is not an argument of tool; it takes: note, parameters, target, walls."
                    .to_owned()
            )
        );
    }
}
