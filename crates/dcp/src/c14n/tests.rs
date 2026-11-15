//! Canonical XML tests: the W3C recommendation's own examples (section 3), and documents checked
//! against libxml2's canonicalizer through Python lxml (an oracle only, skipped when missing).

use super::*;
use std::process::Command;

fn c14n_of(xml: &str, id: Option<&str>, comments: bool) -> String {
    let doc = roxmltree::Document::parse(xml).unwrap();
    let el = match id {
        Some(id) => doc.descendants().find(|n| n.attribute("Id") == Some(id)).unwrap(),
        None => doc.root_element(),
    };
    String::from_utf8(canonicalize(el, comments, None).unwrap()).unwrap()
}

/// REC-xml-c14n-20010315 3.2 (whitespace in content) and 3.3 (start and end tags), as subsets of
/// the root element (no document-level nodes).
#[test]
fn w3c_examples() {
    let ex32 = "<doc>\n   <clean>   </clean>\n   <dirty>   A   B   </dirty>\n   <mixed>\n      A\n      <clean>   </clean>\n      B\n      <dirty>   A   B   </dirty>\n      C\n   </mixed>\n</doc>";
    assert_eq!(c14n_of(ex32, None, false), ex32);
    let ex33 = r#"<doc>
   <e1   />
   <e2   ></e2>
   <e3   name = "elem3"   id="elem3"   />
   <e4   name="elem4"   id="elem4"   ></e4>
   <e5 a:attr="out" b:attr="sorted" attr2="all" attr="I'm"
      xmlns:b="http://www.ietf.org"
      xmlns:a="http://www.w3.org"
      xmlns="http://example.org"/>
   <e6 xmlns="" xmlns:a="http://www.w3.org">
      <e7 xmlns="http://www.ietf.org">
         <e8 xmlns="" xmlns:a="http://www.w3.org">
            <e9 xmlns="" xmlns:a="http://www.ietf.org"/>
         </e8>
      </e7>
   </e6>
</doc>"#;
    let want33 = r#"<doc>
   <e1></e1>
   <e2></e2>
   <e3 id="elem3" name="elem3"></e3>
   <e4 id="elem4" name="elem4"></e4>
   <e5 xmlns="http://example.org" xmlns:a="http://www.w3.org" xmlns:b="http://www.ietf.org" attr="I'm" attr2="all" b:attr="sorted" a:attr="out"></e5>
   <e6 xmlns:a="http://www.w3.org">
      <e7 xmlns="http://www.ietf.org">
         <e8 xmlns="">
            <e9 xmlns:a="http://www.ietf.org"></e9>
         </e8>
      </e7>
   </e6>
</doc>"#;
    assert_eq!(c14n_of(ex33, None, false), want33);
    // 3.4 character modifications (without the DTD parts roxmltree is not asked to process)
    let ex34 = "<doc>\n   <text>First line&#x0d;&#10;Second line</text>\n   <value>&#x32;</value>\n   <compute><![CDATA[value>\"0\" && value<\"10\" ?\"valid\":\"error\"]]></compute>\n   <compute expr='value>\"0\" &amp;&amp; value&lt;\"10\" ?\"valid\":\"error\"'>valid</compute>\n   <norm attr=' &apos;   &#x20;&#13;&#xa;&#9;   &apos; '/>\n</doc>";
    let want34 = "<doc>\n   <text>First line&#xD;\nSecond line</text>\n   <value>2</value>\n   <compute>value&gt;\"0\" &amp;&amp; value&lt;\"10\" ?\"valid\":\"error\"</compute>\n   <compute expr=\"value>&quot;0&quot; &amp;&amp; value&lt;&quot;10&quot; ?&quot;valid&quot;:&quot;error&quot;\">valid</compute>\n   <norm attr=\" '    &#xD;&#xA;&#x9;   ' \"></norm>\n</doc>";
    assert_eq!(c14n_of(ex34, None, false), want34);
}

/// A subset keeps the namespaces in scope and the xml: attributes of its ancestors; comments only
/// when asked; the enveloped element is left out.
#[test]
fn subsets_inherit_namespaces_and_xml_attributes() {
    let xml = r#"<?xml version="1.0"?>
<Root xmlns="urn:a" xmlns:ds="urn:ds" xml:lang="hu"><!-- c --><Pub Id="p" b="2" a="1"><!-- in --><ds:X>t&amp;</ds:X><?pi  data?></Pub><ds:Signature/></Root>"#;
    assert_eq!(c14n_of(xml, Some("p"), false), r#"<Pub xmlns="urn:a" xmlns:ds="urn:ds" Id="p" a="1" b="2" xml:lang="hu"><ds:X>t&amp;</ds:X><?pi data?></Pub>"#);
    assert_eq!(
        c14n_of(xml, Some("p"), true),
        r#"<Pub xmlns="urn:a" xmlns:ds="urn:ds" Id="p" a="1" b="2" xml:lang="hu"><!-- in --><ds:X>t&amp;</ds:X><?pi data?></Pub>"#
    );
    let doc = roxmltree::Document::parse(xml).unwrap();
    let sig = doc.descendants().find(|n| n.tag_name().name() == "Signature").unwrap();
    let out = String::from_utf8(canonicalize(doc.root_element(), false, Some(sig.id())).unwrap()).unwrap();
    assert!(!out.contains("Signature") && out.starts_with("<Root xmlns=\"urn:a\""), "{out}");
}

/// Deep nesting is an error, not a stack overflow.
#[test]
fn deep_nesting_is_refused() {
    let xml = format!("{}{}", "<a>".repeat(400), "</a>".repeat(400));
    let doc = roxmltree::Document::parse_with_options(&xml, roxmltree::ParsingOptions { nodes_limit: 10_000, ..Default::default() }).unwrap();
    assert!(canonicalize(doc.root_element(), false, None).is_err());
}

/// libxml2 (Python lxml) as the oracle on KDM-shaped documents.
#[test]
fn matches_libxml2() {
    let ok = Command::new("python3").args(["-I", "-c", "import lxml"]).output().is_ok_and(|o| o.status.success());
    if !ok {
        assert!(std::env::var_os("CFA4_REQUIRE_ORACLES").is_none(), "python3 lxml not found");
        eprintln!("SKIPPED: python3 lxml not found");
        return;
    }
    let docs = [
        r#"<DCinemaSecurityMessage xmlns="http://www.smpte-ra.org/schemas/430-3/2006/ETM" xmlns:dsig="http://www.w3.org/2000/09/xmldsig#" xmlns:enc="http://www.w3.org/2001/04/xmlenc#">
  <AuthenticatedPublic Id="ID_AuthenticatedPublic">
    <MessageId>urn:uuid:1</MessageId>
    <!-- a comment -->
    <RequiredExtensions><KDMRequiredExtensions xmlns="http://www.smpte-ra.org/schemas/430-1/2006/KDM"><ContentTitleText language="en">A &lt; B &amp; "C"</ContentTitleText><X xmlns="">plain</X></KDMRequiredExtensions></RequiredExtensions>
    <Signer><dsig:X509IssuerName>dnQualifier=abc\+=,CN=.r</dsig:X509IssuerName></Signer>
  </AuthenticatedPublic>
  <AuthenticatedPrivate Id="ID_AuthenticatedPrivate"><enc:EncryptedKey xmlns:enc="http://www.w3.org/2001/04/xmlenc#"><enc:CipherData><enc:CipherValue>AAEC
AwQ=</enc:CipherValue></enc:CipherData></enc:EncryptedKey></AuthenticatedPrivate>
</DCinemaSecurityMessage>"#,
        "<r xmlns:p=\"urn:p\" xml:space=\"preserve\"><s Id=\"x\" p:b=\"1\" a=\"&#9;tab\" z=\"2\">\r\n<t xmlns:p=\"urn:q\" p:c=\"3\"/>&gt;</s></r>",
    ];
    let dir = std::env::temp_dir().join(format!("cfa4-c14n-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("c14n.py");
    std::fs::write(
        &script,
        "import sys\nfrom lxml import etree\nt = etree.parse(sys.argv[1])\nel = t.getroot() if sys.argv[2] == '-' else t.xpath('//*[@Id=$i]', i=sys.argv[2])[0]\nsys.stdout.buffer.write(etree.tostring(el, method='c14n', with_comments=sys.argv[3] == '1'))\n",
    )
    .unwrap();
    for (k, xml) in docs.iter().enumerate() {
        let f = dir.join(format!("d{k}.xml"));
        std::fs::write(&f, xml).unwrap();
        // whole documents only: lxml canonicalizes a subelement from a copy of it in a new
        // document, which differs from a document subset (the KDM signature tests check subsets
        // against xmlsec1)
        for id in [None::<&str>] {
            for comments in [false, true] {
                let out = Command::new("python3").arg("-I").arg(&script).arg(&f).arg(id.unwrap_or("-")).arg(if comments { "1" } else { "0" }).output().unwrap();
                assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
                assert_eq!(c14n_of(xml, id, comments), String::from_utf8(out.stdout).unwrap(), "doc {k} {id:?} comments {comments}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
