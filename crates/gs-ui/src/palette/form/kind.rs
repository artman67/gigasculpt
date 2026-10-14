//! What an argument is, read from its schema (`args_schema`, which the argument struct's serde
//! and schemars derives make), which decides the control the form draws for it (decision 0025,
//! rule 13).

use serde_json::Value;

/// One of an enum's words, with what its doc says it does.
#[derive(Clone, Debug)]
pub struct Word {
    pub value: String,
    pub doc: Option<String>,
}

/// An argument's kind.
#[derive(Clone, Debug)]
pub enum Kind {
    /// One of a few words, or several of them when `true`: a row of lit buttons.
    Words(Vec<Word>, bool),
    /// A boolean: a switch.
    Switch,
    /// A number between two ends, a whole one when `true`: a slider.
    Slider([f64; 2], bool),
    /// A part of the scene, or several when `true`: the scene's parts by name.
    Part(bool),
    /// Words or a number: the words as buttons, then a typed number.
    WordsOrNumber(Vec<Word>),
    /// A list of numbers, typed as a list: `1, 2`.
    Numbers,
    /// Exactly three numbers, a point or a direction: three fields side by side, X, Y and Z, or
    /// R, G and B when `true` (the schema marks a color, `x-rgb`).
    Three(bool),
    /// A color: a color button, sent as `"#rrggbb"`.
    Color,
    /// Text, sent as typed: a path or a name.
    Text,
    /// A value typed as JSON where it reads as JSON, else as text: a number with no ends, a
    /// length (`0.2` or `"5%"`), a table.
    Typed,
    /// Given by a script or the window, never typed: a selection, a maker, nested settings.
    Given,
}

/// The kind of `schema`, an argument's, with its command's `defs`.
pub fn of(schema: &Value, defs: &Value) -> Kind {
    match reference(schema) {
        Some("PartRef") => return Kind::Part(false),
        Some("Parts") => return Kind::Part(true),
        Some("ColorArg") => return Kind::Color,
        Some("Length") => return Kind::Typed,
        _ => {}
    }
    let schema = resolve(schema, defs);
    if let Some(inner) = nullable(schema, defs) {
        return of(inner, defs);
    }
    if let Some(words) = words(schema, defs) {
        return Kind::Words(words, false);
    }
    if let Some(all) = alternatives(schema) {
        let number = |s: &&Value| matches!(type_of(resolve(s, defs)), "number" | "integer");
        let (named, rest): (Vec<&Value>, Vec<&Value>) =
            (all.iter()).partition(|s| words(resolve(s, defs), defs).is_some());
        if named.is_empty() || rest.is_empty() || !rest.iter().all(number) {
            return Kind::Given;
        }
        let named = named
            .into_iter()
            .filter_map(|s| words(resolve(s, defs), defs));
        return Kind::WordsOrNumber(named.flatten().collect());
    }
    match type_of(schema) {
        "boolean" => Kind::Switch,
        "number" | "integer" => match (schema["minimum"].as_f64(), schema["maximum"].as_f64()) {
            (Some(low), Some(high)) => Kind::Slider([low, high], type_of(schema) == "integer"),
            _ => Kind::Typed,
        },
        "array" => match items(&schema["items"], defs) {
            Kind::Numbers if schema["minItems"] == 3 && schema["maxItems"] == 3 => {
                Kind::Three(schema["x-rgb"] == true)
            }
            kind => kind,
        },
        "string" => Kind::Text,
        "object" if schema["properties"].is_null() => Kind::Typed,
        _ => Kind::Given,
    }
}

/// The kind of a list whose items are `items`.
fn items(items: &Value, defs: &Value) -> Kind {
    if reference(items) == Some("PartRef") {
        return Kind::Part(true);
    }
    let items = resolve(items, defs);
    if let Some(words) = words(items, defs) {
        return Kind::Words(words, true);
    }
    match type_of(items) {
        "number" | "integer" => Kind::Numbers,
        "string" => Kind::Typed,
        _ => Kind::Given,
    }
}

/// The schema of the argument `name` in a command's `args` schema; a dotted name (a slider's
/// `subsurface.strength`) is looked up inside its table.
pub fn at<'a>(args: &'a Value, name: &str) -> Option<&'a Value> {
    let defs = &args["$defs"];
    let mut schema = args;
    for (i, part) in name.split('.').enumerate() {
        if i > 0 {
            schema = resolve(schema, defs);
            schema = nullable(schema, defs).map_or(schema, |s| resolve(s, defs));
        }
        schema = schema["properties"].get(part)?;
    }
    Some(schema)
}

/// The words `schema` takes, each with its doc, if it takes only words.
fn words(schema: &Value, defs: &Value) -> Option<Vec<Word>> {
    let word = |value: &Value, doc: &Value| {
        let value = value.as_str()?.to_string();
        let doc = doc.as_str().map(String::from);
        Some(Word { value, doc })
    };
    let words: Option<Vec<Word>> = match schema["enum"].as_array() {
        Some(values) => values.iter().map(|v| word(v, &Value::Null)).collect(),
        None => (alternatives(schema)?.iter())
            .map(|s| resolve(s, defs))
            .map(|s| word(&s["const"], &s["description"]))
            .collect(),
    };
    words.filter(|w| !w.is_empty())
}

/// Whether `schema` takes null too: an argument the engine settles when it is left out.
pub fn optional(schema: &Value, defs: &Value) -> bool {
    let schema = resolve(schema, defs);
    let typed = schema["type"].as_array();
    typed.is_some_and(|all| all.iter().any(|t| t == "null")) || nullable(schema, defs).is_some()
}

/// The one alternative of `schema` besides null, if it is that or null.
fn nullable<'a>(schema: &'a Value, defs: &Value) -> Option<&'a Value> {
    let all = alternatives(schema)?;
    let null = |s: &&Value| type_of(resolve(s, defs)) == "null";
    let rest: Vec<&Value> = all.iter().filter(|s| !null(s)).collect();
    (rest.len() == 1 && all.len() > 1).then(|| rest[0])
}

/// `schema`'s alternatives (`anyOf` or `oneOf`), if it has them.
fn alternatives(schema: &Value) -> Option<&Vec<Value>> {
    schema["anyOf"].as_array().or(schema["oneOf"].as_array())
}

/// `schema`'s type, the one besides null where it lists two; empty where it has none.
fn type_of(schema: &Value) -> &str {
    match &schema["type"] {
        Value::String(t) => t,
        Value::Array(all) => (all.iter().filter_map(Value::as_str))
            .find(|t| *t != "null")
            .unwrap_or_default(),
        _ => "",
    }
}

/// The name of the definition `schema` refers to, if it is a reference.
fn reference(schema: &Value) -> Option<&str> {
    schema["$ref"].as_str()?.rsplit('/').next()
}

/// `schema` with its references followed into `defs`.
pub fn resolve<'a>(schema: &'a Value, defs: &'a Value) -> &'a Value {
    match reference(schema) {
        Some(name) if defs.get(name).is_some() => resolve(&defs[name], defs),
        _ => schema,
    }
}
