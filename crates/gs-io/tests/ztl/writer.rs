//! Builds tool and project files from the layout in `docs/decisions/0005-ztl-format.md`, on top
//! of the brush writer's container pieces: our own bytes, written from the format notes, never
//! copied from a real file.

pub use crate::brush::{icon_pixels, latin1, name_object, object, planar, rect_object, setting};

/// Version of a tool object.
pub const TOOL: u16 = 1004;
/// Plain bytes before the list in every record.
pub const RECORD_TAIL: [u8; 3] = [0, 0, 0];

/// A preamble with the kind tag of a tool (`LTZ\0`) or project (`RPZ\0`) file.
pub fn preamble(tag: &[u8]) -> Vec<u8> {
    let copyright = crate::brush::copyright();
    let mut out = crate::brush::preamble(&copyright, 1008, *b"2021.1\0\0");
    out[76..80].copy_from_slice(tag);
    out
}

/// A whole file: preamble, icon and the tool or project object.
pub fn file(tag: &[u8], main: &[u8]) -> Vec<u8> {
    let icon = crate::brush::picture("Image", &planar(4, 16, &icon_pixels(), 1 << 17, &[]));
    [preamble(tag), icon, main.to_vec()].concat()
}

/// A present array slot: presence byte 1, a 2-byte object and a wrapper around the array.
pub fn array_slot(element: u16, data: &[u8], strip_bytes: usize) -> Vec<u8> {
    array_slot_from(
        element,
        data.len(),
        &planar(element, element, data, strip_bytes, &[]),
    )
}

/// A present array slot around an already written planar array body.
pub fn array_slot_from(element: u16, data_len: usize, planar_body: &[u8]) -> Vec<u8> {
    let count = u32::try_from(data_len / usize::from(element)).unwrap();
    let mut wrapper = 1024u32.to_le_bytes().to_vec();
    wrapper.extend(u32::from(element).to_le_bytes());
    wrapper.push(1);
    wrapper.extend(count.to_le_bytes());
    wrapper.extend(count.to_le_bytes());
    wrapper.push(1);
    wrapper.extend(object(6, planar_body));
    [vec![1], object(13, &[0, 0]), object(4, &wrapper)].concat()
}

/// An absent slot.
pub fn absent() -> Vec<u8> {
    vec![0]
}

/// A mesh object: `head_len` plain bytes, the 260-byte settings object, then the slots and
/// any trailing plain bytes.
pub fn mesh_object(version: u16, head_len: usize, slots: &[Vec<u8>], trailing: &[u8]) -> Vec<u8> {
    let body = [
        vec![0; head_len],
        object(2, &[0; 246]),
        slots.concat(),
        trailing.to_vec(),
    ]
    .concat();
    object(version, &body)
}

/// A companion object: a head ending with the current level and the base vertex count, the
/// slots (the first presence byte right after the head) and the 172-byte tail.
pub fn companion_object(
    version: u16,
    level: u32,
    base_vertices: u32,
    slots: &[Vec<u8>],
    export: Option<([f32; 3], [f32; 3])>,
) -> Vec<u8> {
    let mut head = vec![0u8; 5];
    head.extend(level.to_le_bytes());
    head.extend(base_vertices.to_le_bytes());
    let mut tail = vec![0u8; 172];
    if let Some((scale, offset)) = export {
        for (i, v) in scale.iter().chain(&offset).enumerate() {
            tail[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    object(version, &[head, slots.concat(), tail].concat())
}

/// A record: 25 plain bytes (selected index and count among them), a 2-byte object, a
/// rectangle, the mesh, the companion if any, then `after` (the tail and any list).
pub fn record(
    version: u16,
    selected: u32,
    count: u32,
    mesh: &[u8],
    companion: Option<&[u8]>,
    after: &[u8],
) -> Vec<u8> {
    let mut head = vec![0u8; 4];
    head.extend([0xFF; 4]);
    head.extend([0; 4]);
    head.extend(selected.to_le_bytes());
    head.extend(count.to_le_bytes());
    head.extend([1, 2, 0, 0, 0]);
    let body = [
        head,
        object(13, &[0, 0]),
        rect_object(),
        mesh.to_vec(),
        companion.unwrap_or_default().to_vec(),
        after.to_vec(),
    ]
    .concat();
    object(version, &body)
}

/// The lead byte and type number before a list entry.
pub fn item(lead: u8, kind: u32) -> Vec<u8> {
    [&[lead][..], &kind.to_le_bytes()].concat()
}

/// A list entry: flags, color, name, folder name and `rest` zero bytes.
pub fn entry(flags: u32, color: [u8; 4], name: &str, folder: &str, rest: usize) -> Vec<u8> {
    let mut out = flags.to_le_bytes().to_vec();
    out.extend(color);
    out.extend(latin1(name));
    out.push(0);
    out.extend(latin1(folder));
    out.push(0);
    out.extend(vec![0; rest]);
    out
}

/// A settings object: plain fields, then each record after a presence byte.
pub fn settings_object(head: &[u8], records: &[Vec<u8>]) -> Vec<u8> {
    let mut body = head.to_vec();
    for r in records {
        body.push(1);
        body.extend(r);
    }
    object(10, &body)
}

/// A setting record without a name (name flag 0, nothing after it).
pub fn unnamed_setting(kind: u8, value: f32) -> Vec<u8> {
    let mut body = vec![kind, 1, 0, 0, 0, 0, 0];
    body.extend(object(1, &value.to_le_bytes()));
    body.extend(object(1, &0f32.to_le_bytes()));
    body.extend([0, 0]);
    object(5, &body)
}

/// A tool object: the plain head ending with the selected part's type number, its name,
/// settings and record, and 2 plain bytes.
pub fn tool_object(kind: u32, name: &str, settings: &[u8], record: &[u8]) -> Vec<u8> {
    let mut body = b"ZTool\0\0\0".to_vec();
    body.extend([0; 42]);
    body.extend(kind.to_le_bytes());
    body.extend(name_object(name));
    body.extend(settings);
    body.extend(record);
    body.extend([0, 0]);
    object(TOOL, &body)
}

/// A project object: plain fields ending with the first section's tag, then each section
/// after its tag, and tag 0 at the end.
pub fn project_object(version: u16, sections: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut body = vec![0u8; 16];
    for (tag, section) in sections {
        body.extend(tag.to_le_bytes());
        body.extend(section);
    }
    body.extend(0u32.to_le_bytes());
    object(version, &body)
}

/// The tools section: a flag byte before each tool, then a zero byte and a name list object.
pub fn tools_section(tools: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (flag, tool) in tools {
        body.push(*flag);
        body.extend(tool);
    }
    body.push(0);
    body.extend(object(2, &[0; 8]));
    object(1, &body)
}

/// Little-endian bytes of floats, three per point.
pub fn points(p: &[[f32; 3]]) -> Vec<u8> {
    p.iter().flatten().flat_map(|v| v.to_le_bytes()).collect()
}

/// Little-endian bytes of faces, four indices each.
pub fn faces(f: &[[u32; 4]]) -> Vec<u8> {
    f.iter().flatten().flat_map(|v| v.to_le_bytes()).collect()
}
