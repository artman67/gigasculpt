//! The `Command` trait and the registry that collects every command.
//!
//! A command is a struct whose fields are its arguments. Its doc comments become the help text,
//! its serde and schemars derives become the argument schema, and one `register_command!` line
//! adds it to the catalog with its title, what a person reads for it. Behavior that many
//! commands share (which part, which random seed, who may run it, whether it is logged) is
//! declared with constants, and the engine carries it out in one place, so no command repeats
//! it.

use gs_protocol::{CommandError, Cost, ExampleInfo, Kind};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{Ctx, Example};

/// Why a command cannot run right now. The reason is mandatory text, so an agent never sees a
/// bare "not available".
#[derive(Debug)]
pub struct Unavailable {
    /// What is wrong, in one plain sentence.
    pub reason: String,
    /// How to fix it, if there is a known way.
    pub fix: Option<String>,
    /// The argument that would let the call run, when giving it is the fix: the error names
    /// it as its path and the window asks for it (decision 0016, section 17).
    pub asks: Option<&'static str>,
}

impl Unavailable {
    /// A reason with no known fix.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            fix: None,
            asks: None,
        }
    }

    /// Adds how to fix it.
    #[must_use]
    pub fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    /// Names the argument whose value would let the call run.
    #[must_use]
    pub fn asks(mut self, argument: &'static str) -> Self {
        self.asks = Some(argument);
        self
    }
}

/// How the engine fills in and checks a command's part argument before the command runs, so
/// the log records exactly which part was meant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartArg {
    /// The command has no part argument.
    None,
    /// `part` names an existing part. When it is left out, it means the active part (decision
    /// 0033).
    Existing,
    /// `name` names a new part, which must not exist yet. When it is left out, it becomes the
    /// given base name, or `base-2`, `base-3`... whichever is free.
    New(&'static str),
}

/// The doc of a `part` argument that [`PartArg::Existing`] fills in: its own first sentence,
/// then the one every such argument shares, so the command docs and the schemas read one
/// wording. `#[doc = crate::command::part_doc!("The part to copy.")]`.
macro_rules! part_doc {
    ($first:literal) => {
        concat!($first, " Defaults to the active part.")
    };
}
pub(crate) use part_doc;

/// `true`: the serde default of a switch that is on unless the call turns it off,
/// `#[serde(default = "crate::command::yes")]`.
pub(crate) fn yes() -> bool {
    true
}

/// How the engine fills in a command's view arguments before it runs (decision 0017), so the
/// log records exactly which view, image size and pivot were meant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewArg {
    /// The command has no view argument.
    None,
    /// `view` names a session view; left out, the caller's own (`main` for the human,
    /// `agent:<name>` for an agent).
    Name,
    /// As `Name`, and `viewport` left out becomes the size the view is seen at.
    View,
    /// As `View`, and `pivot` left out becomes the point the view turns about, by its rule.
    Pivot,
    /// As `View`, and a `last-edit` target becomes the `point` target at the view's last edit
    /// point (`view.frame`): the point is the caller's view's, which a replay under another
    /// client name does not have (review 0003 M4).
    Frame,
    /// `view` names an open Reference View (decision 0020); left out, the active one (decision
    /// 0024: the one opened last or moved to by `reference.next`).
    Reference,
    /// `view` names a Reference View to open; left out, `ref:1`, or the next free number. And
    /// `viewport` left out becomes the size it is seen at.
    NewReference,
}

/// One action the engine can perform. See ARCHITECTURE section 7.1.
pub trait Command: Serialize + DeserializeOwned + JsonSchema + Send + Sized + 'static {
    /// Lowercase dotted name, noun first: `mesh.stats`, `primitive.sphere`.
    const NAME: &'static str;
    /// What the command may do.
    const KIND: Kind;
    /// How long it is expected to take.
    const COST: Cost = Cost::Short;
    /// The command's docs, when one type serves several commands. Otherwise the doc comment.
    const DOCS: &'static str = "";
    /// How the engine treats the part argument.
    const PART: PartArg = PartArg::None;
    /// True for commands with a `seed` argument: when it is left out, the engine fills in the
    /// command's own seed, derived from the session seed and its sequence number.
    const RANDOM: bool = false;
    /// For commands that read a file: the argument naming it. When `hash` is left out, the
    /// engine fills in the BLAKE3 hash of the file's bytes, so the log records which content
    /// the call read, and it refuses the call when a given `hash` no longer matches
    /// (ARCHITECTURE 8).
    const READS_FILE: Option<&'static str> = None;
    /// How the engine treats the view arguments.
    const VIEW: ViewArg = ViewArg::None;
    /// For commands that take a camera as a view spec: the argument holding it. A session
    /// view's name there (`"main"`) is pinned to that view's exact camera, so the log does not
    /// depend on where the view was when it replays.
    const CAMERA: Option<&'static str> = None;
    /// True if only the human at the window may run it.
    const HUMAN_ONLY: bool = false;
    /// True for commands whose call always runs as a job, from every caller, the window
    /// included (saves and opens, decision 0016): the engine thread only admits it and takes
    /// the session's copy. Its run changes nothing the session holds; what must change on the
    /// engine thread it hands over with `Ctx::land`, applied when the call commits. An
    /// in-process caller holding the engine runs the same job on its own thread.
    const BACKGROUND: bool = false;
    /// True if a successful call is written to the command log. Everything but queries is,
    /// except commands that change who may edit rather than what is in the session
    /// (`session.control`): a replay never needs them.
    const LOGGED: bool = !matches!(Self::KIND, Kind::Query);
    /// Claims as a JSON array: behavior this command promises, which the test kit checks on
    /// random command sequences. Vocabulary and meaning: `gs-testkit`, module `claims`. Every
    /// edit command is checked for determinism, exact undo and atomicity without declaring
    /// anything; list what it promises beyond that, for example
    /// `["preserves_topology", {"local": {"part": "/part"}}]`. Weakening a claim is a visible
    /// one-line change.
    const CLAIMS: &'static str = "[]";
    /// The calls that put a part in the state this command needs before it changes anything,
    /// as a JSON array of steps written as an example's setup is, without the `part` argument:
    /// `[{"command": "transform.apply", "args": {"op": {"move": [0.5, 0, 0]}}}]`. Only for a
    /// command that random command sequences would check by luck: the test kit then checks its
    /// claims on plans of its own that start with these calls on every part (`gs-testkit`,
    /// module `claims`), and a setup call that fails or changes nothing fails that run.
    const SETUP: &'static str = "[]";
    /// The result. Gets a JSON Schema in the catalog too.
    type Output: Serialize + JsonSchema;

    /// Examples. Tests run every one, so they cannot rot. Every edit command needs at least one.
    fn examples() -> Vec<Example<Self>>;

    /// Fills in and checks the arguments only this command knows how to pin, after they decode
    /// and before the call runs and is logged, so the log records exactly what it used. The
    /// pinning many commands share (part, seed, file hash) is declared with the constants above
    /// instead. `brush.stroke` pins the hashes of its brush and alpha here (decision 0018).
    fn pin(&mut self, _cx: &Ctx) -> Result<(), CommandError> {
        Ok(())
    }

    /// Says why the command cannot run in the current state, in words.
    fn check(&self, _cx: &Ctx) -> Result<(), Unavailable> {
        Ok(())
    }

    /// Does the work. Edits go through `cx.doc`, which records undo automatically.
    fn run(self, cx: &mut Ctx) -> Result<Self::Output, CommandError>;
}

/// A command with its type erased, as the executor sees it.
pub(crate) trait ErasedCommand {
    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError>;
    fn check(&self, cx: &Ctx) -> Result<(), Unavailable>;
    /// The arguments with every default filled in, as written to the log.
    fn to_json(&self) -> Value;
    fn run(self: Box<Self>, cx: &mut Ctx) -> Result<Value, CommandError>;
}

impl<T: Command> ErasedCommand for T {
    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        Command::pin(self, cx)
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        Command::check(self, cx)
    }

    fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    fn run(self: Box<Self>, cx: &mut Ctx) -> Result<Value, CommandError> {
        let output = Command::run(*self, cx)?;
        serde_json::to_value(output).map_err(|e| crate::errors::internal(&e.to_string()))
    }
}

/// A registered command: its constants plus type-erased constructors.
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per Command constant, read by the engine in one place each"
)]
pub struct CommandSpec {
    /// Command name.
    pub name: &'static str,
    /// What a person reads for it, as a menu item reads ("Add sphere"): an undo step's name.
    /// Given where it is registered; empty if it was not.
    pub title: &'static str,
    /// What it may do.
    pub kind: Kind,
    /// How long it takes.
    pub cost: Cost,
    pub(crate) docs: &'static str,
    pub(crate) part: PartArg,
    pub(crate) random: bool,
    pub(crate) reads_file: Option<&'static str>,
    pub(crate) view: ViewArg,
    pub(crate) camera: Option<&'static str>,
    pub(crate) human_only: bool,
    pub(crate) background: bool,
    pub(crate) logged: bool,
    pub(crate) claims: &'static str,
    pub(crate) setup: &'static str,
    pub(crate) examples: fn() -> Vec<ExampleInfo>,
    pub(crate) fields: fn() -> &'static [&'static str],
    pub(crate) args_schema: fn() -> Schema,
    pub(crate) output_schema: fn() -> Schema,
    pub(crate) decode: fn(Value) -> Result<Box<dyn ErasedCommand>, CommandError>,
}

impl CommandSpec {
    /// The spec of command type `T`. Used by `register_command!`.
    pub const fn of<T: Command>() -> Self {
        Self {
            name: T::NAME,
            title: "",
            kind: T::KIND,
            cost: T::COST,
            docs: T::DOCS,
            part: T::PART,
            random: T::RANDOM,
            reads_file: T::READS_FILE,
            view: T::VIEW,
            camera: T::CAMERA,
            human_only: T::HUMAN_ONLY,
            background: T::BACKGROUND,
            logged: T::LOGGED,
            claims: T::CLAIMS,
            setup: T::SETUP,
            examples: examples_of::<T>,
            fields: fields_of::<T>,
            args_schema: schema_of::<T>,
            output_schema: schema_of::<T::Output>,
            decode: decode_as::<T>,
        }
    }

    /// The spec with its title. Used by `register_command!`.
    #[must_use]
    pub const fn titled(mut self, title: &'static str) -> Self {
        self.title = title;
        self
    }

    /// Its arguments' names in the order its struct declares them, which the schema's
    /// properties, a map sorted by name, lose (the adjust panel lists them so, decision 0062);
    /// empty when its arguments are not one plain struct.
    pub fn fields(&self) -> &'static [&'static str] {
        (self.fields)()
    }
}

fn schema_of<T: JsonSchema>() -> Schema {
    SchemaGenerator::default().into_root_schema_for::<T>()
}

/// `T`'s field names as its serde derive gives them to a deserializer asked for a struct.
fn fields_of<T: DeserializeOwned>() -> &'static [&'static str] {
    match T::deserialize(Fields) {
        Err(Probe(fields)) => fields,
        Ok(_) => &[],
    }
}

/// A deserializer that reads nothing: asked for a struct it fails with the struct's field names,
/// and asked for anything else with none.
struct Fields;

/// What [`Fields`] fails with: the field names it was asked for.
#[derive(Debug)]
struct Probe(&'static [&'static str]);

impl std::fmt::Display for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fields {:?}", self.0)
    }
}

impl std::error::Error for Probe {}

impl serde::de::Error for Probe {
    fn custom<M: std::fmt::Display>(_: M) -> Self {
        Probe(&[])
    }
}

impl<'de> serde::Deserializer<'de> for Fields {
    type Error = Probe;

    fn deserialize_any<V: serde::de::Visitor<'de>>(self, _: V) -> Result<V::Value, Probe> {
        Err(Probe(&[]))
    }

    fn deserialize_struct<V: serde::de::Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Probe> {
        Err(Probe(fields))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct map enum identifier
        ignored_any
    }
}

fn examples_of<T: Command>() -> Vec<ExampleInfo> {
    T::examples().into_iter().map(Example::info).collect()
}

/// Decodes arguments, reporting the JSON pointer of the value the decoder refused.
fn decode_as<T: Command>(args: Value) -> Result<Box<dyn ErasedCommand>, CommandError> {
    match serde_path_to_error::deserialize::<_, T>(args) {
        Ok(command) => Ok(Box::new(command)),
        Err(err) => Err(crate::errors::decode_error(&err)),
    }
}

inventory::collect!(CommandSpec);

/// Every registered command, sorted by name.
pub fn registry() -> Vec<&'static CommandSpec> {
    let mut specs: Vec<&'static CommandSpec> = inventory::iter::<CommandSpec>.into_iter().collect();
    specs.sort_by_key(|spec| spec.name);
    specs
}

/// Registers a command type with its title, what a person reads for it: one line next to the
/// command's definition, `register_command!(PrimitiveSphere, "Add sphere");`. Without a title
/// it still builds, so a command written before titles merges, and the catalog test names it.
#[macro_export]
macro_rules! register_command {
    ($command:ty, $title:literal) => {
        $crate::inventory::submit! { $crate::CommandSpec::of::<$command>().titled($title) }
    };
    ($command:ty) => {
        $crate::inventory::submit! { $crate::CommandSpec::of::<$command>() }
    };
}

/// The registered command named `name`.
pub(crate) fn spec_named(name: &str) -> Option<&'static CommandSpec> {
    inventory::iter::<CommandSpec>
        .into_iter()
        .find(|spec| spec.name == name)
}
