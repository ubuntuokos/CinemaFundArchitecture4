# F1 source detachment

This change makes the CFA4 Rust workspace independent of Git fetches from the
upstream libraries during Cargo dependency resolution.

## Changes

- The workspace repository metadata points to
  `ubuntuokos/CinemaFundArchitecture4`.
- `cfa4-docpath` and `cfa4-ui` are local path dependencies under
  `vendor/cfa4-libs/`; their `git+` lockfile sources are removed.
- The pinned upstream source files, tests, licence texts and NOTICE files are
  retained with per-file Git blob provenance manifests.
- A Python standard-library audit checks every workspace manifest, Cargo.lock,
  and the vendored file hashes. Its focused CI job does not install packages or
  compile the media application.

## Scope and remaining work

This removes the two direct Cargo Git fetches. It does not make the complete
application build offline: crates.io dependencies and optional Python/Qt tools
remain separate supply-chain inputs. The optional `CFA4_FONTS_DIR` input is a
local font directory and does not cause a network fetch by itself.

The original attribution and licence notices remain in the vendored crates.
This source migration is not a release or redistribution approval; the release
gate remains on HOLD pending the separate branding and licensing review.
