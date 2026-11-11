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

The root element and its namespace decide the document kind and the standard.

```rust
let doc = cfa4_dcp::read(std::path::Path::new("CPL.xml"), cfa4_dcp::Limits::default())?;
for issue in doc.issues() {
    println!("{}:{} {:?} {} {}", issue.pos.line, issue.pos.column, issue.kind, issue.path, issue.message);
}
```

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

- Signature verification. The model records that `Signer` and `ds:Signature` are present.
- Hashing the assets, finding files on disk, resolving `Path`. These belong to `cfa4-dcp-validate`.
- MXF track files. `cfa4-mxf` reads them.
- Writing DCPs. `cfa4-export` (`dcp_out.rs`) writes them; its CPL layout is one of the test samples.

## Safety

The crate has no `unsafe` code and no panics on input. DTDs (and so entity expansion) are refused.
The tests feed truncated and byte-corrupted documents under `catch_unwind`.
