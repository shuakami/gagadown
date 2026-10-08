# GaGaDown localization and Windows text rendering release plan

## Status and evidence rules

Baseline: d34337a13da76df15dcbde6bcfff5003089a17f7 (v0.1.3).
Work branch: work/i18n-text-rendering.
This is a working plan, not a completion report. An unchecked item is not verified.
Do not publish a release until all acceptance gates pass. Do not store real download URLs, cookies, tokens, or user state in evidence.
Use CI-built binaries, not local builds, consistent with the existing user constraint.
Use isolated application state and small synthetic downloads; never preallocate the user's large download for testing.

## Prerequisites

- [x] Confirm clean baseline and fetch remote references.
- [x] Check repository and ancestor directories for AGENTS.md: none found.
- [x] Create separate work branch to avoid triggering premature releases.
- [ ] Read all tracked source, configuration, documentation, and required vendored patches. Track exact read coverage before implementation.
- [ ] Inventory every user-facing literal and UI surface, including protocol-derived status text.
- [ ] Inspect available displays, DPI, GPU, interactive desktop, screenshot tooling, and CI permissions without changing system-wide display settings.
- [ ] Capture baseline CI diagnostics and baseline Windows screenshots before changing rendering.

## Localization architecture research gate

Initial upstream comparison and rendering trace are recorded in [i18n-rendering-research.md](i18n-rendering-research.md). Fluent with cached static labels is the proposed direction; dependency API verification, benchmarks and the final integration decision remain pending.
Compare rust-i18n against fluent-bundle/i18n-embed using pinned upstream documentation and small isolated measurements.
Evaluate static lookup allocations, lock contention, interpolation, English plural selection, locale switching, missing-key detection, and packaging.
Select one Rust localization stack after recording evidence. Browser chrome.i18n and NSIS LangString remain platform-mandated adapters, not competing Rust stacks.
Core returns structured codes and parameters; app and CLI format them. Preserve useful diagnostic details while scrubbing secrets.
Static labels must avoid repeated parsing and allocation. Dynamic messages can be cached by locale and arguments; benchmark rather than claim zero overhead.
Default is zh-CN, with explicit en and system-language selection. Define fallback behavior for unsupported system locales.

## Terminology

| Chinese | Product English |
| --- | --- |
| 接管 | take over |
| 未接管 | Not taken over |
| 线路 | route |
| 分段 | segment |
| 连接数 | connections |
| 回收站 | Trash |
| 便携版 | Portable |
| 暂停 | Pause |
| 继续 | Resume |
| 重新下载 | Download again |
| 校验 | Verify |
| 缓存 | Cache |

Brand spelling: GaGaDown. No U+00B7 character in new strings, documentation, or commits.

## File change inventory

This inventory will be made exact after the full source review, before implementation.

- Workspace Cargo.toml and Cargo.lock: selected localization dependencies and shared catalog crate if justified.
- crates/gagadown-app/Cargo.toml, build.rs, src/main.rs: localized UI, fonts, settings, tray, windows, diagnostics and extension embedding.
- crates/gagadown-core/src/{config,error,task,api,engine,download,probe,request,route}.rs: structured messages, locale preference storage where appropriate, compatibility and secret scrubbing.
- crates/gagadown-core/src/{lib,limiter,segments}.rs: inspect for coverage; change only if needed.
- crates/gagadown-core/tests/: structured-error compatibility, non-runtime UI thread behavior and existing download regressions.
- crates/gagadown-cli/Cargo.toml and src/main.rs: localized help and output without changing flag names.
- New shared localization catalog/module and tests: bilingual keys, argument schemas, plurals, fallback and switching.
- extension/{manifest.json,background.js,popup.html,popup.js}: native Chrome localization.
- extension/_locales/{zh_CN,en}/messages.json: complete bilingual messages.
- installer/gagadown.nsi: bilingual LangString and system-language selection.
- README.md and new README.en.md: synchronized product documentation and reciprocal links.
- patches/epaint/src/text/, atlas and tessellation sources: physical-pixel placement investigation and bounded fixes.
- patches/egui-wgpu/src/: sampling, blending, shader and capability fallback investigation.
- patches/eframe/src/native/: DPI, surface sizing and viewport lifecycle investigation without breaking hidden-window protection.
- patches/README.md: exact patch rationale and upgrade notes.
- assets/app.manifest: inspect DPI declaration; change only if evidence requires it.
- tools/package.sh and .github/workflows/release.yml: locale resources and required CI gates; preserve all release artifacts.
- New tools for literal scanning, deterministic visual fixtures, lossless capture and pixel measurement.
- docs/i18n-rendering-plan.md and final evidence report: checklist, experiments, results, screenshots and limitations.

## Implementation gates

- [ ] Framework decision with reference links, measurements and rationale.
- [ ] Complete bilingual catalogs with matching keys and placeholders, including plurals.
- [ ] Core structured errors and display-log events; stop parsing localized log strings in route_info.
- [ ] GUI coverage: pages, sidebar, list, empty states, context menus, dropdowns, tooltips, toasts, modals, settings, About, browser guide, tray, download popups, notifications, titles.
- [ ] Locale-aware sizes, speeds, durations, relative and calendar dates.
- [ ] CLI help and output coverage.
- [ ] Chrome extension locale resources and embedded/extracted packaging.
- [ ] NSIS language selection and localized installer strings.
- [ ] README.en.md and reciprocal links.
- [ ] English layout audit with measured widths, preserved alignment and existing menu styles.
- [ ] Windows Segoe UI and Microsoft YaHei fallback; Linux system font fallback.

## Rendering investigation matrix

Each row requires source locations, a testable hypothesis, observations, and a conclusion.

- [ ] a. Glyph bearings, baseline, row layout, fractional DPI, final transformed vertices, padding, UV texel centers and NDC mapping.
- [ ] b. Per-Monitor V2 manifest and winit behavior, DWM stretching, physical client/surface size agreement and monitor transitions.
- [ ] c. Surface color format, shader transfer functions, dual-source blending and contrast.
- [ ] d. Native point/pixel sizing, secondary text contrast, font identity and weight.
- [ ] e. Controlled GDI versus DirectWrite glyph-run analysis comparison; choose based on captured evidence.
- [ ] f. Immediate/deferred popups, modals, toasts and animation transforms, including final settled positions.
- [ ] g. Linux default rendering path and runtime regression check.
- [ ] Unsupported dual-source GPU, ClearType disabled, remote desktop and virtual-machine fallback.

Preserve KEEP_ROOT_UI, first-frame allowance, hidden-before-first-paint child windows, and no presentation while hidden/minimized.
Do not call ctx.input inside ctx.data_mut. Core sync entrypoints use Inner.rt rather than naked tokio::spawn.
Release panic=abort makes panic avoidance a release gate.

## Validation checklist

- [ ] cargo check -p gagadown-app.
- [ ] cargo clippy --target x86_64-pc-windows-gnu -p gagadown-app; compare warning baseline, no new warnings.
- [ ] cargo test -p gagadown-core, including single_resume and ui_thread; never weaken existing expectations.
- [ ] Bilingual key and placeholder equality tests; English singular/plural tests.
- [ ] Source-aware Chinese-literal scan of crates and extension, excluding catalogs/comments; separate protocol input data from UI text explicitly.
- [ ] Prohibited-character scan and secret scan.
- [ ] bash tools/package.sh produces Setup, Portable, CLI and source archives with complete resources.
- [ ] Linux release build of app and CLI, plus actual runtime smoke test.
- [ ] Real Windows capture matrix: 100%, 125%, 150% x Chinese/English x light/dark, lossless native-resolution PNGs.
- [ ] Screenshots cover every interface surface and include native Explorer or Settings reference text.
- [ ] Quantitative before/after report: equal-size crops, normalized comparison area and glyph count, split-stroke frequency, peak ink and colored-pixel fraction; retain script parameters and raw measurements.
- [ ] Review each English screenshot for clipping, overflow, overlap, wrapping and alignment.
- [ ] Test locale changes, persistence and system-language fallback.
- [ ] Verify no presentation stall while minimized/hidden and first child popup becomes visible.
- [ ] Match final installed binary and downloadable artifacts to tested commit, hashes and version tag.

## Implementation evidence log

- Added Chrome bilingual catalogs, manifest localization, popup and context-menu lookups, rejected-ping handling, and desktop embedding of both locale files. Expanded popup width for English; actual browser screenshot audit remains pending.
- `node --test tools/test-extension-i18n.cjs`: 8 tests passed, covering key/placeholder parity, source literals, embedding and both languages across connected/offline/rejected responses. These are mocked DOM tests, not browser screenshots.
- `node --check` passed for popup.js and background.js; `git diff --check` passed.
- NSIS bilingual strings and English version metadata implemented; compilation and actual installer language verification remain pending.
- GDI font face-validation path now restores the previously selected object before rejecting/deleting a font. No blur fix is claimed for this resource-lifetime correction.
- Added shared Fluent catalog crate with typed cached labels, explicit/default/system language resolution, error-returning formatting, switch snapshots and plural tests. GUI/CLI integration and full catalog coverage are not complete.
- Added work-branch-only validation workflow with no release permissions. Rust builds/tests run in CI, not locally.
- Lockfile resolution preserves existing package versions; metadata downloaded dependencies but did not compile locally.

- CI run 37754063141, catalogs job passed for eca5a4c: extension tests and initial Fluent unit tests. Packaging and Windows jobs were still running at inspection; this is not full CI acceptance.
- Added AST-based bilingual key/argument parity tests, including nested selectors and a mutation-detection test. Awaiting CI for these new tests.
- Integrated the shared catalog into App startup with fallible initialization, persisted language preference and a settings selector. Migrated settings controls and proxy mode labels to typed cached labels. Remaining GUI surfaces, CLI and core diagnostic migration are still outstanding.
- Added old-settings compatibility and preference round-trip tests. Added English README with reciprocal links and included both READMEs and LICENSE in the source package.

- Work commits pushed without releasing: eca5a4c, 8ec0c63, 46f4bee. The main branch and release tags remain unchanged by this work.
- CI 37754063141: catalogs and Windows jobs passed, including app check and core tests. Package job remained in progress at last inspection.
- CI 37755375572: catalogs job passed, now including AST parity tests. Windows/package remained in progress at last inspection.
- Navigation tooltip IDs now depend on typed label IDs rather than translated text. About labels and completed-download summary use Fluent.
- Font fallback order now prefers Segoe UI on Windows (Segoe UI Semibold for emphasized text), then Microsoft YaHei; Linux probes DejaVu/Noto before CJK fallbacks. These source changes still require visual verification.
- Added Linux release build and an isolated Xvfb launch smoke test to CI. A timed launch is only startup evidence, not a screenshot/layout or interactive regression test.

- CI 37754063141 and 37755375572 completed successfully (catalogs, Windows checks/tests, cross-target clippy and Windows package).
- CI 37755702038 failed the Linux runtime test after successful compilation: the downloaded runtime log reports missing libxkbcommon-x11.so and a process abort. Added the missing runtime package to the CI environment; the runtime acceptance assertion is unchanged. A rerun is required.
- Began CLI integration: explicit --language, translated command descriptions and progress labels. Full help/error/completion coverage remains unfinished.
- CLI inspection found it unconditionally removed the caller-supplied --data-dir after a download. Cleanup now applies only to the automatically created temporary directory. Also removed an error-path unwrap; runtime regression coverage is still required.

## Observations recorded so far

- Existing app text helpers round galley origin to physical pixels; this alone does not establish final glyph alignment.
- route_info currently parses Chinese log prose. Localization must replace this coupling with structured data, not translate the prefix and hope parsing works.
- Browser extension files are explicitly embedded in main.rs. New locale JSON files must be added to that list as well as packaged.
- build.rs embeds assets/app.manifest. Inspect the actual manifest and backend behavior before concluding DPI awareness.
- Baseline/row rounding, glyph-bearing rounding, final galley rounding, NDC mapping and dual-source capability gating have been traced in source; runtime geometry is not measured.
- ClearType settings are cached for the process lifetime; GDI font rejection attempts to delete a selected object. These deserve separate fixes/tests and do not establish the blur root cause.
- Read-only host inspection found both AMD hardware and a virtual display adapter. Actual DPI and local-versus-remote capture conditions remain unverified.
- No root cause, performance claim, screenshot result or release-readiness claim is established yet.
