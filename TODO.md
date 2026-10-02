# Camera culling implementation

Goal: implement the workflow and performance architecture in
[docs/culling-redesign.md](docs/culling-redesign.md), preserving CLI copy behavior.
Primary setup: Fujifilm X-T5 RAW+JPEG, 200–500 shots, 1440p, 64 GB RAM, 9950X3D.

## Foundation

- [x] Audit existing code and document the design.
- [x] Refresh stable dependencies and validate the existing Windows build.
- [x] Separate review/session state, command handling, image loading, viewer, and imports.
- [x] Create a fast local-folder index without blocking on content hashes or analysis.
- [x] Pair JPEG/RAW by source folder and exact stem; report conflicts/missing companions.

## Review and comparison

- [x] JPEG/RAW linking enabled by default, with reversible independent asset decisions.
- [x] JPEG/RAW/Video visibility filters independent of selection; videos included by default.
- [x] Keep/Reject/Unreviewed, select toggle, bulk changes, optional advance, undo/redo.
- [x] Pin arbitrary A while browsing B; select either pane and swap A/B.
- [x] Single, side-by-side, vertical wipe, hold-to-blink views.
- [x] Shared image-space zoom/pan, cursor-anchored zoom, correct physical-pixel 100%.
- [x] Keyboard divider controls, manual B alignment and reset.
- [x] Central configurable command registry, conflict checks, help/command palette.
- [x] Shortcuts respect text input and modal focus.

## Performance

- [x] Bounded prioritized decode workers with obsolete work cancellation.
- [x] Separate byte-limited thumbnail, fit, native-resolution and GPU caches.
- [x] Pin active textures; bound uploads per frame; adapt previews to viewport/DPI.
- [x] Directional prefetch with full-resolution neighbors for focus inspection.
- [x] Timings/cache statistics and repeatable release benchmarks.
- [x] Evaluate JPEG decoder alternatives with representative files; record results/limits.

## Camera and import

- [x] Background whole-session JPEG staging with active/nearby priority.
- [x] Pause/cancel staging, disk quota, byte/file progress, close-session handling.
- [x] Session generations on requests/events; reject stale device results.
- [x] Explicit selected asset import plans including default-selected videos.
- [x] Reuse verified staged JPEG originals; transfer only missing selected companions.
- [x] Progress and cooperative cancellation for local and MTP imports.
- [x] Retry interrupted imports without clobber or losing staged data.
- [x] Preserve source files and existing destination/album layout.

## Persistence and focus

- [x] Persist review settings, destinations, selections and resumable session manifests.
- [x] Reconcile reconnects using source identity/metadata; handle ambiguity explicitly.
- [x] Safe cache cleanup and stale-session maintenance.
- [x] Background native-resolution focus map with noise suppression.
- [x] Adjustable peaking threshold/opacity and a linked comparison region.
- [x] Region scores use equal source-pixel scale; no automatic decisions.
- [x] Validate focus behavior against blur, noise and texture fixtures.
- [x] Optional burst grouping based on capture times and image similarity.

## Validation and documentation

- [x] Functional tests for linking/filtering/undo, comparison transforms, queue/cache limits.
- [x] Regression tests for stale events, path collisions, transfers and retry/cancel.
- [x] Run formatting, tests and strict Clippy; validate Linux adapter APIs.
- [x] Exercise UI with representative/generated JPEG+RAF+video fixtures.
- [x] Update README, handover, shortcuts and workflow documentation.
- [x] Record measured navigation/frame/decode performance and remaining hardware checks.

Physical X-T5/NAS and native Linux checks require those devices/environments.
Keep measured software results separate from hardware validation and the proposed
20–30 ms warm navigation target. Automatic registration and face/eye suggestions
are optional future work, outside this implementation goal.

## Completed software verification

- Windows default and SIMD builds: 40 passing tests, plus two opt-in benchmark/fixture tests.
- Strict Clippy and formatting pass; SIMD Windows build has matching MSVC CRT.
- Linux adapter API harness: 14 passing tests and strict Clippy on the Windows host.
- Native GPU smoke: comparison, focus/ROI, linked decisions, selected import,
  identical retry and resume pass. Measurements/limits: [docs/performance.md](docs/performance.md).

Hardware validation is deliberately separate from software checklist completion:

- [ ] Physical X-T5 session (200-500 pairs): enumeration, staging priority, disconnect/reconnect.
- [ ] NAS import, interrupted-copy retry and verifying selected originals byte-for-byte.
- [ ] Actual 1440p/high-DPI input-to-presentation and cold native GPU-upload measurements.
- [ ] Native Linux USB/udev and Nix package checks.

These require the respective hardware/environment. RAW rendering, video playback,
color-managed previews, automatic registration/face suggestions and offline manifest
browsing remain future scope. Initial decoder measurements use generated 40 MP JPEGs;
real camera JPEG benchmarking remains part of the hardware checks.

## October 2026 UI refresh

- [x] Commit and push the functional checkpoint before changing the presentation.
- [x] Integrate remote CLI improvements while prioritizing the new culling implementation.
- [x] Apply a consistent dark palette, spacing and restrained accent colors.
- [x] Group review/comparison tools and move filters next to the filmstrip.
- [x] Simplify thumbnail states and keep progress/errors visible in the compact footer.
- [x] Move diagnostics to a popup and split Settings into three tabs with visible Save/Cancel.
- [x] Fix duplicate shortcut IDs and migrate legacy default bindings.
- [x] Verify native rendering at 1440p and 1024x768, including all Settings tabs.

The physical X-T5 successfully staged 232 JPEGs, with stable metadata available
for cache reuse. The user reports smooth animated zoom and pan on a 240 Hz display;
this is qualitative hardware feedback, not an instrumented latency measurement.
Disconnect/reconnect, NAS imports and native Linux checks remain as listed above.

## October 2026 source and import conveniences

- [x] Keep/reject/unreviewed borders around viewer images and filmstrip previews.
- [x] Persist recent folders (including companion RAW folders) and MTP device/path suggestions.
- [x] Center welcome actions using their actual text/padding dimensions.
- [x] Open the UI when the binary is launched without arguments.
- [x] Add user-owned JSON import presets with add/edit/duplicate/delete and external reload.
- [x] Run one-click home-page presets using CLI copy planning and the existing device worker, without culling or staging.
- [x] Cover preset date/layout/config behavior and simulated-worker copy/retry; verify native default launch and editor layouts.

Physical phone/NAS preset import validation remains a hardware check.

## October 2026 Windows distribution

- [x] Confirm the previous application state is committed and pushed before packaging changes.
- [x] Add an editable camera icon with embedded executable, installer and window assets.
- [x] Embed product/version metadata and align taskbar/shortcut application identity.
- [x] Detach Explorer-created UI consoles while retaining CLI terminal behavior and startup errors.
- [x] Build a per-user installer with Start-menu and optional desktop shortcuts and uninstall registration.
- [x] Preserve presets, cached photos and review choices across upgrade/uninstall; guard running applications.
- [x] Bundle the SIMD release, Visual C++ runtime and dependency/native/font notices in installer and portable ZIP.
- [x] Generate SHA256 checksums and add manual/tag-based builds with draft release automation.
- [x] Verify package resources, CLI compatibility, isolated install/upgrade/uninstall and default UI launch.
- [x] Document package builds, branding and release behavior.

The Windows suite passes 53 tests, with two opt-in benchmark/fixture tests;
formatting and strict Clippy pass. Installer lifecycle tests use a separate
identity and workspace directory, preserving the real application's data.
Clean Windows machine testing and GitHub workflow execution remain environment
checks. Packages are currently unsigned; certificate-based signing is future work.

## October 2026 reel and sampling improvements

- [x] Default Keep to 1 and Reject to 2, migrating old defaults while preserving customized keys.
- [x] Follow previous/next navigation when the active photo leaves the visible reel.
- [x] Add Ctrl-click batch selection, Shift-click ranges and Ctrl+A selection without deciding.
- [x] Apply Keep/Reject/Unreviewed to the batch with linked RAW decisions and one undo action.
- [x] Resize the reel, scale row thumbnails and toggle a virtualized vertical grid with Ctrl+G.
- [x] Scroll the row horizontally with an ordinary wheel and persist an adjustable wheel multiplier.
- [x] Add thumbnail/canvas context menus, matching filter padding and drawn button icons.
- [x] Add Smooth/Linear/Nearest sampling under Performance, with real GPU mipmaps and memory accounting.
- [x] Test selection/linking/undo, key migration and mip budgets; exercise native reel input at 1440p and 1024x768.

Verification passes 60 tests (two opt-in), strict Clippy and formatting. Native
500-shot reel input and GPU checkerboard readback pass at both window sizes;
the full viewer/focus/import/retry/resume smoke also passes. See
[docs/performance.md](docs/performance.md) for timings and limits.

## October 2026 review follow-ups

- [x] Navigate grid rows with Up/Down using the current column count and follow the active photo.
- [x] Stop speculative GPU uploads exceeding the cache budget, including mip levels, to prevent repeated eviction/upload cycles.
- [x] Reuse saved camera/photographer destination presets in the reviewed import dialog without changing decisions or dates.
- [x] Prefill new presets from review destinations and remembered camera names/paths; return to the import dialog after saving.
- [x] Explain import layout and release-build requirements; preserve staged originals and review choices.
- [x] Cover grid boundaries, filtered order, preset semantics and GPU prefetch admission; verify native grid input and idle native comparison.
- [x] Add persistent colourblind-friendly blue/orange/gray decision colours for viewer borders, reel cards and buttons, with distinct white editing-selection markers.

- [x] Replace the viewer's ambiguous "native detail" caption with actual zoom percentage and an explicit full-resolution loading message.
- [x] Dock the reel at Bottom/Left/Right, turn side strips vertical, and remember placement and separate height/width preferences.
- [x] Add a placement menu and Ctrl+Shift+G shortcut; adapt filters, resizing, wheel scrolling and keyboard-follow to side strips and grids.
- [x] Verify persisted settings and virtualized side strips; exercise actual docking shortcuts, side-grid navigation and panel resizing in the native renderer.

- [x] Fill side-grid width with evenly sized columns closest to the preferred thumbnail size, accounting for the scroll viewport.
- [x] Add a live Size slider and configurable smaller/larger grid-thumbnail shortcuts; retain the preference and active photo across reflow.
- [x] Cover nearest-size/full-width geometry and settings persistence; exercise the slider, resize edge and changed grid navigation in the native renderer.

- [x] Preserve the active photo's viewport position continuously when resizing any reel layout or changing thumbnail size; retain manually scrolled views.
- [x] Resolve scroll offsets before painting to eliminate frames combining new thumbnail geometry with the old scroll position.
- [x] Increase the preferred thumbnail width limit from 300 to 390 px (30%).
- [x] Add continuous native drag checks for Bottom/Left/Right in strip and grid mode, including column changes and returning to the original size.
- [x] Fit bottom-grid columns across the full viewport width using the same preferred-size snapping as side grids.
