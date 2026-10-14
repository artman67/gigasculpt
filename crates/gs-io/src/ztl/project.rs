//! The project object of a project file: plain fields, then sections, each stored as a `u32`
//! tag and an object, ending with tag 0. The tools section (tag 2) holds a flag byte before
//! every tool object and ends with a zero byte and an object listing the tools' names.

use super::{Project, RawSection, Tool, list};
use crate::ReadError;
use crate::container::chunk::{Item, Source, Span, walk};

/// The tag of the section that holds the tools.
const TOOLS: u32 = 2;
/// The version of a tool object.
const TOOL_VERSION: u16 = 1004;

/// The sections of a project and the tools in its tools section.
pub(crate) fn read(src: &mut impl Source, span: Span) -> Result<(Project, Vec<Tool>), ReadError> {
    let mut sections = Vec::new();
    let mut tools = Vec::new();
    let mut tag = None;
    for item in walk(src, span.body_start(), span.body_end())? {
        match item {
            Item::Raw { offset, len } => {
                let bytes = src.bytes(offset, len, "project section tag")?;
                tag = bytes.last_chunk::<4>().map(|w| u32::from_le_bytes(*w));
            }
            Item::Obj(section) => {
                let tag = tag.take().ok_or(ReadError::BadLayout {
                    offset: section.offset,
                    reason: "a project section has a tag before it",
                })?;
                if tag == TOOLS {
                    tools.extend(read_tools(src, section)?);
                }
                sections.push((tag, RawSection::of(section)));
            }
        }
    }
    let project = Project {
        version: span.version,
        sections,
    };
    Ok((project, tools))
}

/// The tools section: a flag byte before every tool object, then a zero byte and a name list.
fn read_tools(src: &mut impl Source, span: Span) -> Result<Vec<Tool>, ReadError> {
    let mut tools = Vec::new();
    let mut flag = None;
    for item in walk(src, span.body_start(), span.body_end())? {
        match item {
            Item::Raw { offset, len } => {
                flag = src.bytes(offset, len, "project tool flag")?.last().copied();
            }
            Item::Obj(obj) if obj.version == TOOL_VERSION => {
                tools.push(list::tool(src, obj, flag.take())?);
            }
            Item::Obj(_) => {}
        }
    }
    Ok(tools)
}
