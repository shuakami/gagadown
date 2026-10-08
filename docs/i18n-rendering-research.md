# Localization research and rendering evidence

This document records investigation, not release acceptance.

## Rust localization comparison

Upstream documentation reviewed:

- https://github.com/longbridge/rust-i18n/blob/main/README.md
- https://github.com/projectfluent/fluent-rs/blob/main/fluent-bundle/README.md
- https://github.com/kellpossible/cargo-i18n/blob/master/i18n-embed/README.md

These are moving upstream references, not pinned dependency versions. No dependency has been added and no local benchmark has been run.

| Requirement | rust-i18n | Fluent |
| --- | --- | --- |
| Embedded resources | Compile-time mapping generation | Embed FTL resources; parse into bundles once |
| Interpolation | Documented named arguments | Pattern arguments and selectors |
| English plurals | Reviewed README does not establish automatic plural rules | Language-sensitive selectors; verify with 0, 1, 2 and fractional counts |
| Missing keys | Extractor and optional missing-translation logging | Bundle lookup returns absence; formatting reports errors; i18n-embed-fl adds compile-time checks |
| Switching | Global set_locale or explicit locale | Replace selected bundle/catalog; i18n-embed offers language selection |
| Static hot path | Lookup remains work even without interpolation | Formatting remains work unless cached |

Proposed choice: Fluent with an application-owned typed static-label cache. Resolve static patterns on initialization and language changes, then return borrowed labels by enum index during rendering. Keep dynamic interpolation and plural selection in Fluent. This avoids per-frame global locale locking and pattern formatting, without claiming measured zero overhead. Missing static keys and formatting errors must be rejected by catalog tests, not hidden by fallback. Validate key sets and argument sets in both locales, including identifiers inside selectors and references.

This recommendation is not yet an implemented framework decision or benchmark result. Direct fluent-bundle versus i18n-embed integration must be settled after checking actual dependency APIs and the application's ownership model. Do not add two independent Rust translation systems.

## Rendering trace

Source evidence checked:

- assets/app.manifest already declares PerMonitorV2. Runtime awareness and client/surface dimensions remain unmeasured.
- patches/epaint/src/text/text_layout.rs, galley_from_rows: row heights, baselines and accumulated row positions are rounded through PointScale.
- The same file, tessellate_glyphs: glyph position plus bearing is rounded in both axes before constructing the glyph quad.
- patches/epaint/src/tessellator.rs, tessellate_text: galley origin is rounded to physical pixels; row and glyph positions are then added. Rotated text follows a separate path. Final layer transforms and DPI transitions still need runtime tracing.
- patches/egui-wgpu/src/egui.wgsl: NDC conversion uses screen dimensions without a hardcoded half-pixel offset. This is not evidence that the runtime dimensions are correct.
- patches/egui-wgpu/src/renderer.rs: viewport uses physical screen dimensions. Dual-source pipeline creation is gated by the enabled device feature; the rasterizer subpixel flag follows pipeline availability.
- crates/gagadown-app/src/main.rs, gdi_glyph: physical font size is rounded; GDI font table checks attempt to verify face identity; grayscale uses GGO_GRAY8_BITMAP. ClearType draws into a top-down 32-bit DIB with two pixels of padding and baseline positioning derived from GLYPHMETRICS.
- cleartype_enabled caches the OS smoothing settings in OnceLock. A setting change during the process lifetime therefore cannot update this value. This is a concrete fallback-refresh risk, not proof of the reported split strokes.

Do not add a speculative half-pixel translation: the inspected baseline and tessellation paths already snap. Capture atlas pixels and final screen pixels for the same glyph, physical size, theme and frame, to distinguish rasterization from later resampling or compositing.

GDI object lifetime also warrants review: cached font creation selects a new HFONT before validating identity, and the rejection path calls DeleteObject without first restoring the old selected object. Windows does not permit deleting a selected GDI object. This is separate from the unproven blur cause.

## Host observations

Read-only inspection reported an AMD Radeon RX 9070 GRE at 2560 x 1440 and a GameViewer Virtual Display Adapter. GPU driver: 32.0.31041.1004. Culture and UI culture: zh-CN. SESSIONNAME was empty, which does not establish whether capture is local or remote. No DPI, monitor configuration or desktop settings were changed.

Free space at this observation: C: 9.93 GiB, E: 216.04 GiB. Avoid local release builds, full-size download reproductions, or unbounded capture output. These readings do not attribute the disk usage to any particular process.

No screenshots, DirectWrite comparison, before/after pixel measurements, Linux runtime tests or acceptance builds have been collected during this investigation.
