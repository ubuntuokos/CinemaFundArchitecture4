# cfa4-surface-catalog

Layer L1. The device catalogue and support status of the hardware control surfaces
(plan CFA4-HW-CONTROL-SURFACES-001 FINAL 1.1; integration annex
[`docs/plans/CFA4-HW-CONTROL-SURFACES-001-FINAL-1.1-CFA4-ILLESZTES.md`](../../docs/plans/CFA4-HW-CONTROL-SURFACES-001-FINAL-1.1-CFA4-ILLESZTES.md)).
No device I/O: the event model and the device layer belong to `cfa4-surface-model` / `cfa4-surface`
(CFA4-CONTROL-SURFACE-001).

## Catalogue (`catalog/*.toml`)

One file per family (or per device). A new device needs a new file, no code change. Each model has
an id (`ajazz.akp153e`), a support goal (`FULL`, `BEST_EFFORT`, `MODEL_DEPENDENT`, `INDIRECT`), the
adapter family that drives it, a phase, a system path (`none`, `kernel_hid_input`,
`userspace_uinput`), USB ids with their evidence (`CONFIRMED` only from adapter source or a real
device), controls with the adapter's declaration per feature, the baseline features, and blockers.
A control feature is `<control id>.<feature>` (`keys.press`); the features of each control type are
in `ControlType::features` (plan §3.3).

The catalogue may not declare `VERIFIED`: that comes only from a report (AC-04).

## Reports (`docs/audit/hw/*.yaml`)

`format: cfa4-hw-report`, `version: 1`, `category` (`cfa4-hil`, `cfa4-ci`, `external`), the device
(catalogue model id, USB ids, `serial_hash: "sha256:<64 hex>"`, never a raw serial number,
firmware), the environment (OS, kernel, session, CFA4 version, adapter id and version), one result
per feature (`pass`, `fail`, `skipped`), the tester and the date.

## Status

`status::evaluate` applies plan §3.2 in order: no adapter in this build → `PLANNED` (or
`UNAVAILABLE`), a model-wide blocker → `BLOCKED`, every baseline feature verified and nothing
untested → `VERIFIED`, any feature verified → `PARTIAL`, else `EXPERIMENTAL`. The newest report that
tested a feature decides; `cfa4-ci` reports verify only the model's `protocol_features`, `external`
ones never; a report for another adapter major version asks for re-verification (§4.3).
`status::gate` is the mapping editor's rule (§3.4).

## Tests

`cargo test -p cfa4-surface-catalog`: the shipped catalogue covers every family of plan §2 (AC-01),
the reports in `docs/audit/hw/` are valid (AC-04, AC-14), the §3.2 rules, and bad files as errors.
