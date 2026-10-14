//! The stroke class a preset uses and its settings: how pointer movement becomes dabs. What a
//! dab stands for is specified in `crate::brush` ("Dabs"); `crate::stroke` runs each class.

use serde::{Deserialize, Serialize};

/// How pointer movement becomes dabs: the stroke class and its settings.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "class", rename_all = "snake_case", deny_unknown_fields)]
pub enum StrokeSpec {
    /// Spaced dabs along the path, accumulating: the Dots and Freehand strokes, which differ only
    /// in spacing (decision 0036 section 1).
    Dab {
        /// Distance between dabs, as a fraction of the brush diameter: the Step, a whole number
        /// of which separates each dab from the press. Dots' 0.2 and Freehand's 0.75, read in
        /// radii, are 0.1 and 0.375.
        #[serde(default = "tenth")]
        spacing: f32,
        /// The stroke length a press stands for, in radii.
        #[serde(default = "half")]
        tap: f32,
        #[serde(default)]
        smoothing: Smoothing,
        /// Directional: only stroke that moves away from the press lays dabs; going back, or
        /// round at one distance, adds nothing. Written only when on, so the hash of a preset
        /// without it stays as it was (decision 0036 section 4).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        directional: bool,
    },
    /// The result follows the stroke-start surface and the latest pointer position (Move).
    Drag {
        #[serde(default)]
        smoothing: Smoothing,
    },
    /// One dab, redone from the stroke-start surface on every update: centered on the press
    /// and sized by the drag, or (`follow`) at the pointer with the brush size.
    Anchored {
        #[serde(default)]
        follow: bool,
        #[serde(default = "half")]
        tap: f32,
        /// Smooths the point that sizes the dab. Written only when on, so the hash of a preset
        /// without it stays as it was (decision 0036).
        #[serde(default, skip_serializing_if = "Smoothing::is_off")]
        smoothing: Smoothing,
    },
    /// Dabs laid along the whole path when the stroke ends; `straight` uses only its ends.
    Curve {
        #[serde(default = "tenth")]
        spacing: f32,
        #[serde(default)]
        straight: bool,
        /// Smooths the path held. Written only when on.
        #[serde(default, skip_serializing_if = "Smoothing::is_off")]
        smoothing: Smoothing,
    },
    /// A screen-space gesture that runs once at stroke end (ARCHITECTURE 4.2.1).
    Gesture {
        shape: Shape,
        /// The rectangle's Square: both sides as long as the drag's longer part across the
        /// screen. Only the rectangle reads it; written only when on (decision 0051).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        square: bool,
        /// The rectangle's Center: the press is its middle and the release a corner. Only the
        /// rectangle reads it; written only when on.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        center: bool,
        /// Smooths the outline. Written only when on.
        #[serde(default, skip_serializing_if = "Smoothing::is_off")]
        smoothing: Smoothing,
    },
}

impl StrokeSpec {
    /// The class, as a preset names it: `dab`, `drag`, `anchored`, `curve` or `gesture`.
    pub fn class(&self) -> &'static str {
        match self {
            StrokeSpec::Dab { .. } => "dab",
            StrokeSpec::Drag { .. } => "drag",
            StrokeSpec::Anchored { .. } => "anchored",
            StrokeSpec::Curve { .. } => "curve",
            StrokeSpec::Gesture { .. } => "gesture",
        }
    }

    /// The stroke's Stroke Smoothing, which every class has (decision 0036).
    pub fn smoothing(&self) -> Smoothing {
        match *self {
            StrokeSpec::Dab { smoothing, .. }
            | StrokeSpec::Drag { smoothing }
            | StrokeSpec::Anchored { smoothing, .. }
            | StrokeSpec::Curve { smoothing, .. }
            | StrokeSpec::Gesture { smoothing, .. } => smoothing,
        }
    }

    /// The Step: a dab or curve stroke's spacing, in brush diameters; `None` for the others.
    pub fn spacing(&self) -> Option<f32> {
        match *self {
            StrokeSpec::Dab { spacing, .. } | StrokeSpec::Curve { spacing, .. } => Some(spacing),
            _ => None,
        }
    }

    /// Whether only stroke moving away from the press lays dabs (a dab stroke's Directional).
    pub fn directional(&self) -> bool {
        matches!(
            self,
            StrokeSpec::Dab {
                directional: true,
                ..
            }
        )
    }

    /// Whether the stroke runs along a path through points the sculptor taps in with Alt: a
    /// straight curve stroke (Clay Line) and a curve gesture (Clip Curve; decision 0055).
    pub fn takes_points(&self) -> bool {
        matches!(
            self,
            StrokeSpec::Curve { straight: true, .. }
                | StrokeSpec::Gesture {
                    shape: Shape::Curve,
                    ..
                }
        )
    }

    /// Whether Space held moves what the stroke has drawn instead of drawing on: a path through
    /// points ([`StrokeSpec::takes_points`]) and Drag Rect's rectangle (decision 0062, row 782).
    pub fn moves(&self) -> bool {
        self.takes_points() || matches!(self, StrokeSpec::Anchored { follow: false, .. })
    }
}

impl Default for StrokeSpec {
    fn default() -> StrokeSpec {
        StrokeSpec::Dab {
            spacing: tenth(),
            tap: half(),
            smoothing: Smoothing::default(),
            directional: false,
        }
    }
}

/// The settings of the stroke table `from` that a stroke of `class` reads, without the class:
/// each is kept when it reads as a setting of that class on its own, so what carries over to
/// another class cannot drift from the classes (decision 0051, section 6). Stroke Smoothing
/// carries to every class, `tap` between dab and anchored, `spacing` between dab and curve.
pub fn carried(from: &toml::Table, class: &str) -> toml::Table {
    let reads = |key: &str, value: &toml::Value| {
        let alone = [("class", class.into()), (key, value.clone())];
        let alone = toml::Table::from_iter(alone.map(|(k, v)| (k.to_string(), v)));
        key != "class" && StrokeSpec::deserialize(toml::Value::Table(alone)).is_ok()
    };
    (from.iter())
        .filter(|(k, v)| reads(k, v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// The stroke types the picker offers (decision 0051, section 7), in that section's order with
/// the types not built yet left out. Each is a class and one setting: Dots and Freehand are the dab
/// class at the brush's own Step and at Freehand's, Drag Rect and Drag Dot the anchored class
/// without and with `follow`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrokeType {
    Dots,
    DragRect,
    Freehand,
    DragDot,
}

/// Freehand's Step: 0.75 radii, in brush diameters (decision 0036).
const FREEHAND: f32 = 0.375;

impl StrokeType {
    /// Every type, in the picker's order: a type's index is its place here. The index is never
    /// saved, so a type joining the list later changes no brush.
    pub const ALL: [StrokeType; 4] = [
        StrokeType::Dots,
        StrokeType::DragRect,
        StrokeType::Freehand,
        StrokeType::DragDot,
    ];

    /// The name `brush.set` takes and the name the picker shows.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            StrokeType::Dots => ("dots", "Dots"),
            StrokeType::DragRect => ("drag_rect", "Drag Rect"),
            StrokeType::Freehand => ("freehand", "Freehand"),
            StrokeType::DragDot => ("drag_dot", "Drag Dot"),
        }
    }

    /// The name `brush.set` takes.
    pub fn name(self) -> &'static str {
        self.names().0
    }

    /// The name the picker shows.
    pub fn title(self) -> &'static str {
        self.names().1
    }

    /// The type named `name`.
    pub fn named(name: &str) -> Option<StrokeType> {
        StrokeType::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The stroke class the type runs.
    pub fn class(self) -> &'static str {
        match self {
            StrokeType::Dots | StrokeType::Freehand => "dab",
            StrokeType::DragRect | StrokeType::DragDot => "anchored",
        }
    }

    /// The one stroke setting that picking the type writes, and its value; `None` drops it, so
    /// the brush's own value applies (or the class default): Dots drops the Step, Drag Rect
    /// drops `follow`.
    pub fn setting(self) -> (&'static str, Option<toml::Value>) {
        match self {
            StrokeType::Dots => ("spacing", None),
            StrokeType::DragRect => ("follow", None),
            StrokeType::Freehand => ("spacing", Some(f64::from(FREEHAND).into())),
            StrokeType::DragDot => ("follow", Some(true.into())),
        }
    }

    /// The type a stroke shows: Freehand for a dab stroke at Freehand's Step and Dots for any
    /// other (the Step is all that tells them apart), Drag Dot for an anchored stroke with
    /// `follow` and Drag Rect for one without; none for drag, curve and gesture strokes.
    pub fn of(stroke: &StrokeSpec) -> Option<StrokeType> {
        match *stroke {
            StrokeSpec::Dab { spacing, .. } if spacing.to_bits() == FREEHAND.to_bits() => {
                Some(StrokeType::Freehand)
            }
            StrokeSpec::Dab { .. } => Some(StrokeType::Dots),
            StrokeSpec::Anchored { follow: true, .. } => Some(StrokeType::DragDot),
            StrokeSpec::Anchored { .. } => Some(StrokeType::DragRect),
            _ => None,
        }
    }
}

fn half() -> f32 {
    0.5
}

fn tenth() -> f32 {
    0.1
}

/// The gesture of a gesture stroke: points on a plane facing the camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// An open path with a shadow side (Clip Curve).
    Curve,
    /// A closed outline through every point (a lasso).
    Polygon,
    /// A rectangle square to the screen from the press to the release, corner to corner
    /// (Select Rect; decision 0051, section 4).
    Rect,
}

/// Stroke Smoothing: the paint point trails the pointer on a string. Its Step is a dab stroke's
/// `spacing`, in brush diameters, so it follows the brush size (Relative, always on; decision
/// 0036 section 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Smoothing {
    /// String length, in brush radii. 0 turns smoothing off.
    pub radius: f32,
    /// Extra lag, 0 to 0.99: how much of the old point each move keeps.
    pub factor: f32,
}

impl Smoothing {
    /// No string and no lag: the paint point is the pointer.
    pub fn is_off(&self) -> bool {
        *self == Smoothing::default()
    }
}
