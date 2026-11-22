//! AMWA AS-02 (MXF Versioning, 1.0, 2011): the bundle manifest (`manifest.xml`, section 9 and the
//! schema of 9.3) and the layout rules of an AS-02 bundle (sections 4.3.3 to 4.3.11).
//!
//! A bundle is a root folder holding the version files (OP1b / OP2b / OP3b, material package only),
//! the manifest and the shim file, a `media/` folder with the essence component files (mono
//! essence OP1a files) and an `extra/` folder for everything else. This module reads and writes
//! the manifest and checks it against those rules; it does not open the MXF files.
//!
//! The AS-02 specification is an AMWA document; no code was copied.

use std::collections::HashSet;
use std::fmt::Write as _;

/// The manifest namespace (section 9.3).
pub const MANIFEST_NS: &str = "http://www.amwa.tv/as-02/1.0/manifest";

/// The most `File` elements a manifest may hold (a bound on hostile input).
const MAX_FILES: usize = 1_000_000;

/// The role of a file (Annex B).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    File,
    Version,
    EssenceComponent,
    Shim,
    Manifest,
    Qc,
    Graphic,
    Caption,
    Folder,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::File => "file",
            Role::Version => "version",
            Role::EssenceComponent => "essencecomponent",
            Role::Shim => "shim",
            Role::Manifest => "manifest",
            Role::Qc => "qc",
            Role::Graphic => "graphic",
            Role::Caption => "caption",
            Role::Folder => "folder",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        Some(match s.trim() {
            "file" => Role::File,
            "version" => Role::Version,
            "essencecomponent" => Role::EssenceComponent,
            "shim" => Role::Shim,
            "manifest" => Role::Manifest,
            "qc" => Role::Qc,
            "graphic" => Role::Graphic,
            "caption" => Role::Caption,
            "folder" => Role::Folder,
            _ => return None,
        })
    }
}

/// A media integrity check (9.2.5): the algorithm, what it covers and the value in hex.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mic {
    /// `hmac-sha1`, `crc32`, `crc16` or `md5`.
    pub kind: String,
    /// `essence_only` or `entire_file`.
    pub scope: String,
    pub value: String,
}

/// One `File` element: a file or folder of the bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// A `urn:uuid:`, `urn:smpte:umid:` or `urn:smpte:ul:` identifier, or empty.
    pub id: String,
    pub role: Role,
    pub size: u64,
    /// Path relative to the bundle root, `/` separated.
    pub path: String,
    pub mic: Option<Mic>,
    pub annotations: Vec<String>,
}

/// The manifest of a bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub bundle_name: String,
    /// `urn:uuid:…`.
    pub bundle_id: String,
    pub creator: String,
    /// `xs:dateTime`.
    pub creation_date: String,
    pub annotations: Vec<String>,
    pub files: Vec<File>,
}

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

/// The file identifier of a file whose own identifier is the UMID `umid` (9.2.1): a UMID that
/// starts with `060a2b34 01010105 01010f20 13000000` gives its last 16 bytes as a UUID URN, any
/// other the SMPTE ST 2029 UMID URN.
pub fn file_id_of_umid(umid: &[u8; 32]) -> String {
    const UUID_UMID: [u8; 16] = [0x06, 0x0A, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x05, 0x01, 0x01, 0x0F, 0x20, 0x13, 0x00, 0x00, 0x00];
    if umid.starts_with(&UUID_UMID) {
        let u = &umid[16..];
        let h: String = u.iter().map(|b| format!("{b:02x}")).collect();
        return format!("urn:uuid:{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]);
    }
    let groups: Vec<String> = umid.chunks(4).map(|c| c.iter().map(|b| format!("{b:02x}")).collect::<String>()).collect();
    format!("urn:smpte:umid:{}", groups.join("."))
}

/// Whether `id` matches the `IdType` of the schema: a UUID, UMID or UL URN, or empty.
pub fn valid_id(id: &str) -> bool {
    let id = id.trim();
    let hex_groups = |s: &str, n: usize| {
        let g: Vec<&str> = s.split('.').collect();
        g.len() == n && g.iter().all(|g| g.len() == 8 && g.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    if id.is_empty() {
        return true;
    }
    if let Some(u) = id.strip_prefix("urn:uuid:") {
        let p: Vec<&str> = u.split('-').collect();
        return p.iter().map(|s| s.len()).eq([8, 4, 4, 4, 12]) && p.iter().all(|s| s.bytes().all(|b| b.is_ascii_hexdigit()));
    }
    if let Some(u) = id.strip_prefix("urn:smpte:umid:") {
        return hex_groups(u, 8);
    }
    if let Some(u) = id.strip_prefix("urn:smpte:ul:") {
        return hex_groups(u, 4);
    }
    false
}

fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name && c.tag_name().namespace() == Some(MANIFEST_NS))
}

impl Manifest {
    /// Read a manifest. Text that is not XML, a wrong root element or namespace and the missing
    /// required elements are errors; the values are checked by [`Manifest::check`].
    pub fn parse(xml: &str) -> Result<Manifest, String> {
        let opt = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 8_000_000, entity_resolver: None };
        let doc = roxmltree::Document::parse_with_options(xml, opt).map_err(|e| format!("manifest: not well-formed XML: {e}"))?;
        let root = doc.root_element();
        if root.tag_name().name() != "Manifest" || root.tag_name().namespace() != Some(MANIFEST_NS) {
            return Err(format!("manifest: the root element must be Manifest in {MANIFEST_NS}"));
        }
        let text = |n: Option<roxmltree::Node<'_, '_>>| n.map(|n| n.text().unwrap_or("").trim().to_string());
        let need = |n: roxmltree::Node<'_, '_>, name: &str| text(child(n, name)).ok_or_else(|| format!("manifest: {name} is missing"));
        let notes = |n: roxmltree::Node<'_, '_>| -> Vec<String> {
            n.children().filter(|c| c.is_element() && c.tag_name().name() == "AnnotationText").map(|c| c.text().unwrap_or("").trim().to_string()).collect()
        };
        let list = child(root, "FileList").ok_or("manifest: FileList is missing")?;
        let mut files = Vec::new();
        for f in list.children().filter(|c| c.is_element() && c.tag_name().name() == "File") {
            if files.len() >= MAX_FILES {
                return Err(format!("manifest: more than {MAX_FILES} files"));
            }
            let role_text = need(f, "Role")?;
            let role = Role::parse(&role_text).ok_or_else(|| format!("manifest: unknown role \"{role_text}\""))?;
            let size_text = need(f, "Size")?;
            let size = size_text.parse::<u64>().map_err(|_| format!("manifest: Size \"{size_text}\" is not a number"))?;
            let mic = child(f, "MIC").map(|m| Mic {
                kind: m.attribute("type").unwrap_or("").to_string(),
                scope: m.attribute("scope").unwrap_or("").to_string(),
                value: m.text().unwrap_or("").trim().to_string(),
            });
            files.push(File { id: need(f, "FileID")?, role, size, path: need(f, "Path")?, mic, annotations: notes(f) });
        }
        Ok(Manifest {
            bundle_name: need(root, "BundleName")?,
            bundle_id: need(root, "BundleID")?,
            creator: need(root, "Creator")?,
            creation_date: need(root, "CreationDate")?,
            annotations: notes(root),
            files,
        })
    }

    /// The manifest as XML (the sample of section 9.4 has this layout).
    pub fn to_xml(&self) -> String {
        let mut o = String::new();
        let _ = writeln!(o, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
        let _ = writeln!(o, "<mft:Manifest xmlns:mft=\"{MANIFEST_NS}\">");
        let _ = writeln!(o, "  <mft:BundleName>{}</mft:BundleName>", escape(&self.bundle_name));
        let _ = writeln!(o, "  <mft:BundleID>{}</mft:BundleID>", escape(&self.bundle_id));
        let _ = writeln!(o, "  <mft:Creator>{}</mft:Creator>", escape(&self.creator));
        let _ = writeln!(o, "  <mft:CreationDate>{}</mft:CreationDate>", escape(&self.creation_date));
        for a in &self.annotations {
            let _ = writeln!(o, "  <mft:AnnotationText>{}</mft:AnnotationText>", escape(a));
        }
        let _ = writeln!(o, "  <mft:FileList>");
        for f in &self.files {
            let _ = writeln!(o, "    <mft:File>");
            let _ = writeln!(o, "      <mft:FileID>{}</mft:FileID>", escape(&f.id));
            let _ = writeln!(o, "      <mft:Role>{}</mft:Role>", f.role.name());
            let _ = writeln!(o, "      <mft:Size>{}</mft:Size>", f.size);
            let _ = writeln!(o, "      <mft:Path>{}</mft:Path>", escape(&f.path));
            if let Some(m) = &f.mic {
                let _ = writeln!(o, "      <mft:MIC type=\"{}\" scope=\"{}\">{}</mft:MIC>", escape(&m.kind), escape(&m.scope), escape(&m.value));
            }
            for a in &f.annotations {
                let _ = writeln!(o, "      <mft:AnnotationText>{}</mft:AnnotationText>", escape(a));
            }
            let _ = writeln!(o, "    </mft:File>");
        }
        let _ = writeln!(o, "  </mft:FileList>");
        let _ = writeln!(o, "</mft:Manifest>");
        o
    }

    /// The version files (role `version`).
    pub fn versions(&self) -> impl Iterator<Item = &File> {
        self.files.iter().filter(|f| f.role == Role::Version)
    }

    /// The essence component files (role `essencecomponent`).
    pub fn essence_components(&self) -> impl Iterator<Item = &File> {
        self.files.iter().filter(|f| f.role == Role::EssenceComponent)
    }

    /// Check the manifest against the specification: identifiers, paths, roles and the bundle
    /// layout of 4.3.7 (version files in the root, essence components in `media/`, a folder entry
    /// for every subfolder, no entry for the manifest itself). Returns every problem found.
    pub fn check(&self) -> Vec<String> {
        let mut p: Vec<String> = Vec::new();
        if self.bundle_name.is_empty() || self.bundle_name.contains(['/', '\\']) {
            p.push("BundleName must be the name of the root folder".into());
        }
        let uuid = self.bundle_id.trim();
        if !(uuid.starts_with("urn:uuid:") && valid_id(uuid)) {
            p.push(format!("BundleID \"{uuid}\" is not a URN-encoded UUID"));
        }
        let mut seen: HashSet<&str> = HashSet::new();
        let mut folders: HashSet<String> = HashSet::new();
        for f in &self.files {
            if f.role == Role::Folder {
                folders.insert(f.path.trim_end_matches('/').to_string());
            }
        }
        for f in &self.files {
            let path = f.path.as_str();
            if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.split('/').any(|s| s == ".." || s == ".") {
                p.push(format!("path \"{path}\" is not relative to the bundle root"));
                continue;
            }
            if !seen.insert(path) {
                p.push(format!("path \"{path}\" is listed twice"));
            }
            if !valid_id(&f.id) {
                p.push(format!("{path}: FileID \"{}\" is not a UUID, UMID or UL URN", f.id));
            }
            if f.role == Role::Manifest || path.eq_ignore_ascii_case("manifest.xml") {
                p.push("the manifest does not list itself".into());
            }
            let in_root = !path.contains('/');
            let is_mxf = path.to_ascii_lowercase().ends_with(".mxf");
            match f.role {
                Role::Version if !(in_root && is_mxf) => p.push(format!("{path}: a version file is an .mxf file in the root folder")),
                Role::EssenceComponent if !(path.starts_with("media/") && is_mxf) => {
                    p.push(format!("{path}: an essence component is an .mxf file under media/"))
                }
                Role::Shim if path != "shim.xml" => p.push(format!("{path}: the shim file is shim.xml in the root folder")),
                Role::Folder if f.size != 0 => p.push(format!("{path}: a folder has size 0")),
                _ => {}
            }
            if let Some(m) = &f.mic {
                if !["hmac-sha1", "crc32", "crc16", "md5"].contains(&m.kind.as_str()) {
                    p.push(format!("{path}: MIC type \"{}\" is not hmac-sha1, crc32, crc16 or md5", m.kind));
                }
                if !["essence_only", "entire_file"].contains(&m.scope.as_str()) {
                    p.push(format!("{path}: MIC scope \"{}\" is not essence_only or entire_file", m.scope));
                }
                if m.value.is_empty() || m.value.len() % 2 != 0 || !m.value.bytes().all(|b| b.is_ascii_hexdigit()) {
                    p.push(format!("{path}: the MIC value is not hex binary"));
                }
            }
            // every subfolder named by a path has a folder entry
            if let Some((dir, _)) = path.rsplit_once('/') {
                let mut acc = String::new();
                for part in dir.split('/') {
                    if !acc.is_empty() {
                        acc.push('/');
                    }
                    acc.push_str(part);
                    if folders.insert(acc.clone()) {
                        p.push(format!("{path}: the folder \"{acc}\" has no entry"));
                    }
                }
            }
        }
        p
    }

    /// Check the simple bundle rules (4.3.8): exactly one version file, named like the root
    /// folder.
    pub fn check_simple(&self) -> Vec<String> {
        let mut p = self.check();
        let versions: Vec<&File> = self.versions().collect();
        if versions.len() != 1 {
            p.push(format!("a simple bundle has one version file, not {}", versions.len()));
        }
        let want = format!("{}.mxf", self.bundle_name);
        if let Some(v) = versions.first()
            && v.path != want
        {
            p.push(format!("the simple version file is called {want}, not {}", v.path));
        }
        p
    }
}

/// The shim namespace (section 10.2).
pub const SHIM_NS: &str = "http://www.amwa.tv/as-02/1.0/shim";

/// The shim file (`shim.xml`, section 10): the name of the shim, optionally its identifier URI
/// and annotations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shim {
    pub name: String,
    /// `ShimID`, an `xs:anyURI`.
    pub id: Option<String>,
    pub annotations: Vec<String>,
}

impl Shim {
    /// The shim file as XML (the sample of section 10.3 has this layout).
    pub fn to_xml(&self) -> String {
        let mut o = String::new();
        let _ = writeln!(o, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
        let _ = writeln!(o, "<Shim xmlns=\"{SHIM_NS}\">");
        let _ = writeln!(o, "  <ShimName>{}</ShimName>", escape(&self.name));
        if let Some(id) = &self.id {
            let _ = writeln!(o, "  <ShimID>{}</ShimID>", escape(id));
        }
        for a in &self.annotations {
            let _ = writeln!(o, "  <AnnotationText>{}</AnnotationText>", escape(a));
        }
        let _ = writeln!(o, "</Shim>");
        o
    }

    /// Read a shim file: a `Shim` element in the shim namespace with a `ShimName`.
    pub fn parse(xml: &str) -> Result<Shim, String> {
        let opt = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 100_000, entity_resolver: None };
        let doc = roxmltree::Document::parse_with_options(xml, opt).map_err(|e| format!("shim: not well-formed XML: {e}"))?;
        let root = doc.root_element();
        if root.tag_name().name() != "Shim" || root.tag_name().namespace() != Some(SHIM_NS) {
            return Err(format!("shim: the root element must be Shim in {SHIM_NS}"));
        }
        let text_of = |name: &str| {
            root.children()
                .filter(|c| c.is_element() && c.tag_name().name() == name && c.tag_name().namespace() == Some(SHIM_NS))
                .map(|c| c.text().unwrap_or("").trim().to_string())
                .collect::<Vec<_>>()
        };
        let name = text_of("ShimName").into_iter().next().ok_or("shim: ShimName is missing")?;
        Ok(Shim { name, id: text_of("ShimID").into_iter().next(), annotations: text_of("AnnotationText") })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: &str, role: Role, path: &str) -> File {
        File { id: id.into(), role, size: if role == Role::Folder { 0 } else { 100 }, path: path.into(), mic: None, annotations: Vec::new() }
    }

    fn bundle() -> Manifest {
        Manifest {
            bundle_name: "show".into(),
            bundle_id: "urn:uuid:D08C1211-5172-4875-96D5-1B731E1D71CD".into(),
            creator: "CFA4".into(),
            creation_date: "2026-11-15T10:00:00Z".into(),
            annotations: vec!["a & b".into()],
            files: vec![
                file("urn:smpte:umid:060a2b34.01010105.01010d11.13000000.f7693d57.31540580.905f0022.5f0bc2f2", Role::Version, "show.mxf"),
                file("urn:uuid:D67509D3-5579-4388-8148-9FF1E08BBB59", Role::Shim, "shim.xml"),
                file("", Role::Folder, "media"),
                file("urn:uuid:ACC8E4E7-5D37-4CE6-889D-E88C3DA4023C", Role::EssenceComponent, "media/v1.mxf"),
            ],
        }
    }

    #[test]
    fn manifest_round_trips() {
        let mut m = bundle();
        m.files[0].mic = Some(Mic { kind: "md5".into(), scope: "entire_file".into(), value: "fc8578b82d4eb729be0212633f81318f".into() });
        m.files[0].annotations = vec!["Single <version>".into()];
        let back = Manifest::parse(&m.to_xml()).unwrap();
        assert_eq!(back, m);
        assert_eq!(m.check(), Vec::<String>::new());
        assert_eq!(m.check_simple(), Vec::<String>::new());
    }

    #[test]
    fn rejects_bad_documents() {
        assert!(Manifest::parse("<a/>").is_err());
        assert!(Manifest::parse("not xml").unwrap_err().contains("not well-formed"));
        let no_list = format!("<m:Manifest xmlns:m=\"{MANIFEST_NS}\"><m:BundleName>x</m:BundleName></m:Manifest>");
        assert!(Manifest::parse(&no_list).is_err());
        let dtd = format!("<!DOCTYPE m [<!ENTITY e \"x\">]><m:Manifest xmlns:m=\"{MANIFEST_NS}\"/>");
        assert!(Manifest::parse(&dtd).is_err());
        let x = bundle().to_xml().replace(">version<", ">weird<");
        assert!(Manifest::parse(&x).unwrap_err().contains("unknown role"));
    }

    #[test]
    fn file_ids_follow_the_umid_rules() {
        let mut u = [0u8; 32];
        u[..16].copy_from_slice(&[0x06, 0x0A, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x05, 0x01, 0x01, 0x0F, 0x20, 0x13, 0, 0, 0]);
        u[16..].copy_from_slice(&[0xD0, 0x8C, 0x12, 0x11, 0x51, 0x72, 0x48, 0x75, 0x96, 0xD5, 0x1B, 0x73, 0x1E, 0x1D, 0x71, 0xCD]);
        assert_eq!(file_id_of_umid(&u), "urn:uuid:d08c1211-5172-4875-96d5-1b731e1d71cd");
        u[12] = 0x0D;
        assert_eq!(file_id_of_umid(&u), "urn:smpte:umid:060a2b34.01010105.01010f20.0d000000.d08c1211.51724875.96d51b73.1e1d71cd");
        assert!(valid_id(&file_id_of_umid(&u)));
    }

    #[test]
    fn layout_problems_are_listed() {
        let mut m = bundle();
        m.files.push(file("urn:uuid:ACC8E4E7-5D37-4CE6-889D-E88C3DA4023D", Role::EssenceComponent, "other/x.mxf"));
        m.files.push(file("bad", Role::File, "../x"));
        m.files.push(file("", Role::Manifest, "manifest.xml"));
        m.files.push(file("", Role::File, "extra/a/q.txt"));
        m.files.push(file("", Role::Version, "show.mxf"));
        m.bundle_id = "d08c".into();
        let e = m.check().join("\n");
        for want in [
            "BundleID",
            "under media/",
            "not relative",
            "does not list itself",
            "folder \"extra\" has no entry",
            "folder \"extra/a\" has no entry",
            "listed twice",
            "other/x.mxf: the folder",
        ] {
            assert!(e.contains(want), "{want} missing in:\n{e}");
        }
        let mut s = bundle();
        s.bundle_name = "other".into();
        assert!(s.check_simple().iter().any(|e| e.contains("is called other.mxf")));
    }

    #[test]
    fn shim_round_trips() {
        let sh =
            Shim { name: "Network <HD> Contribution".into(), id: Some("http://amwa.tv/AS-02-shim/2010/hd-j2k.txt".into()), annotations: vec!["a & b".into()] };
        assert_eq!(Shim::parse(&sh.to_xml()).unwrap(), sh);
        let bare = Shim { name: "x".into(), ..Default::default() };
        assert_eq!(Shim::parse(&bare.to_xml()).unwrap(), bare);
        // the sample of section 10.3
        let sample = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Shim xmlns=\"http://www.amwa.tv/as-02/1.0/shim\"><ShimName>Network HD Contribution</ShimName><ShimID>http://amwa.tv/AS-02-shim/2010/hd-j2k.txt</ShimID><AnnotationText>An example</AnnotationText></Shim>";
        assert_eq!(Shim::parse(sample).unwrap().name, "Network HD Contribution");
        assert!(Shim::parse("<Shim xmlns=\"http://www.amwa.tv/as-02/1.0/shim\"/>").unwrap_err().contains("ShimName"));
        assert!(Shim::parse("<Shim/>").is_err());
        assert!(Shim::parse("<<").is_err());
    }
}
