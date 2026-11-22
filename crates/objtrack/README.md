# cfa4-objtrack

Multi-object box tracking for Object Mask (layer L2, [`OBJECT-MASK-PLAN-001`](../../docs/plans/OBJECT-MASK-PLAN-001-2026-11-03.md) OM-A4).

Pure Rust, no `unsafe`, no dependency beyond `cfa4-geom`; it knows nothing of projects, models or time: a
caller feeds the detections (or user boxes) of one frame at a time and gets tracked boxes with stable ids
back. The pieces:

- `iou`: intersection over union of two boxes.
- `hungarian`: optimal assignment on a rectangular cost matrix (Kuhn–Munkres, O(n³)).
- `kalman`: a constant-velocity Kalman filter over a box (centre, area, aspect ratio).
- `sort`: the SORT tracker (Bewley et al., 2016) built from them: predict, match by IoU, update, create
  and retire tracks. Deterministic: the same input always gives the same ids.

The algorithm is written from the paper; no code of the BSD-3 SORT implementation or of libopenshot is
copied. Hostile input (NaN or infinite boxes, empty boxes, huge detection counts) is dropped or bounded,
never a panic.
