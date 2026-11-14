//! XML helpers shared by the document readers: limits, positions, child lookup and the issue list.

use roxmltree::{Document, Node, ParsingOptions};

use crate::{Error, Issue, IssueKind, Limits, Pos, Rational, Result, Uuid};

pub(crate) fn parse(text: &str, limits: Limits) -> Result<Document<'_>> {
    let size = u64::try_from(text.len()).unwrap_or(u64::MAX);
    if size > limits.max_bytes {
        return Err(Error::TooLarge { size, limit: limits.max_bytes });
    }
    // No DTD (no entity expansion at all), and a cap on the node count.
    let opt = ParsingOptions { allow_dtd: false, nodes_limit: limits.max_nodes, ..ParsingOptions::default() };
    Document::parse_with_options(text, opt).map_err(|e| Error::Xml(e.to_string()))
}

/// Walks a document, collecting issues with element paths and positions.
pub(crate) struct Reader<'a, 'input> {
    pub doc: &'a Document<'input>,
    pub issues: Vec<crate::Issue>,
}

impl<'a, 'input> Reader<'a, 'input> {
    pub fn new(doc: &'a Document<'input>) -> Self {
        Reader { doc, issues: Vec::new() }
    }

    pub fn pos(&self, node: Node<'_, '_>) -> Pos {
        let p = self.doc.text_pos_at(node.range().start);
        Pos { line: p.row, column: p.col }
    }

    pub fn issue(&mut self, kind: IssueKind, path: &str, node: Node<'_, '_>, message: impl Into<String>) {
        let pos = self.pos(node);
        self.issues.push(Issue { kind, path: path.to_string(), pos, message: message.into() });
    }

    /// The element children of `node` with this local name in the document's namespace (the root
    /// element's). An element of the same name in another namespace is not a schema element; the
    /// reader reports it as unknown.
    pub fn children<'n>(node: Node<'n, 'input>, name: &'n str) -> impl Iterator<Item = Node<'n, 'input>> + 'n {
        node.children().filter(move |c| c.is_element() && c.tag_name().name() == name && in_document_ns(*c))
    }

    /// The single child `name`; a repeat is an issue and the first one wins.
    pub fn child(&mut self, node: Node<'a, 'input>, path: &str, name: &str) -> Option<Node<'a, 'input>> {
        let mut it = node.children().filter(|c| c.is_element() && c.tag_name().name() == name && in_document_ns(*c));
        let first = it.next()?;
        if let Some(second) = it.next() {
            self.issue(IssueKind::Repeated, &format!("{path}/{name}"), second, format!("{name} appears more than once; the first is used"));
        }
        Some(first)
    }

    /// The single child `name` and its trimmed text; with `required`, an absent element is a
    /// `Missing` issue.
    pub fn value(&mut self, node: Node<'a, 'input>, path: &str, name: &str, required: bool) -> Option<(String, Node<'a, 'input>)> {
        match self.child(node, path, name) {
            Some(c) => {
                // a simple-typed value holds text only; markup inside it is not part of the value
                if let Some(e) = c.children().find(|n| n.is_element()) {
                    let msg = format!("{name} must hold text only, found element <{}>", e.tag_name().name());
                    self.issue(IssueKind::Invalid, &format!("{path}/{name}"), e, msg);
                }
                Some((c.text().unwrap_or("").trim().to_string(), c))
            }
            None => {
                if required {
                    self.issue(IssueKind::Missing, &format!("{path}/{name}"), node, format!("{name} is missing"));
                }
                None
            }
        }
    }

    /// Trimmed text of the single child `name`.
    pub fn text(&mut self, node: Node<'a, 'input>, path: &str, name: &str) -> Option<String> {
        self.value(node, path, name, false).map(|(t, _)| t)
    }

    /// Like [`Reader::text`], but an absent element is a `Missing` issue.
    pub fn required_text(&mut self, node: Node<'a, 'input>, path: &str, name: &str) -> Option<String> {
        self.value(node, path, name, true).map(|(t, _)| t)
    }

    /// Parses the text of the single child `name`; text that does not parse is an `Invalid` issue
    /// with `what` describing the expected form.
    fn parsed<T>(&mut self, node: Node<'a, 'input>, path: &str, name: &str, required: bool, what: &str, f: impl FnOnce(&str) -> Option<T>) -> Option<T> {
        let (t, c) = self.value(node, path, name, required)?;
        let v = f(&t);
        if v.is_none() {
            self.issue(IssueKind::Invalid, &format!("{path}/{name}"), c, format!("{name} '{t}' is not {what}"));
        }
        v
    }

    /// A `urn:uuid:`. A bare UUID is an `Invalid` issue but its value is kept.
    pub fn uuid(&mut self, node: Node<'a, 'input>, path: &str, name: &str, required: bool) -> Option<Uuid> {
        let (t, c) = self.value(node, path, name, required)?;
        if let Some(u) = Uuid::parse(&t) {
            return Some(u);
        }
        let lenient = Uuid::parse_lenient(&t);
        let msg = if lenient.is_some() { format!("{name} '{t}' lacks the urn:uuid: prefix") } else { format!("{name} '{t}' is not a urn:uuid") };
        self.issue(IssueKind::Invalid, &format!("{path}/{name}"), c, msg);
        lenient
    }

    pub fn u64(&mut self, node: Node<'a, 'input>, path: &str, name: &str, required: bool) -> Option<u64> {
        self.parsed(node, path, name, required, "a non-negative integer", |t| t.parse::<u64>().ok())
    }

    /// A strictly positive integer (`xs:positiveInteger`).
    pub fn positive(&mut self, node: Node<'a, 'input>, path: &str, name: &str, required: bool) -> Option<u64> {
        self.parsed(node, path, name, required, "a positive integer", |t| t.parse::<u64>().ok().filter(|v| *v > 0))
    }

    /// An `xs:dateTime` (`2026-11-09T21:00:00Z`, `…+01:00`, or without a zone), kept as written.
    /// The calendar check (month length, leap years) is `chrono`'s.
    pub fn date_time(&mut self, node: Node<'a, 'input>, path: &str, name: &str, required: bool) -> Option<String> {
        self.parsed(node, path, name, required, "an xs:dateTime", |t| is_date_time(t).then(|| t.to_string()))
    }

    pub fn rational(&mut self, node: Node<'a, 'input>, path: &str, name: &str, required: bool) -> Option<Rational> {
        self.parsed(node, path, name, required, "two integers with a positive denominator", Rational::parse)
    }

    /// A base64 SHA-1 `Hash`: the text as written and the digest when it decodes.
    pub fn hash(&mut self, node: Node<'a, 'input>, path: &str, name: &str, required: bool) -> (Option<String>, Option<[u8; 20]>) {
        let Some((t, c)) = self.value(node, path, name, required) else { return (None, None) };
        let d = crate::decode_sha1(&t);
        if d.is_none() {
            self.issue(IssueKind::Invalid, &format!("{path}/{name}"), c, format!("{name} '{t}' is not base64 of a 20-byte SHA-1 digest"));
        }
        (Some(t), d)
    }

    /// Reports every element child of `node` whose local name is not in `known` as `Unknown`, and
    /// returns their `{namespace}name` for the model. `known` is in schema order (the documents are
    /// `xs:sequence`s), so a known element after one that comes later in `known` is `Invalid`.
    pub fn unknown(&mut self, node: Node<'a, 'input>, path: &str, known: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        let mut last: Option<usize> = None;
        let mut misordered = false;
        for c in node.children().filter(|c| c.is_element()) {
            let name = c.tag_name();
            let signature = name.name() == "Signature" && name.namespace() == Some(crate::ns::XMLDSIG);
            let index = if signature {
                known.iter().position(|k| *k == "Signature")
            } else {
                known.iter().position(|k| *k == name.name()).filter(|_| in_document_ns(c))
            };
            if let Some(i) = index {
                // the first element out of order is reported; the ones after it would only repeat it
                if let Some(l) = last.filter(|l| i < *l && !misordered) {
                    misordered = true;
                    let after = known.get(l).copied().unwrap_or("");
                    self.issue(
                        IssueKind::Invalid,
                        &format!("{path}/{}", name.name()),
                        c,
                        format!("{} is out of schema order: it must come before {after}", name.name()),
                    );
                }
                last = Some(last.map_or(i, |l| l.max(i)));
                continue;
            }
            if signature {
                continue;
            }
            let full = qualified(c);
            self.issue(IssueKind::Unknown, &format!("{path}/{}", name.name()), c, format!("element {full} is not read"));
            out.push(full);
        }
        out
    }
}

/// `xs:dateTime`: date `T` time, optional fraction, optional `Z` or `±hh:mm`.
pub(crate) fn is_date_time(t: &str) -> bool {
    if t.contains([' ', 't', 'z']) {
        return false;
    }
    chrono::DateTime::parse_from_rfc3339(t).is_ok() || chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S%.f").is_ok()
}

/// The element is in the namespace of its document's root element.
pub(crate) fn in_document_ns(node: Node<'_, '_>) -> bool {
    node.tag_name().namespace() == node.document().root_element().tag_name().namespace()
}

/// `ds:Signature` (XML-DSig) is a child of `node`.
pub(crate) fn has_signature(node: Node<'_, '_>) -> bool {
    node.children().any(|c| c.is_element() && c.tag_name().name() == "Signature" && c.tag_name().namespace() == Some(crate::ns::XMLDSIG))
}

/// `{namespace}name`, or `name` without a namespace.
pub(crate) fn qualified(node: Node<'_, '_>) -> String {
    let n = node.tag_name();
    match n.namespace() {
        Some(ns) => format!("{{{ns}}}{}", n.name()),
        None => n.name().to_string(),
    }
}
