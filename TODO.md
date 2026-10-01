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
