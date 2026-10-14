//! The loaded registry: every command's spec, its catalog entry and its compiled argument
//! validator, plus argument validation that reports every violation.

use gs_protocol::{Catalog, CommandError, CommandInfo, ErrorCode, PROTOCOL_VERSION, Violation};
use jsonschema::Validator;
use jsonschema::error::ValidationErrorKind;
use serde_json::{Value, json};

use crate::command::{CommandSpec, ErasedCommand, registry};
use crate::errors::{self, did_you_mean, error};
use crate::pinning::pinned_names;

pub(crate) struct Registry {
    specs: Vec<&'static CommandSpec>,
    /// The catalog, in the same order as `specs`.
    pub catalog: Catalog,
    validators: Vec<Result<Validator, String>>,
}

impl Registry {
    /// Loads every registered command. Panics if a command's claims are not valid JSON, which
    /// every test would catch at once.
    pub fn load() -> Self {
        let specs = registry();
        let commands: Vec<CommandInfo> = specs.iter().map(|spec| info(spec)).collect();
        let hash = blake3::hash(&serde_json::to_vec(&commands).unwrap_or_default());
        let validators = commands
            .iter()
            .map(|c| jsonschema::validator_for(&c.args_schema).map_err(|e| e.to_string()))
            .collect();
        let catalog = Catalog {
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: PROTOCOL_VERSION,
            hash: hash.to_hex().to_string(),
            commands,
        };
        Self {
            specs,
            catalog,
            validators,
        }
    }

    /// A command by name, or `E_UNKNOWN_COMMAND` with did-you-mean suggestions.
    pub fn find(&self, name: &str) -> Result<&'static CommandSpec, CommandError> {
        match self.specs.binary_search_by_key(&name, |spec| spec.name) {
            Ok(index) => Ok(self.specs[index]),
            Err(_) => Err(CommandError {
                suggestions: did_you_mean(name, self.specs.iter().map(|s| s.name)),
                hint: Some(
                    "search with commands.search, or list them all with `gs commands`".into(),
                ),
                ..error(
                    ErrorCode::UnknownCommand,
                    format!("there is no command named '{name}'"),
                )
            }),
        }
    }

    /// Validates `args` against the command's schema, reporting every violation, then decodes.
    pub fn decode(
        &self,
        spec: &CommandSpec,
        args: Value,
    ) -> Result<Box<dyn ErasedCommand>, CommandError> {
        let index = self
            .specs
            .iter()
            .position(|s| s.name == spec.name)
            .unwrap_or_default();
        let info = &self.catalog.commands[index];
        let validator = self.validators[index]
            .as_ref()
            .map_err(|e| errors::internal(e))?;
        let found: Vec<(Violation, Vec<String>)> = validator
            .iter_errors(&args)
            .map(|e| explain(&e, &info.args_schema))
            .collect();
        let Some((first, suggestions)) = found.first() else {
            return (spec.decode)(args).map_err(|e| {
                let message = format!("{}: {}", info.name, e.message);
                with_help(CommandError { message, ..e }, info)
            });
        };
        let message = format!("{}: {}", info.name, first.message);
        Err(CommandError {
            suggestions: suggestions.clone(),
            violations: found.iter().map(|(v, _)| v.clone()).collect(),
            ..with_help(errors::args_invalid(&first.path, message), info)
        })
    }
}

/// Adds the fix hint and a valid example to an argument error.
fn with_help(err: CommandError, info: &CommandInfo) -> CommandError {
    CommandError {
        hint: Some(format!(
            "see the arguments with `gs call {} --help` or commands.describe",
            info.name
        )),
        example: info.examples.first().map(|e| e.args.clone()),
        ..err
    }
}

/// One schema violation in words, with did-you-mean candidates for unknown argument names.
fn explain(e: &jsonschema::ValidationError, schema: &Value) -> (Violation, Vec<String>) {
    let path = e.instance_path().to_string();
    let (message, suggestions) = match e.kind() {
        ValidationErrorKind::AdditionalProperties { unexpected } => {
            let name = unexpected.first().map_or("", String::as_str);
            let object = e
                .schema_path()
                .as_str()
                .strip_suffix("/additionalProperties")
                .unwrap_or("");
            let known = schema
                .pointer(object)
                .and_then(|s| s.get("properties"))
                .and_then(Value::as_object);
            let known = known.into_iter().flat_map(|k| k.keys().map(String::as_str));
            let violation = Violation {
                path: format!("{path}/{name}"),
                message: format!("unknown argument '{}'", unexpected.join("', '")),
            };
            return (violation, did_you_mean(name, known));
        }
        ValidationErrorKind::Not { schema } => {
            let names = schema
                .get("required")
                .map(Value::to_string)
                .unwrap_or_default();
            (
                format!("use only one of these arguments: {names}"),
                Vec::new(),
            )
        }
        _ => (e.to_string(), Vec::new()),
    };
    (Violation { path, message }, suggestions)
}

/// The catalog entry of a command.
fn info(spec: &CommandSpec) -> CommandInfo {
    let mut args_schema = (spec.args_schema)().to_value();
    if !spec.docs.is_empty() {
        args_schema["description"] = spec.docs.into();
    }
    // Commands without arguments get an empty `properties`: without it, the validator reports
    // an unknown argument as a bare "false schema" error that does not name the argument.
    if args_schema.get("properties").is_none() {
        args_schema["properties"] = json!({});
    }
    let docs = args_schema
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    CommandInfo {
        name: spec.name.to_string(),
        kind: spec.kind,
        cost: spec.cost,
        summary: docs.lines().next().unwrap_or_default().to_string(),
        docs,
        args_schema,
        output_schema: (spec.output_schema)().to_value(),
        examples: (spec.examples)(),
        human_only: spec.human_only,
        pinned: pinned_names(spec),
        claims: serde_json::from_str(spec.claims)
            .unwrap_or_else(|e| panic!("the claims of {} are not valid: {e}", spec.name)),
        setup: serde_json::from_str(spec.setup)
            .unwrap_or_else(|e| panic!("the setup of {} is not valid: {e}", spec.name)),
    }
}
