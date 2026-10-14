//! A view as `view.get` reports it.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Serialize;

use super::{NavSettings, OrbitArg, PivotRule, Views};
use crate::document::Document;
use crate::view::secondary::ReferenceInfo;
use crate::view::spec::{ExactCamera, NamedView};

/// A view, as `view.get` reports it.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ViewInfo {
    /// Its name.
    pub view: String,
    /// Its camera, exactly; pass it to `view.set` or a look's `view` to get the same camera.
    pub camera: ExactCamera,
    /// The side it looks from while it looks along an axis view (within a degree, whatever its
    /// roll): front, back, left, right, top or bottom; absent between them. The window's 3D
    /// view names the same side in its corner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub looks_from: Option<NamedView>,
    /// True while no command has set the camera: it is the front view framing every part.
    pub framed: bool,
    /// The image size it is seen at, `[width, height]` in pixels.
    pub viewport: [u32; 2],
    /// Locked views refuse every camera change.
    pub locked: bool,
    /// What a horizontal drag turns about.
    pub orbit: OrbitArg,
    /// How it finds the point it turns about.
    pub pivot_rule: PivotRule,
    /// That point now.
    pub pivot: gs_protocol::Vec3,
    /// How its moves behave (`view.settings`).
    #[serde(flatten)]
    pub nav: NavSettings,
    /// Its edges are drawn over the surface (`view.look`).
    pub wireframe: bool,
    /// Where the last stroke on its behalf first touched the surface.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_edit: Option<gs_protocol::Vec3>,
    /// Steps `view.undo` and `view.redo` can take.
    pub undo: u32,
    pub redo: u32,
    /// The gesture in progress, by id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gesture: Option<u64>,
    /// A Reference View's setup: where its scene came from, what follows main, its alignment
    /// and its look.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<ReferenceInfo>,
}

impl Views {
    /// Every view in use, as `view.get` reports each.
    pub fn report(&self, doc: &Document) -> BTreeMap<String, ViewInfo> {
        (self.list.keys())
            .map(|name| (name.clone(), self.info(name, doc)))
            .collect()
    }

    /// View `name` as `view.get` reports it.
    pub(crate) fn info(&self, name: &str, doc: &Document) -> ViewInfo {
        let view = self.view(name);
        let camera = self.resolved(name, doc);
        let reference = (view.secondary.as_ref()).map(|s| {
            let parts = s.scene.doc(doc).names().map(String::from).collect();
            s.info(parts)
        });
        ViewInfo {
            view: name.to_string(),
            looks_from: camera.side().map(NamedView::of),
            camera: camera.into(),
            framed: view.camera.is_none(),
            viewport: view.viewport,
            locked: view.locked,
            orbit: view.orbit,
            pivot_rule: view.pivot,
            pivot: gs_protocol::Vec3(self.pivot(name, doc, None)),
            nav: view.nav,
            wireframe: self.wireframe(name),
            last_edit: view.last_edit.map(gs_protocol::Vec3),
            undo: view.undo.len() as u32,
            redo: view.redo.len() as u32,
            gesture: view.gesture.as_ref().map(|g| g.id),
            reference,
        }
    }
}
