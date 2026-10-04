//! A strict walk over an XML report.
//!
//! quick-xml processes no DTD: a `<!DOCTYPE>` (istanbul and llvm-cov write one) is skipped, never
//! read, so no entity is ever declared, expanded or fetched. Only the five predefined entities
//! and character references resolve; any other is malformed. The walk fails on anything not well
//! formed, and on a document that ends before its root element does, which is how a file caught
//! half-written shows.

use std::borrow::Cow;

use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::{Loaded, Why, line_at};

/// One step of the walk. `Start` comes for every element, `End` after its content (an empty
/// element gets both).
pub(crate) enum Step<'e> {
    Start {
        name: &'e str,
        attributes: &'e [(String, String)],
    },
    End {
        name: &'e str,
    },
    Text(&'e str),
}

/// Walks `file`, calling `visit` with each step and the names of the elements around it,
/// outermost first (for `Start` and `End`, not including the element itself).
pub(crate) fn walk(
    file: &Loaded,
    text: &str,
    mut visit: impl FnMut(&[String], Step<'_>),
) -> Result<(), Why> {
    let malformed = |offset: u64| Why::Malformed {
        path: file.path.clone(),
        line: Some(line_at(
            &file.bytes,
            usize::try_from(offset).unwrap_or(usize::MAX),
        )),
    };
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<String> = Vec::new();
    let mut root_closed = false;
    let mut pending = String::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|_| malformed(reader.error_position()))?;
        let position = reader.buffer_position();
        let is_markup = matches!(
            event,
            Event::Start(_) | Event::Empty(_) | Event::End(_) | Event::Eof
        );
        if is_markup && !flush(&mut pending, &stack, &mut visit) {
            return Err(malformed(position));
        }
        match event {
            Event::Start(start) | Event::Empty(start) if root_closed => {
                // A second root element.
                let _ = start;
                return Err(malformed(position));
            }
            Event::Start(start) => {
                let (name, attributes) = element(&start).ok_or_else(|| malformed(position))?;
                visit(
                    &stack,
                    Step::Start {
                        name: &name,
                        attributes: &attributes,
                    },
                );
                stack.push(name);
            }
            Event::Empty(start) => {
                let (name, attributes) = element(&start).ok_or_else(|| malformed(position))?;
                visit(
                    &stack,
                    Step::Start {
                        name: &name,
                        attributes: &attributes,
                    },
                );
                visit(&stack, Step::End { name: &name });
                root_closed = stack.is_empty();
            }
            Event::End(_) => {
                // quick-xml has already checked the name against the open element.
                let name = stack.pop().ok_or_else(|| malformed(position))?;
                visit(&stack, Step::End { name: &name });
                root_closed = stack.is_empty();
            }
            Event::Text(text) => pending.push_str(&text.xml_content(XmlVersion::Implicit1_0)),
            Event::CData(data) => pending.push_str(&data.xml_content(XmlVersion::Implicit1_0)),
            Event::GeneralRef(reference) => {
                let resolved = match reference.resolve_char_ref() {
                    Ok(Some(c)) => Cow::Owned(c.to_string()),
                    Ok(None) => Cow::Borrowed(
                        resolve_predefined_entity(&reference.xml_content(XmlVersion::Implicit1_0))
                            .ok_or_else(|| malformed(position))?,
                    ),
                    Err(_) => return Err(malformed(position)),
                };
                pending.push_str(&resolved);
            }
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) | Event::DocType(_) => {}
            Event::Eof => {
                return if root_closed && stack.is_empty() {
                    Ok(())
                } else {
                    Err(malformed(position))
                };
            }
        }
    }
}

/// Hands the text collected since the last tag to `visit`. Outside the root only whitespace may
/// stand; anything else is `false`.
fn flush(
    pending: &mut String,
    stack: &[String],
    visit: &mut impl FnMut(&[String], Step<'_>),
) -> bool {
    if pending.is_empty() {
        return true;
    }
    let ok = if stack.is_empty() {
        pending.trim().is_empty()
    } else {
        visit(stack, Step::Text(pending));
        true
    };
    pending.clear();
    ok
}

/// An element's local name and its attributes, unescaped, or `None` when either is malformed.
fn element(start: &BytesStart<'_>) -> Option<(String, Vec<(String, String)>)> {
    let name = start.local_name().into_inner().to_owned();
    let mut attributes = Vec::new();
    for attribute in start.attributes() {
        let attribute = attribute.ok()?;
        let key = attribute.key.local_name().into_inner().to_owned();
        let value = attribute
            .normalized_value(XmlVersion::Implicit1_0)
            .ok()?
            .into_owned();
        attributes.push((key, value));
    }
    Some((name, attributes))
}

/// The value of attribute `key`.
pub(crate) fn attribute<'a>(attributes: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}
