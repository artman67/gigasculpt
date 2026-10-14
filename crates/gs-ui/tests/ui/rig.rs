//! A real engine with the real window host attached, and the shell drawn by `egui_kittest`
//! instead of a window. Whatever the interface asks for is run on the engine as the human at
//! the window, exactly as the window does it.

use egui_kittest::Harness;
use gs_engine::{EngineConfig, Origin};
use gs_protocol::{CmdResponse, Role, Step};
use gs_testkit::session::{Session, success};
use gs_ui::{Host, Keymap, Layout, LayoutFile, Shell, Studio};
use serde_json::{Value, json};

pub struct Rig {
    pub host: Host,
    pub session: Session,
}

pub fn window() -> Origin {
    Origin {
        client_id: 0,
        client: "window".into(),
        role: Role::Human,
    }
}

pub fn agent() -> Origin {
    Origin {
        client_id: 9,
        client: "claude".into(),
        role: Role::Agent,
    }
}

impl Rig {
    /// An engine whose window is a shell with `keymap` and no renderer.
    pub fn with_keymap(keymap: Keymap) -> Self {
        let mut session = Session::with(EngineConfig::default(), Role::Human);
        let layout = LayoutFile::in_memory(Layout::default());
        let shell = Shell::new(session.engine.catalog().clone(), keymap, layout);
        let host = Host::new(Studio::new(shell, None, [1000, 700], 1.0));
        session.engine.attach_window(Box::new(host.clone()));
        Self { host, session }
    }

    pub fn new() -> Self {
        Self::with_keymap(Keymap::default())
    }

    /// Runs a command as `origin`.
    pub fn call(&mut self, origin: &Origin, command: &str, args: Value) -> CmdResponse {
        self.session.origin = origin.clone();
        self.session.call(command, args)
    }

    /// Runs a command that must succeed as `origin`, and returns its result.
    pub fn ok(&mut self, origin: &Origin, command: &str, args: Value) -> Value {
        success(self.call(origin, command, args), command).result
    }

    /// True if the engine has a part called `name`.
    pub fn has_part(&mut self, name: &str) -> bool {
        self.session.origin = agent();
        self.session
            .run("part.get", json!({ "part": name }))
            .is_ok()
    }

    /// One frame of the shell, then the commands it asked for, run as the window.
    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let steps: Vec<Step> = {
            let mut studio = self.host.lock();
            studio.shell.ui(ui);
            studio.shell.take_commands()
        };
        self.session.origin = window();
        for step in steps {
            if let Err(e) = self.session.run(&step.command, step.args.clone()) {
                self.host.lock().shell.refused(Some(step), &e);
            }
        }
    }
}

/// The rig in a kittest harness of 1000 x 700 points.
pub fn harness(rig: Rig) -> Harness<'static, Rig> {
    Harness::builder()
        .with_size([1000.0, 700.0])
        .build_ui_state(|ui, rig: &mut Rig| rig.frame(ui), rig)
}
