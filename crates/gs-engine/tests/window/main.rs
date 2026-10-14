//! The window commands (`pen.input`, `ui.describe`, `ui.screenshot`, `ui.palette`) and the
//! window hook: in a session without a window, with the real window host from gs-ui attached,
//! and with the session server. Everything goes through commands, the way clients use the
//! engine; calls switch between the human at the window, an agent and an observer. The
//! offscreen looks (`view.snapshot`, `view.contact_sheet`) are tested in gs-ui's `looks`.

mod headless;
mod server;
mod with_window;

use gs_engine::Origin;
use gs_protocol::Role;
use gs_testkit::session::Session;

/// The client a call comes from.
pub fn origin(role: Role) -> Origin {
    let (client_id, client) = match role {
        Role::Human => (0, "window"),
        Role::Agent => (9, "claude"),
        Role::Observer => (5, "watcher"),
    };
    Origin {
        client_id,
        client: client.into(),
        role,
    }
}

/// Lets a test session speak as any client.
pub trait As {
    /// The session, speaking as `role` from now on.
    fn by(&mut self, role: Role) -> &mut Session;
}

impl As for Session {
    fn by(&mut self, role: Role) -> &mut Session {
        self.origin = origin(role);
        self
    }
}
