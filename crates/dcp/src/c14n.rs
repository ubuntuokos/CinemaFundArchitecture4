//! Canonical XML 1.0 (W3C REC-xml-c14n-20010315, "inclusive" canonicalization) of an element
//! and its descendants, as XML Signature uses it for the `SignedInfo` and for same-document
//! references (`URI="#id"`).
//!
//! The document comes from `roxmltree`: line breaks and attribute values are already normalised by
//! the parser, and the prefixes are read back from the source text (`roxmltree` keeps only the
//! namespace URIs), so the output uses the document's own qualified names.

use std::collections::BTreeMap;

use roxmltree::{Node, NodeId, NodeType};

/// Namespace of the `xml:` prefix.
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
/// Deepest element nesting canonicalized (a KDM is a few levels deep).
const MAX_DEPTH: usize = 256;

/// The canonical form of `apex` and its descendants (a document subset), without `skip` (and its
/// descendants: the enveloped-signature transform). Comments are kept when `with_comments`.
pub(crate) fn canonicalize(apex: Node<'_, '_>, with_comments: bool, skip: Option<NodeId>) -> Result<Vec<u8>, String> {
    if !apex.is_element() {
        return Err("only an element can be canonicalized".into());
    }
    let mut out = Vec::new();
    let c = C14n { input: apex.document().input_text(), with_comments, skip };
    c.element(apex, &BTreeMap::new(), true, 0, &mut out)?;
    Ok(out)
}

struct C14n<'i> {
    input: &'i str,
    with_comments: bool,
    skip: Option<NodeId>,
}

impl C14n<'_> {
    /// The element's qualified name as written in the source.
    fn qname<'a>(&'a self, n: Node<'_, '_>) -> &'a str {
        let start = n.range().start.saturating_add(1);
        let rest = self.input.get(start..).unwrap_or_default();
        let end = rest.find(|c: char| c.is_ascii_whitespace() || c == '/' || c == '>').unwrap_or(rest.len());
        rest.get(..end).unwrap_or_default()
    }

    fn element(&self, n: Node<'_, '_>, rendered: &BTreeMap<String, String>, apex: bool, depth: usize, out: &mut Vec<u8>) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(format!("elements nested deeper than {MAX_DEPTH}"));
        }
        let qname = self.qname(n);
        if qname.is_empty() {
            return Err("element name not found in the source".into());
        }
        // the namespaces in scope ("" is the default namespace)
        let mut scope: BTreeMap<String, String> = BTreeMap::new();
        for ns in n.namespaces() {
            let prefix = ns.name().unwrap_or_default();
            if prefix == "xml" {
                continue;
            }
            scope.insert(prefix.to_string(), ns.uri().to_string());
        }
        out.push(b'<');
        out.extend_from_slice(qname.as_bytes());
        // namespace nodes not already in scope of the output parent, sorted by prefix (default first)
        if scope.get("").is_none_or(|u| u.is_empty()) && rendered.get("").is_some_and(|u| !u.is_empty()) {
            out.extend_from_slice(b" xmlns=\"\"");
        }
        for (prefix, uri) in &scope {
            if rendered.get(prefix) == Some(uri) || (prefix.is_empty() && uri.is_empty()) {
                continue;
            }
            if prefix.is_empty() {
                out.extend_from_slice(b" xmlns=\"");
            } else {
                out.extend_from_slice(b" xmlns:");
                out.extend_from_slice(prefix.as_bytes());
                out.extend_from_slice(b"=\"");
            }
            attr_escape(uri, out);
            out.push(b'"');
        }
        // attributes sorted by (namespace URI, local name); the apex inherits xml:* attributes
        let mut attrs: Vec<(String, String, String, String)> = n
            .attributes()
            .map(|a| {
                let q = self.input.get(a.range_qname()).unwrap_or(a.name()).to_string();
                (a.namespace().unwrap_or_default().to_string(), a.name().to_string(), q, a.value().to_string())
            })
            .collect();
        if apex {
            for anc in n.ancestors().skip(1).filter(Node::is_element) {
                for a in anc.attributes().filter(|a| a.namespace() == Some(XML_NS)) {
                    if !attrs.iter().any(|x| x.0 == XML_NS && x.1 == a.name()) {
                        attrs.push((XML_NS.to_string(), a.name().to_string(), format!("xml:{}", a.name()), a.value().to_string()));
                    }
                }
            }
        }
        attrs.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        for (_, _, q, v) in &attrs {
            out.push(b' ');
            out.extend_from_slice(q.as_bytes());
            out.extend_from_slice(b"=\"");
            attr_escape(v, out);
            out.push(b'"');
        }
        out.push(b'>');
        for c in n.children() {
            if Some(c.id()) == self.skip {
                continue;
            }
            match c.node_type() {
                NodeType::Element => self.element(c, &scope, false, depth + 1, out)?,
                NodeType::Text => text_escape(c.text().unwrap_or_default(), out),
                NodeType::Comment if self.with_comments => {
                    out.extend_from_slice(b"<!--");
                    out.extend_from_slice(c.text().unwrap_or_default().as_bytes());
                    out.extend_from_slice(b"-->");
                }
                NodeType::PI => {
                    if let Some(pi) = c.pi() {
                        out.extend_from_slice(b"<?");
                        out.extend_from_slice(pi.target.as_bytes());
                        if let Some(v) = pi.value.filter(|v| !v.is_empty()) {
                            out.push(b' ');
                            out.extend_from_slice(v.as_bytes());
                        }
                        out.extend_from_slice(b"?>");
                    }
                }
                _ => {}
            }
        }
        out.extend_from_slice(b"</");
        out.extend_from_slice(qname.as_bytes());
        out.push(b'>');
        Ok(())
    }
}

fn attr_escape(s: &str, out: &mut Vec<u8>) {
    for c in s.chars() {
        match c {
            '&' => out.extend_from_slice(b"&amp;"),
            '<' => out.extend_from_slice(b"&lt;"),
            '"' => out.extend_from_slice(b"&quot;"),
            '\t' => out.extend_from_slice(b"&#x9;"),
            '\n' => out.extend_from_slice(b"&#xA;"),
            '\r' => out.extend_from_slice(b"&#xD;"),
            c => {
                let mut b = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
        }
    }
}

fn text_escape(s: &str, out: &mut Vec<u8>) {
    for c in s.chars() {
        match c {
            '&' => out.extend_from_slice(b"&amp;"),
            '<' => out.extend_from_slice(b"&lt;"),
            '>' => out.extend_from_slice(b"&gt;"),
            '\r' => out.extend_from_slice(b"&#xD;"),
            c => {
                let mut b = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests;
