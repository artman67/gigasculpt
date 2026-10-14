//! Builds brush files from the layout in `docs/decisions/0004-zbp-format.md`: our own bytes,
//! written from the format notes, never copied from a real file.

pub const START: [u8; 4] = [0x03, 0x80, 0x01, 0x00];
pub const END: [u8; 4] = [0x03, 0x88, 0x00, 0x00];

pub fn object(version: u16, body: &[u8]) -> Vec<u8> {
    let len = u32::try_from(body.len() + 14).unwrap();
    [
        &START[..],
        &version.to_le_bytes(),
        &len.to_le_bytes(),
        body,
        &END,
    ]
    .concat()
}

/// Our own run-length encoder: three or more equal bytes become a run (at most 127 long),
/// everything else literal stretches (at most 128 long).
pub fn pack_runs(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut literal: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let run = data[i..]
            .iter()
            .take(127)
            .take_while(|&&b| b == data[i])
            .count();
        if run >= 3 {
            flush_literal(&mut out, &mut literal);
            out.extend([u8::try_from(run).unwrap(), data[i]]);
            i += run;
        } else {
            literal.push(data[i]);
            if literal.len() == 128 {
                flush_literal(&mut out, &mut literal);
            }
            i += 1;
        }
    }
    flush_literal(&mut out, &mut literal);
    out
}

fn flush_literal(out: &mut Vec<u8>, literal: &mut Vec<u8>) {
    if !literal.is_empty() {
        out.push(u8::try_from(256 - literal.len()).unwrap());
        out.append(literal);
    }
}

/// One stored strip: encoding 1 and the packed byte planes of `chunk`.
pub fn strip(chunk: &[u8], element: usize) -> Vec<u8> {
    let count = chunk.len() / element;
    let planes: Vec<u8> = (0..element)
        .flat_map(|p| (0..count).map(move |i| chunk[i * element + p]))
        .collect();
    [&[1, 0, 0, 0][..], &pack_runs(&planes)].concat()
}

/// A planar array body: `data` cut into strips of `strip_bytes` decoded bytes, with `stale`
/// leftover table entries after the terminating zero.
pub fn planar(element: u16, row: u16, data: &[u8], strip_bytes: usize, stale: &[u32]) -> Vec<u8> {
    let strips: Vec<Vec<u8>> = data
        .chunks(strip_bytes)
        .map(|c| strip(c, usize::from(element)))
        .collect();
    planar_from_strips(
        u32::try_from(data.len()).unwrap(),
        element,
        row,
        &strips,
        stale,
    )
}

/// A planar array body from strips already stored.
pub fn planar_from_strips(
    size: u32,
    element: u16,
    row: u16,
    strips: &[Vec<u8>],
    stale: &[u32],
) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(size.to_le_bytes());
    body.extend(element.to_le_bytes());
    body.extend(row.to_le_bytes());
    body.extend([1, 0, 0, 0, 0, 0, 2, 0]);
    body.extend(
        u32::try_from(strips.len() + 1 + stale.len())
            .unwrap()
            .to_le_bytes(),
    );
    for s in strips {
        body.extend(u32::try_from(s.len()).unwrap().to_le_bytes());
    }
    body.extend(0u32.to_le_bytes());
    for s in stale {
        body.extend(s.to_le_bytes());
    }
    for s in strips {
        body.extend(s);
    }
    body
}

/// An image container: plain head fields, a flag byte and the planar array object.
pub fn container(planar_body: &[u8]) -> Vec<u8> {
    object(
        10,
        &[&[0u8; 37][..], &[1], &object(6, planar_body)].concat(),
    )
}

/// Text as the files store it: one byte per character (Latin-1).
pub fn latin1(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap())
        .collect()
}

pub fn name_object(name: &str) -> Vec<u8> {
    object(12, &[&[0u8, 0][..], &latin1(name), &[0]].concat())
}

pub fn rect_object() -> Vec<u8> {
    object(1, &[0u8; 34])
}

/// How the icon and the alpha are stored: name, rectangle, image container.
pub fn picture(name: &str, planar_body: &[u8]) -> Vec<u8> {
    [name_object(name), rect_object(), container(planar_body)].concat()
}

/// The 11 bytes every brush, tool and project file starts with (its signature text).
pub const SIGNATURE: [u8; 11] = [
    0x5A, 0x42, 0x72, 0x75, 0x73, 0x68, 0x20, 0x46, 0x69, 0x6C, 0x65,
];

/// The signature as text.
pub fn signature() -> String {
    String::from_utf8_lossy(&SIGNATURE).into_owned()
}

/// The copyright text the test files carry: the signature, then a note of their own.
pub fn copyright() -> String {
    signature() + ".Copyright test"
}

/// The signature with its last letter wrong, then `more`: no file the readers know.
pub fn near_signature(more: &str) -> Vec<u8> {
    let mut out = SIGNATURE.to_vec();
    out[10] = b'x';
    out.extend(more.as_bytes());
    out
}

pub fn preamble(copyright: &str, format: u32, writer: [u8; 8]) -> Vec<u8> {
    let mut out = copyright.as_bytes().to_vec();
    out.resize(76, 0);
    out.extend(b"PBZ\0");
    out.extend(format.to_le_bytes());
    out.extend(writer);
    out
}

/// A file header with its fixed part sized by version and the credit lines after it.
pub fn header(version: u32, base_type: u32, z: f32, rgb: f32, credits: &[&str]) -> Vec<u8> {
    let fixed = match version {
        1002 => 42,
        1005 => 92,
        1006 => 96,
        1007 => 116,
        _ => 60,
    };
    let mut out = vec![0; fixed];
    out[0..4].copy_from_slice(&version.to_le_bytes());
    out[4..8].copy_from_slice(&base_type.to_le_bytes());
    out[8..12].copy_from_slice(&z.to_le_bytes());
    out[12..16].copy_from_slice(&rgb.to_le_bytes());
    for line in credits {
        out.extend(line.as_bytes());
        out.push(0);
    }
    out
}

/// A curve object: 24 head bytes, the point count, `middle` more head bytes, the points.
pub fn curve(version: u16, middle: usize, points: &[(f32, f32, u32, f32)]) -> Vec<u8> {
    let mut body = vec![7; 24];
    body.extend(u32::try_from(points.len()).unwrap().to_le_bytes());
    body.extend(vec![9; middle]);
    for &(x, y, kind, weight) in points {
        body.extend(x.to_le_bytes());
        body.extend(y.to_le_bytes());
        body.extend(kind.to_le_bytes());
        body.extend(weight.to_le_bytes());
    }
    object(version, &body)
}

/// A setting record. `curve` is a whole curve object.
pub fn setting(kind: u8, flags: u8, value: f32, name: &str, curve: Option<&[u8]>) -> Vec<u8> {
    let mut body = vec![kind, 0x01, flags, 0, 2, 0, 3];
    body.extend(object(1, &value.to_le_bytes()));
    body.extend(object(1, &0.5f32.to_le_bytes()));
    match curve {
        Some(c) => {
            body.push(1);
            body.extend(c);
        }
        None => body.push(0),
    }
    body.push(1);
    body.extend(latin1(name));
    body.push(0);
    object(5, &body)
}

/// The parts of a brush object, written in order by [`brush_object`].
#[derive(Clone)]
pub struct Brush {
    pub version: u16,
    pub draw_size: f32,
    pub z: f32,
    pub rgb: f32,
    pub unknown: f32,
    pub alpha_flags: u32,
    /// A whole picture (name, rectangle, container).
    pub alpha: Option<Vec<u8>>,
    /// Name, mesh body and companion body of each entry.
    pub meshes: Vec<(&'static str, Vec<u8>, Vec<u8>)>,
    /// Everything after the mesh area: plain fields, settings, other objects.
    pub records: Vec<u8>,
}

pub fn brush_object(b: &Brush) -> Vec<u8> {
    let mut body = Vec::new();
    if b.version >= 0x4C {
        body.extend(b.draw_size.to_le_bytes());
    }
    for f in [b.z, b.rgb, b.unknown] {
        body.extend(f.to_le_bytes());
    }
    body.extend(b.alpha_flags.to_le_bytes());
    body.extend(b.alpha.clone().unwrap_or_default());
    body.extend(0u32.to_le_bytes());
    if b.version >= 0x5A {
        body.extend(0u16.to_le_bytes());
    }
    body.push(u8::from(!b.meshes.is_empty()));
    if !b.meshes.is_empty() {
        body.extend(u32::try_from(b.meshes.len()).unwrap().to_le_bytes());
        for (name, mesh, companion) in &b.meshes {
            body.extend(2u32.to_le_bytes());
            body.extend(name_object(name));
            body.extend(rect_object());
            body.extend(object(0x59, mesh));
            body.extend(object(0x13, companion));
        }
    }
    body.extend(&b.records);
    object(b.version, &body)
}

/// A whole file: preamble, icon, header bytes, brush object bytes.
pub fn file(header: &[u8], brush: &[u8]) -> Vec<u8> {
    let icon = picture("Image", &planar(4, 16, &icon_pixels(), 1 << 17, &[]));
    [
        preamble(&copyright(), 1008, *b"4.7\x004.7\x00"),
        icon,
        header.to_vec(),
        brush.to_vec(),
    ]
    .concat()
}

/// A 4 by 4 icon, 4 bytes per pixel, with runs and literals.
pub fn icon_pixels() -> Vec<u8> {
    (0..64u8).map(|i| if i < 32 { 0 } else { i * 3 }).collect()
}

/// A gradient with flat stretches, so strips mix runs and literals.
pub fn alpha_pixels(width: usize, height: usize) -> Vec<u16> {
    (0..width * height)
        .map(|i| {
            if i % width < width / 3 {
                0
            } else {
                u16::try_from(i * 37 % 65536).unwrap()
            }
        })
        .collect()
}

pub fn le16(pixels: &[u16]) -> Vec<u8> {
    pixels.iter().flat_map(|p| p.to_le_bytes()).collect()
}
