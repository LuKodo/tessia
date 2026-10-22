//! Minimal, namespace-unaware, read-only XML tree (the Go code used `etree` and
//! `encoding/xml`, which ignore namespace declarations too). The embedded
//! invoice inside the CDATA usually relies on prefixes declared by the outer
//! document, so a namespace-resolving parser would reject it.

use quick_xml::Reader;
use quick_xml::events::Event;

/// An element: local name (text after the first `:`), direct character data
/// (CDATA included) and child elements.
#[derive(Debug, Default)]
pub(crate) struct Element {
    pub name: String,
    pub text: String,
    pub children: Vec<Element>,
}

impl Element {
    /// Direct child with local name `tag`.
    pub fn child(&self, tag: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == tag)
    }

    /// Trimmed text of the direct child `tag`, empty when missing.
    pub fn child_text(&self, tag: &str) -> String {
        self.child(tag)
            .map(|c| c.text.trim().to_owned())
            .unwrap_or_default()
    }

    /// Descendants (not `self`) named `tag`, in document order.
    pub fn descendants<'a>(&'a self, tag: &str) -> Vec<&'a Element> {
        let mut out = Vec::new();
        self.collect(tag, &mut out);
        out
    }

    fn collect<'a>(&'a self, tag: &str, out: &mut Vec<&'a Element>) {
        for c in &self.children {
            if c.name == tag {
                out.push(c);
            }
            c.collect(tag, out);
        }
    }

    /// Text of `<parent>/<tag>` for the first descendant `parent` that has a
    /// child `tag`; `parent == ""` means a direct child of `self`.
    pub fn first_text(&self, parent: &str, tag: &str) -> String {
        if parent.is_empty() {
            return self.child_text(tag);
        }
        for p in self.descendants(parent) {
            if let Some(c) = p.child(tag) {
                return c.text.trim().to_owned();
            }
        }
        String::new()
    }
}

fn local_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    match name.split_once(':') {
        Some((_, local)) => local.to_owned(),
        None => name.into_owned(),
    }
}

fn resolve_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

/// Walks the document, calling `on_root_seen` once a start tag appears and
/// building the tree. Returns the root element (if any).
fn parse(s: &str) -> Result<Option<Element>, String> {
    let mut reader = Reader::from_str(s);
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    loop {
        let ev = reader.read_event().map_err(|e| e.to_string())?;
        match ev {
            Event::Start(e) => stack.push(Element {
                name: local_name(e.name().as_ref()),
                ..Element::default()
            }),
            Event::Empty(e) => {
                let el = Element {
                    name: local_name(e.name().as_ref()),
                    ..Element::default()
                };
                close(&mut stack, &mut root, el);
            }
            Event::End(_) => {
                let el = stack.pop().ok_or_else(|| "unexpected end tag".to_owned())?;
                close(&mut stack, &mut root, el);
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.decode().map_err(|e| e.to_string())?);
                }
            }
            Event::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.decode().map_err(|e| e.to_string())?);
                }
            }
            Event::GeneralRef(r) => {
                let resolved = match r.resolve_char_ref().map_err(|e| e.to_string())? {
                    Some(c) => c,
                    None => {
                        let name = r.decode().map_err(|e| e.to_string())?;
                        resolve_entity(&name).ok_or_else(|| format!("unknown entity &{name};"))?
                    }
                };
                if let Some(top) = stack.last_mut() {
                    top.text.push(resolved);
                }
            }
            Event::Eof => break,
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_) => {}
        }
    }
    if !stack.is_empty() {
        return Err("unexpected EOF: unclosed element".to_owned());
    }
    Ok(root)
}

fn close(stack: &mut [Element], root: &mut Option<Element>, el: Element) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(el),
        None => {
            if root.is_none() {
                *root = Some(el);
            }
        }
    }
}

/// Parses `s` and returns the root element, or `None` for a document without one.
pub(crate) fn parse_root(s: &str) -> Result<Option<Element>, String> {
    parse(s)
}

/// True when `s` is well formed and has a root element.
pub(crate) fn has_root(s: &str) -> Result<bool, String> {
    Ok(parse(s)?.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_tree_ignoring_namespaces() {
        let root = parse_root("<a:r><b:x>1 &amp; <![CDATA[2]]></b:x><y/></a:r>")
            .unwrap()
            .unwrap();
        assert_eq!(root.name, "r");
        assert_eq!(root.child_text("x"), "1 & 2");
        assert!(root.child("y").is_some());
    }

    #[test]
    fn rejects_mismatched_and_unclosed() {
        assert!(parse_root("<a><b></a>").is_err());
        assert!(parse_root("<a><b>").is_err());
        assert!(parse_root("<<<").is_err());
    }
}
