# CFA4-AI-MATTING-ENGINE-001: Unified AI Matting Engine — BMV2 + RVM

> **Átvitel a repóba** (`docs/coordination.md` §2.11, 2026-11-23): a terv szövege István beszélgetésbeli üzenetéből került ide, **tartalmilag változatlanul**; csak Markdown-formázás (címsorok) és egyetlen szó változott: a szülőprojekt neve a márkaszűrő miatt (`python/tests/test_independence.py`) „upstream”-re cserélődött (lásd 2. pont, utolsó előtti tétel). Az illesztés a többi tervhez: `CFA4-GREENSCREEN-MATTING-INTEGRATION-001-0.1.md` (PR #1783, még nem beolvasztva). A repóbeli fájl ettől kezdve az irányadó.

Version: 1.0
Status: DEVELOPMENT PLAN — APPROVAL BASELINE
Project: CinemaFundArchitecture4 (CFA4)
Target: Linux / Ubuntu 26.04 / amd64 only

## 1. OBJECTIVE

Implement a unified, CFA4-native AI Matting Engine by integrating two complementary matting backends:

1. BackgroundMattingV2 (BMV2)
2. RobustVideoMatting (RVM)

The goal is not to integrate two standalone applications. The goal is to implement one common CFA4 AI Matting subsystem with two specialized inference backends, one shared result-processing pipeline, one UI workflow, and native integration with Timeline, Compositor, VFX, background replacement, and CFA4 export.

The implementation must reuse existing CFA4 plans, components, services, infrastructure, and architecture wherever possible. Do not recreate existing subsystems.

## 2. PROJECT CONSTRAINTS

* Linux-only.
* Ubuntu 26.04 and compatible variants.
* amd64 only.
* Qt6 GUI.
* CFA4-native implementation and integration.
* Local-first and offline capable.
* No cloud inference dependency.
* No automatic AI model download or installation.
* AI models remain user-controlled.
* Integrate with the existing AI Model Manager.
* Integrate with the existing Adaptive Capability System.
* Integrate with the existing AI Video Tools architecture.
* Reuse existing Timeline, Compositor, VFX, media, rendering, caching, and export systems.
* Do not introduce a second database engine.
* Do not introduce a separate AI Model Manager.
* Do not create duplicate Compositor or VFX systems.
* Do not modify the upstream parent/source project.
* Preserve CFA4 independence from upstream applications.
* Ensure upstream applications can coexist with CFA4 without package, path, MIME, configuration, cache, or other resource conflicts.
* Do not claim implementation or validation completion without verifiable evidence.

## 3. BACKEND RESPONSIBILITIES

### 3.1 BackgroundMattingV2 (BMV2)

Purpose: reference-background-based, high-resolution foreground matting.

Capabilities:

* Reference background input.
* Reference image management.
* Reference validation.
* Foreground RGB generation.
* Alpha matte generation.
* High-resolution refinement where supported.
* Image and video processing.
* Preview workflow.
* Matte output.
* Foreground output.
* Integration with CFA4 matte processing and post-processing.

Typical use:

* Controlled shooting.
* Static camera.
* Captured clean background plate.
* Green-screen-free background removal.
* Detailed foreground extraction where a reference image is available.

The BMV2 backend must not require CFA4 to reproduce the upstream standalone application or scripts.

### 3.2 RobustVideoMatting (RVM)

Purpose: temporal human video matting without requiring a reference background image.

Capabilities:

* Human foreground extraction.
* Video matting.
* Temporal guidance.
* Recurrent state processing.
* Foreground RGB output.
* Alpha matte output.
* Adjustable downsample ratio.
* Preview processing.
* Image processing where supported by the backend.
* Sequential frame processing with correct recurrent state handling.

Model variants:

* MobileNetV3: primary lower-resource model.
* ResNet50: larger model for quality-oriented processing where hardware capability and validation justify it.

RVM recurrent state requirements:

* Process frames in correct sequential order.
* Maintain state only within the correct clip and processing job.
* Never share recurrent state across clips or unrelated jobs.
* Reset states when processing context changes.
* Invalidate state after cancellation or corruption.
* Handle seek using state reinitialization or validated recovery.
* Never reuse stale state.
* Ensure preview processing does not corrupt full-render state.
* Ensure range changes correctly reset or recover state.

## 4. UNIFIED ARCHITECTURE

Implement:

```
AI Video Tools
|
AI Matting Engine Controller
|
Backend Registry / Selector
|
Unified Matting Backend Interface
|
+-- BMV2 Backend Adapter
|
+-- RVM Backend Adapter
|
Unified Matte Processing Pipeline
|
+-- Foreground RGB
+-- Alpha Matte
+-- Preview
+-- Matte Refinement
+-- Cache
+-- Validation
+-- Export
|
CFA4 Integration
+-- Timeline
+-- Compositor
+-- VFX
+-- Background Replacement
+-- Render / Export
```

All components must be CFA4-native integration layers.

## 5. UNIFIED BACKEND INTERFACE

Define a common backend contract.

Input

* Source image/video.
* Source asset identifier.
* Processing range.
* Resolution.
* Frame rate and timestamps.
* Processing profile.
* Model identifier/version.
* Backend parameters.
* Hardware capability information.
* Output configuration.
* Optional background reference.
* Optional refinement parameters.

Output

* Foreground RGB.
* Alpha matte.
* Preview data.
* Composite preview.
* Frame timestamps.
* Resolution.
* Frame count.
* Backend identifier.
* Model identifier/version.
* Processing parameters.
* Processing metadata.
* Warnings.
* Error/status information.

Required operations

* Query backend capabilities.
* Query available models.
* Validate input.
* Validate model availability.
* Create job.
* Start job.
* Cancel job.
* Retrieve progress.
* Retrieve preview.
* Retrieve result.
* Validate result.
* Release resources.
* Clean up state and temporary files.

Backend-specific controls must remain backend-specific. The common interface must not invent parameters unsupported by a backend.

## 6. UNIFIED MATTE PROCESSING PIPELINE

Implement one shared processing pipeline for both backends.

Responsibilities:

* Foreground RGB handling.
* Alpha matte handling.
* Output validation.
* Resolution validation.
* Frame count and timestamp validation.
* Matte preview.
* Transparent preview.
* Composite preview.
* Matte post-processing.
* Optional refinement.
* Matte asset creation.
* Cache management.
* Export preparation.
* Timeline asset preparation.
* Compositor/VFX integration.

Supported result types:

* Foreground.
* Alpha matte.
* Foreground + alpha.
* Matte-only asset.
* Transparent foreground.
* Composite output.

Maintain consistent output contracts regardless of backend.

## 7. BMV2 WORKFLOW

Implement:

* Source clip/image selection.
* Background reference selection/import.
* Reference preview.
* Reference validation.
* Resolution and aspect ratio checks.
* Reference mismatch warnings.
* Backend/model selection.
* Preview processing.
* Matte inspection.
* Refinement where supported.
* Full processing.
* Result validation.
* Foreground/alpha output.
* Timeline integration.

Reference validation should detect or warn about relevant mismatches where technically possible:

* Resolution.
* Aspect ratio.
* Background differences.
* Lighting differences.
* Camera/perspective changes.
* Invalid or missing reference.
* Unsupported reference format.

Do not claim perfect camera, lighting, or scene-change detection unless actually implemented and tested.

BMV2 refinement must expose only validated backend controls. Additional CFA4 matte corrections may be implemented in the shared post-processing layer rather than attributed to BMV2.

## 8. RVM WORKFLOW

Implement:

* Source selection.
* Model selection.
* Processing profile.
* Downsample ratio control.
* Preview.
* Sequential inference.
* Recurrent state management.
* State reset.
* Seek/recovery handling.
* Range change handling.
* Cancel and cleanup.
* Full processing.
* Result validation.
* Foreground/alpha output.
* Timeline integration.

Recurrent State Manager

Must support:

* State initialization.
* State lifecycle ownership.
* State forwarding between sequential frames.
* Clip isolation.
* Job isolation.
* State reset.
* State invalidation.
* State release.
* Cancellation cleanup.
* Stale-state detection.
* Validated seek recovery.

Frame inference must remain sequential wherever recurrent state dependency requires it. Decode, encoding, output writing, and other safe pipeline operations may be optimized independently without violating recurrent inference ordering.

Downsample Ratio

* Hardware-aware defaults.
* Resolution-aware defaults.
* Content-sensitive recommendations where possible.
* User override.
* Preview validation.
* Memory-aware selection.
* Quality/speed tradeoff.
* No assumption that a higher ratio always produces a better result.

## 9. AI MATTING UI — QT6

Add a dedicated AI Matting workspace inside the existing AI Video Tools architecture.

UI elements:

* Source selector.
* Processing range.
* Backend selector.
* Auto backend selection.
* Model selector.
* Model availability.
* Processing profile.
* Resolution.
* Downsample ratio for RVM.
* Background reference for BMV2.
* Preview mode.
* Foreground preview.
* Alpha preview.
* Transparent preview.
* Composite preview.
* Matte inspection.
* Supported refinement controls.
* Processing progress.
* Cancel control.
* Resource status.
* Result output selection.
* Export.
* Timeline handoff.
* Compositor/VFX handoff.

Preview modes:

* Foreground RGB.
* Alpha matte.
* Transparent foreground.
* Composite with selected background.
* Edge/detail inspection where supported.

Statuses:

* Model missing.
* Download required.
* Backend unavailable.
* Reference required.
* Reference validation warning.
* Hardware warning.
* Preview ready.
* Processing.
* Cancelled.
* Failed.
* Result ready.
* Export ready.

Auto mode may recommend or select a backend based on task conditions, but it must not automatically download models or silently change workflow requirements.

## 10. ADAPTIVE CAPABILITY SYSTEM

Reuse the existing CFA4 Adaptive Capability System.

Assess:

* CPU capability.
* Available CPU threads.
* GPU availability.
* Supported inference providers.
* RAM.
* VRAM.
* Model memory requirements.
* Resolution.
* Frame buffers.
* RVM recurrent state memory.
* Preview memory.
* Cache memory.
* Decode/encode load.
* Other concurrent jobs.

Profiles:

* Draft: Lower-resource preview and rapid validation.
* Balanced: Balanced quality, resource consumption, and processing time.
* Quality: Quality-oriented model/processing settings where hardware and backend permit.
* Custom: User-defined validated backend parameters.

Rules:

* Never exceed available hardware resources.
* Do not force GPU memory overcommit.
* CPU fallback only if the backend/runtime supports it.
* Slower CPU execution is acceptable.
* Reduce resolution/profile or ask the user when resource limits are reached.
* Account for RVM recurrent state memory.
* Scheduler must manage concurrent jobs.
* Preview and full processing must not compete without resource control.
* Do not promise upstream benchmark speeds as CFA4 performance.

## 11. MODEL MANAGER INTEGRATION

Reuse CFA4 AI Model Manager.

Model entries must include:

* Model name.
* Backend.
* Version.
* Model format.
* Source link.
* Download size, if known.
* Installed storage requirement, if known.
* Checksum, if supplied.
* License.
* Runtime requirements.
* Hardware compatibility.
* Installation status.
* Validation status.
* Model removal option.

Required model entries:

* BMV2 supported models.
* RVM MobileNetV3 models.
* RVM ResNet50 models, where approved.

Download rules:

1. Show model details.
2. Show download source.
3. Show estimated storage.
4. Validate compatibility.
5. Require explicit user download action.
6. Validate downloaded model.
7. Register it with Model Manager.

Forbidden:

* Automatic download.
* Automatic first-run model installation.
* Silent model updates.
* Automatic model replacement.
* Bundling large model files without explicit project approval.

## 12. TIMELINE INTEGRATION

Matting output must become reusable Timeline assets.

Implement:

* Matting on selected clip.
* Processing range.
* Preview.
* Matte asset.
* Foreground asset.
* Foreground layer creation.
* Alpha association.
* Background layer insertion.
* Non-destructive workflow.
* Original source preservation.
* Reprocessing.
* Version management.
* Parameter provenance.
* Asset relinking.

Output options:

* Original clip + matte.
* Foreground + alpha.
* Matte-only asset.
* Transparent foreground.
* Composite result.
* Timeline-ready asset.

## 13. COMPOSITOR INTEGRATION

Matting outputs must be native Compositor inputs.

Support:

* Foreground RGB.
* Alpha.
* Matte mask.
* Background input.
* Composite.
* Mask-driven blending.
* Matte-driven effects.
* Alpha correction.
* Reusable matte assets.

Workflow:

Source → AI Matting → Foreground + Alpha → Background → Composite

Matting backends must not be implemented independently inside Compositor.

## 14. VFX INTEGRATION

Matte assets must be reusable inside the existing CFA4 VFX system.

Support:

* Subject isolation.
* Masking.
* Background replacement.
* Foreground effects.
* Edge-based effects.
* Alpha compositing.
* Matte-driven blur.
* Matte-driven color correction.
* Matte-driven glow.
* Matte-driven lighting effects.
* VFX node inputs.
* Reusable matte assets.

VFX must consume the Unified Matting Engine outputs instead of maintaining separate BMV2/RVM implementations.

## 15. BACKGROUND REPLACEMENT

Implement a non-destructive workflow:

1. Select source clip.
2. Select Auto, BMV2, or RVM.
3. Preview.
4. Validate foreground and alpha.
5. Refine matte.
6. Select replacement background.
7. Composite.
8. Perform color/light integration using existing CFA4 tools.
9. Preview render.
10. Export.

Background inputs:

* Video.
* Image.
* Another Timeline clip.
* Compositor-generated background.
* VFX background.
* CFA4 project background.

## 16. CACHE AND PROJECT DATA

Cache identity must include:

* Source asset ID/hash.
* Source range.
* Backend.
* Backend version.
* Model.
* Model version/checksum.
* Resolution.
* Processing profile.
* Backend parameters.
* BMV2 reference image ID/hash.
* RVM downsample ratio.
* Refinement parameters.
* Output format.

Cache requirements:

* Backend separation.
* Model version invalidation.
* Reference image invalidation for BMV2.
* Preview/full output separation.
* Controlled storage usage.
* User-controlled cache cleanup.
* No cross-job recurrent state reuse.
* Recurrent states are not treated as generic output cache.

Persist project metadata:

* Job.
* Backend.
* Model.
* Model version.
* Processing parameters.
* Processing range.
* Background reference.
* Matte.
* Foreground.
* Preview settings.
* Job status.
* Warnings.
* Timeline association.
* Compositor/VFX association.
* Cache references.

Projects must reopen even if a model/backend is missing. Missing dependencies must be reported; no automatic download.

## 17. JOB SCHEDULER

Implement:

* Job queue.
* Priority.
* Preview jobs.
* Full processing jobs.
* Cancel.
* Progress.
* Hardware resource reservation.
* GPU memory guard.
* CPU thread management.
* Concurrent job control.
* Preview/full arbitration.
* Job cleanup.
* Stale job detection.
* Output validation.
* Temporary file cleanup.
* Recurrent state cleanup.
* Failure recovery.

RVM sequential inference ordering must be preserved.

## 18. EXPORT AND RENDER

Use CFA4's existing rendering and export pipeline.

Supported output targets:

* Alpha matte image sequence.
* Foreground image sequence.
* Foreground with alpha.
* Transparent video, only where codec/container support is validated.
* Composite video.
* Timeline-ready intermediate asset.
* Matte-only output.

Validate:

* Alpha preservation.
* Frame count.
* Resolution.
* Frame order.
* Timestamp integrity.
* Color format.
* Alpha format.
* Codec/container capability.
* VFR timestamp preservation.
* Output integrity.

Never advertise alpha export support for a codec/container combination that loses or cannot represent alpha.

## 19. LICENSING GATE

### BMV2

Audit:

* MIT license.
* Attribution requirements.
* Model/weight license.
* Dependencies.
* Dataset terms where applicable.
* Third-party components.
* Distribution compatibility.

### RVM

Audit:

* GPL-3.0 obligations.
* Direct integration implications.
* Modified source obligations.
* Binary distribution.
* Model/weight license.
* Dependencies.
* Exported model formats.
* Distribution implications.
* Plugin/process architecture implications.
* CFA4 packaging and license compatibility.

A separate process/plugin boundary must not be assumed to automatically remove GPL obligations.

F0 must produce a formal licensing decision:

* LICENSE_APPROVED
* LICENSE_APPROVED_WITH_CONDITIONS
* LICENSE_BLOCKED
* LICENSE_REVIEW_REQUIRED

RVM implementation is blocked until licensing compatibility is approved.

If RVM is blocked, BMV2 development may continue independently. Do not silently replace RVM with an unapproved alternative.

## 20. ERROR HANDLING

Handle:

* Missing backend.
* Missing runtime.
* Missing model.
* Model checksum failure.
* Model load failure.
* Unsupported inference provider.
* GPU failure.
* VRAM allocation failure.
* RAM exhaustion.
* Invalid resolution.
* Unsupported format.
* Missing/invalid BMV2 reference.
* RVM state corruption.
* Incorrect frame ordering.
* Decode failure.
* Alpha output failure.
* Foreground output failure.
* Output write failure.
* Cancelled job.
* Stale state.
* Unsupported parameter.
* License/dependency validation failure.

Error UI must:

* Identify the failure.
* Identify backend/model where relevant.
* State whether the job can continue.
* Offer only valid supported recovery actions.
* Provide technical diagnostics.
* Avoid silent backend switching.
* Avoid silently losing source data or matte results.

## 21. DEVELOPMENT PHASES

### F0 — Audit and License Gate

* Audit existing CFA4 plans and implementation.
* Review AI Video Tools, Model Manager, Adaptive Capability System, Timeline, Compositor, VFX, cache, export.
* Identify already existing capabilities.
* Produce a gap-only implementation scope.
* Audit BMV2 and RVM source, model formats, runtimes, dependencies, and licenses.
* Complete RVM GPL-3.0 legal gate.
* Output: audit report, gap list, license decision.

### F1 — Unified Architecture

* Controller.
* Backend Registry.
* Unified Backend Interface.
* Input/result contracts.
* Job lifecycle.
* Error model.
* State ownership.
* Output: architecture and interface validation.

### F2 — Matte Processing Pipeline

* Foreground/alpha contract.
* Preview.
* Composite preview.
* Validation.
* Matte assets.
* Refinement interface.
* Cache.
* Export interface.
* Output: shared pipeline.

### F3 — BMV2 Backend

* Adapter.
* Model Manager integration.
* Reference workflow.
* Validation.
* Preview.
* Refinement.
* Image/video processing.
* Error handling.
* Output: validated BMV2 backend.

### F4 — RVM Backend

Proceed only after license gate approval.

* Adapter.
* MobileNetV3.
* ResNet50 where validated.
* Recurrent State Manager.
* Sequential frame inference.
* Downsample control.
* State isolation.
* Seek/restart/cancel handling.
* Image/video workflow.
* Output: validated RVM backend.

### F5 — Qt6 UI

* AI Matting workspace.
* Backend/model selection.
* Reference workflow.
* Profile controls.
* Preview modes.
* Matte inspection.
* Job queue.
* Progress.
* Error handling.
* Output controls.
* Output: functional Qt6 UI.

### F6 — CFA4 Integration

* Timeline.
* Compositor.
* VFX.
* Background replacement.
* Asset management.
* Render/export.
* Cache.
* Project persistence.
* Output: end-to-end integration.

### F7 — Adaptive Capability Validation

* CPU.
* GPU.
* RAM.
* VRAM.
* Model profiles.
* Resolution.
* Concurrency.
* Preview/full jobs.
* Resource exhaustion.
* Fallback validation.
* Output: hardware validation report.

### F8 — QA, Packaging, Release Gate

* End-to-end.
* Regression.
* Qt6.
* Codec/alpha.
* Ubuntu 26.04 amd64.
* .deb.
* Flatpak.
* AppImage.
* Tarball.
* License notices.
* Model Manager metadata.
* Upstream coexistence.
* Release readiness evidence.

Do not mark a phase complete without test evidence.

## 22. TESTING

### BMV2 tests

* Valid background reference.
* Missing reference.
* Invalid reference.
* Static camera.
* Background mismatch.
* Lighting variation.
* Camera movement.
* Preview.
* Refinement.
* High-resolution processing.
* CPU.
* GPU.
* Cancellation.
* Foreground/alpha integrity.

### RVM tests

* Reference-free video.
* MobileNetV3.
* ResNet50.
* Sequential frame ordering.
* Recurrent state propagation.
* Clip state isolation.
* Job state isolation.
* Seek.
* Range change.
* Cancel.
* Restart.
* State recovery.
* Downsample ratio.
* VRAM failure.
* CPU fallback where supported.
* Foreground/alpha integrity.

### Shared pipeline

* Alpha range.
* Frame count.
* Timestamp integrity.
* Resolution.
* Preview.
* Matte export.
* Foreground export.
* Composite export.
* Cache invalidation.
* Backend separation.
* Model version changes.
* Missing model.
* Project reopen.
* Timeline relink.
* Compositor relink.
* VFX relink.
* Temp cleanup.
* Resource cleanup.

### End-to-end test

Import → Matting → Preview → Validation → Refinement → Background Replacement → Compositor → Timeline → Render → Export → Project Reopen.

Run the E2E workflow separately for each supported backend where the backend prerequisites are satisfied.

## 23. ACCEPTANCE CRITERIA

* Existing CFA4 plans and implementations audited.
* Gap-only scope confirmed.
* Unified engine architecture implemented.
* Shared backend interface implemented.
* BMV2 adapter validated.
* RVM adapter validated, if license approved.
* BMV2 reference workflow functional.
* RVM recurrent state lifecycle correct.
* Qt6 UI functional.
* Model Manager integration functional.
* User-controlled model downloads only.
* Adaptive Capability integration validated.
* Preview modes functional.
* Matte validation functional.
* Timeline integration functional.
* Compositor integration functional.
* VFX integration functional.
* Background replacement functional.
* Cache invalidation correct.
* Project persistence/reopen validated.
* Alpha export validated.
* CPU/GPU paths validated where supported.
* Resource limits enforced.
* Licensing audit documented.
* Ubuntu 26.04 amd64 validated.
* All packaging formats validated.
* Upstream coexistence validated.
* E2E tests passed.
* Evidence attached to every completed phase.

## 24. EXCLUSIONS

This plan does not include:

* Full rotoscoping.
* AI tracking system.
* 3D depth reconstruction.
* General-purpose object segmentation.
* Chroma key replacement.
* AI model training platform.
* Cloud inference.
* Automatic model download.
* A new AI Model Manager.
* A new VFX system.
* A new Compositor.
* Upstream standalone application duplication.
* Windows/macOS.
* Non-amd64 targets.
* Repository changes performed by ChatGPT.

These exclusions prevent scope duplication with existing or planned CFA4 systems.

## 25. FINAL IMPLEMENTATION PRINCIPLE

BMV2 and RVM must become two complementary backends of one CFA4-native AI Matting Engine.

BMV2 provides reference-background-based matting and refinement.

RVM provides reference-free, temporally guided human video matting with recurrent state.

Both feed the same matte-processing pipeline and integrate with CFA4 AI Video Tools, Timeline, Compositor, VFX, Background Replacement, and Render/Export.

Reuse existing CFA4 architecture. Implement only verified missing capabilities. Preserve hardware limitations, local-first execution, user-controlled models, and non-destructive media workflows.

End of specification.
