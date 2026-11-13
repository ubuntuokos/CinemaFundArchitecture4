//! AV2 oracle harness self-test (AV2 plan II.3): the pinned AVM v1.0.0 `avmdec` must reproduce
//! the committed manifest hashes on every corpus vector, and the `av2-normative` cross-check
//! build must agree with it.
//!
//! Needs `scripts/dev/av2-oracle.py build` and `corpus`; otherwise every test reports SKIPPED
//! (a failure under `CFA4_REQUIRE_AV2_ORACLE=1`).

use cfa4_testkit::av2::{self, Corpus};

#[test]
fn avmdec_reproduces_the_manifest() {
    let corpus = Corpus::load().unwrap();
    let ctx = "av2_oracle::avmdec_reproduces_the_manifest";
    let Some(avmdec) = av2::avmdec(&corpus).unwrap() else {
        return av2::skip(ctx, "AVM v1.0.0 avmdec not built (scripts/dev/av2-oracle.py build)");
    };
    let mut failures = Vec::new();
    let mut checked = 0;
    for v in &corpus.vectors {
        let Some(obu) = av2::vector_bitstream(v).unwrap() else {
            av2::skip(ctx, &format!("{}: corpus bitstream missing (scripts/dev/av2-oracle.py corpus)", v.name));
            continue;
        };
        match av2::decode_with_avmdec(&avmdec, &obu) {
            Ok((geometry, frames)) => {
                let cmp = av2::compare(v, &geometry, &frames);
                eprintln!("{}: {cmp}", v.name);
                if !cmp.pass() {
                    failures.push(format!("{}: {cmp}", v.name));
                }
            }
            Err(e) => failures.push(format!("{}: {e}", v.name)),
        }
        checked += 1;
    }
    eprintln!("avmdec v1.0.0: {checked}/{} vectors checked", corpus.vectors.len());
    assert!(failures.is_empty(), "oracle differs from the manifest:\n{}", failures.join("\n"));
}

#[test]
fn av2_normative_agrees_with_the_oracle() {
    let corpus = Corpus::load().unwrap();
    let ctx = "av2_oracle::av2_normative_agrees_with_the_oracle";
    let Some(pin) = corpus.cross_check.iter().find(|p| p.reference == "av2-normative") else {
        panic!("the manifest pins no av2-normative cross-check build");
    };
    let Some(avmdec) = av2::cross_check_avmdec(pin).unwrap() else {
        return av2::skip(ctx, "AVM av2-normative avmdec not built (scripts/dev/av2-oracle.py build --ref av2-normative)");
    };
    let mut differences = Vec::new();
    for v in &corpus.vectors {
        let Some(obu) = av2::vector_bitstream(v).unwrap() else {
            av2::skip(ctx, &format!("{}: corpus bitstream missing", v.name));
            continue;
        };
        match av2::decode_with_avmdec(&avmdec, &obu) {
            Ok((geometry, frames)) => {
                let cmp = av2::compare(v, &geometry, &frames);
                eprintln!("{} (av2-normative): {cmp}", v.name);
                if !cmp.pass() {
                    differences.push(format!("{}: {cmp}", v.name));
                }
            }
            Err(e) => differences.push(format!("{}: rejected by av2-normative: {e}", v.name)),
        }
    }
    // Plan II.3: a difference must be documented and the vector leaves the "conformant" set.
    assert!(differences.is_empty(), "av2-normative differs from AVM v1.0.0 (document it, plan II.3):\n{}", differences.join("\n"));
}
