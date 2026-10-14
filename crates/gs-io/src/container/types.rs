//! Values the brush, tool and project files share: the preamble, pictures, named settings and
//! objects kept as stored.

/// The text before the first object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preamble {
    /// The copyright line, without padding.
    pub copyright: String,
    /// A format number (1007 or 1008 in every file seen).
    pub format: u32,
    /// The version text of the program that wrote the file; empty in newer files.
    pub writer: String,
}

/// A picture with the name it is stored under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedImage {
    pub name: String,
    pub image: Image,
}

/// A decoded image: `width * height` pixels of `bytes_per_pixel` little-endian bytes, rows top
/// to bottom. Mesh arrays use the same storage with a width of one element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub bytes_per_pixel: u16,
    pub data: Vec<u8>,
}

impl Image {
    /// The pixels as 16-bit values, for 2-byte images.
    pub fn gray16(&self) -> Option<Vec<u16>> {
        (self.bytes_per_pixel == 2).then(|| {
            self.data
                .as_chunks()
                .0
                .iter()
                .map(|&p| u16::from_le_bytes(p))
                .collect()
        })
    }
}

/// One named setting record.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub name: String,
    /// Which group of records this one is in (0 to 3 in every brush file seen; always 0 in
    /// tool files); a name can repeat in different groups.
    pub group: u8,
    pub kind: SettingKind,
    /// The on/off state.
    pub on: bool,
    pub value: f32,
    /// A second value, 0 in every brush file seen.
    pub second: f32,
    /// The record head as stored: kind, flags and four bytes not understood yet.
    pub head: [u8; 7],
    pub curve: Option<Curve>,
}

/// What a setting record is, from its first head byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    Slider,
    Switch,
    Curve,
    Other(u8),
}

impl SettingKind {
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            0x23 => Self::Slider,
            0x81 => Self::Switch,
            0x29 => Self::Curve,
            other => Self::Other(other),
        }
    }
}

/// An editable curve: its points and the header bytes before them.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    /// Curve object version (9 or 10 seen; 10 has 4 more header bytes).
    pub version: u16,
    /// Everything before the points, including the point count.
    pub head: Vec<u8>,
    pub points: Vec<CurvePoint>,
}

/// One curve point, `x` and `y` from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
    /// Point kind (1 in nearly every point seen; 0, 4 and 5 occur).
    pub kind: u32,
    /// 1 in every point seen.
    pub weight: f32,
}

/// An object kept as stored: its version and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawObject {
    pub version: u16,
    pub bytes: Vec<u8>,
}
