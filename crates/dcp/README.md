# cfa4-dcp

Reads the XML documents of a Digital Cinema Package into a serialisable model. This is task DCP-02
of [the DCP validation plan](../../docs/plans/DCP-VALIDATION-2026-10-27.md); the crate layout was
proposed in [DCP-00](../../docs/audit/DCP-00-CAPABILITY-AUDIT-2026-10-27.md).

| Document | SMPTE | Interop |
|---|---|---|
| Asset map, volume index | ST 429-9:2014 (namespace `429-9/2007/AM`) | `PROTO-ASDCP-AM-20040311` |
| Packing list | ST 429-8:2007 (`429-8/2007/PKL`) | `PROTO-ASDCP-PKL-20040311` |
| Composition playlist | ST 429-7:2006 (`429-7/2006/CPL`) | `PROTO-ASDCP-CPL-20040511` |
| Stereoscopic picture asset | ST 429-10:2008 | `437-Y/2007/Main-Stereo-Picture-CPL` |
| Composition metadata asset | ST 429-16:2014 (kept as an extension asset; its fields are listed by name) | n/a |
| Key Delivery Message (`kdm` module) | ST 430-1:2023 (`430-1/2006/KDM`, ETM `430-3/2006/ETM`) | n/a |
| Key Delivery Bundle (`kdb` module) | ST 430-9:2008 + Am1:2011 (ustar, `CATALOG` in `429-10/2008/KDMB`) | n/a |

The root element and its namespace decide the document kind and the standard.

Elements must come in the order of the standard's own schema (an element out of order is `Invalid`).
The two asset map schemas differ here: SMPTE ST 429-9 has `Creator, VolumeCount, IssueDate, Issuer`,
the Interop `PROTO-ASDCP-AM-20040311` schema `VolumeCount, IssueDate, Issuer, Creator` (as libdcp
writes and checks it). Plan: [CFA4-DCPVAL-LIBDCP-001](../../docs/plans/CFA4-DCPVAL-LIBDCP-001-FINAL-1.0.md).

```rust
let doc = cfa4_dcp::read(std::path::Path::new("CPL.xml"), cfa4_dcp::Limits::default())?;
for issue in doc.issues() {
    println!("{}:{} {:?} {} {}", issue.pos.line, issue.pos.column, issue.kind, issue.path, issue.message);
}
```

## Key Delivery Messages

`kdm::open_kdm` decrypts the content keys of a KDM with the recipient's RSA private key
(`kdm::private_key_from_pem`: PEM PKCS#1 or PKCS#8, no password) using RSA-OAEP with SHA-1 (ST
430-3; RustCrypto `rsa` 0.10, constant-time). It checks the Structure ID of every `EncryptedKey`,
that each names the composition of the public part, and the validity window against the time given.
Keys come back in memory only (`KdmKey`'s `Debug` hides them); nothing is stored.

The KDM's XML signature is checked and reported in `Kdm::signature` (#1028, ST 430-1 / 430-2 /
430-3):

- `Valid` (with the chain's common names): the signed content and the signer's chain check out.
- `Untrusted` (chain and reason): the content is intact, but who signed it is not established. The
  chain breaks an ST 430-2 rule, a certificate is outside its validity, an issuer is missing, or
  the encrypted key blocks name another signer certificate (their Certificate Thumbprint, ST 430-1
  Table 6) than the one that signed.
- `Missing`: the KDM is not signed.
- `Invalid` (reason): the signed content does not check out. The KDM was changed after signing,
  a part is not covered, or the signature cannot be verified.

The caller decides what to do. `Signature::is_overridable` is true only for `Untrusted`. Owner
decision 2026-11-13: the engine's `dcp.loadKdm` always refuses `Missing` and `Invalid`, and loads
an `Untrusted` KDM only with `allowUnverified`.

`xmldsig::verify` checks an enveloped XML Signature in two steps. First the signed content:
same-document references (`URI="#id"`, optional enveloped-signature and C14N transforms) with
SHA-256 or SHA-1 digests, inclusive Canonical XML 1.0 with or without comments (`c14n`), and the
RSA PKCS#1 v1.5 (SHA-256 or SHA-1) signature value under the signer certificate's key. A failure
there is an error. Then the certificate chain in `KeyInfo`, whose failure is
`Verified::chain_problem`. Each certificate is checked against ST 430-2 6.2
(`Certificate::st430_2_problem`):

- version 3, and no unknown critical extension
- BasicConstraints, AuthorityKeyIdentifier and KeyUsage present
- CAs with a path length and keyCertSign (cRLSign) only; leaves with digitalSignature or
  keyEncipherment
- exactly one dnQualifier, OrganizationName and CommonName per name, the OrganizationName equal
  to the issuer's
- a role in a leaf's CommonName
- sha256WithRSAEncryption inside and outside
- a 2048-bit RSA key with exponent 65537
- the dnQualifier equal to the public key thumbprint

Along the chain: signed by the next certificate, issuer and subject names chained, within the
issuer's validity and path length, all valid at the given time, ending in a self-signed root
(`x509`: a small DER reader for the fields this needs). For a KDM both `AuthenticatedPublic` and
`AuthenticatedPrivate` must be referenced. Not checked: a list of trusted roots (ST 430-2 step 19)
and revocation lists, which CFA4 does not have.

`kdb::read_bundle` reads a Key Delivery Bundle (ST 430-9) from memory: a POSIX ustar archive
(header checksums checked; regular files with relative paths only, links and `..` paths ignored
with an issue; at most 64 MiB, 4096 files, 4 MiB per KDM) with its `CATALOG` (Id, annotation,
creator, and per `KDMFile` the CPL, `FilePath` under `CONTENT/`, recipient subject and key
validity). Catalog entries without a file become issues; `CONTENT` files the catalog does not
list are still taken. The engine's `dcp.loadKdm` accepts a bundle in place of a KDM and loads every
KDM made for the given private key, each under the signature rule, skipping the other
recipients' (#1144).

`testing` (feature `test-util`) makes ST 430-2 conforming and plain X.509 signing chains with the
openssl and xmlsec1 command lines for tests, plus `ustar` / `catalog` to build bundles.

## What counts as an error

Errors stop reading: the text is not UTF-8, it is not well-formed XML, it is over the
[`Limits`](src/lib.rs) (32 MiB, 2 000 000 nodes by default), it has a DTD, or the root element is
not a DCP document.

Everything else becomes an `Issue` with its element path and line and column, and the field stays
`None`:

- `Missing`: a required element is absent.
- `Invalid`: the value does not parse. A UUID without `urn:uuid:` is reported and still read.
- `Unknown`: the reader does not know the element. It is also listed in the model's `extensions`, so
  a validator can tell "not supported" from "absent".
- `Repeated`: a single element appears more than once; the first one is used.

The reader is lenient on purpose: it serves the validator (DCP-03), which decides what an issue
means for each standard.

## Not here

- Signature verification of CPLs and PKLs. The model records that `Signer` and `ds:Signature` are present.
- Finding files on disk and resolving `Path`. These belong to `cfa4-dcp-validate`, which also runs
  the hash over the files. The digest itself is here: `sha1::Sha1` is a streaming SHA-1 (FIPS 180-4)
  for the packing list `Hash`, checked against the FIPS 180-4 test vectors.
- MXF track files. `cfa4-mxf` reads them.
- Writing DCPs. `cfa4-export` (`dcp_out.rs`) writes them; its CPL layout is one of the test samples.

## Safety

The crate has no `unsafe` code and no panics on input. DTDs (and so entity expansion) are refused.
The tests feed truncated and byte-corrupted documents under `catch_unwind`.
