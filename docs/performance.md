# Performance and validation

Measurements below are from this Windows workspace on 2026-09-30. The native
renderer reported an RTX 4070 Ti. The automation desktop exposes 1024x768 pixels,
so this is not a 1440p presentation test. No physical camera or NAS transfer was
performed. Fixtures are generated patterns with blur/noise, 7728x5152 pixels,
quality-92 JPEG; RAF/video fixtures contain placeholder bytes for pairing/import
tests. They exercise image dimensions and the workflow, not real RAF/video decoding.

## Decoder evaluation

Single-file sequential release measurements over eight generated JPEGs. Files
were already in the filesystem cache; reported times include decode, orientation,
resizing and egui color conversion. Focus additionally computes the native map.
These initial measurements preceded the later EXIF/burst metadata additions.

| Path | image/zune median | SIMD turbo median | SIMD p95 |
| --- | ---: | ---: | ---: |
| Thumbnail 192 px | 175.67 ms | 77.42 ms | 116.90 ms |
| Fit 2304 px | 244.72 ms | 142.25 ms | 183.67 ms |
| Native 40 MP | 171.22 ms | 140.56 ms | 181.03 ms |
| Native + focus map | 407.15 ms | 343.83 ms | 384.61 ms |

The `turbo` feature uses SIMD libjpeg-turbo and scaled JPEG decompression for
thumbnail/fit tiers. The default pure Rust decoder remains available to simplify
building. Windows SIMD builds require CMake/NASM; the supplied build script aligns
the C runtime with Rust and produces a standalone executable without a turbojpeg DLL.

Cold decoding exceeds the proposed 20-30 ms navigation target. The implementation
therefore prepares neighboring JPEGs and textures in the background. The target
applies to warm navigation with ready local data, not an uncached MTP object or
an uncached full-resolution upload.

## Native UI exercise

The GPU UI smoke harness generated screenshots of side-by-side, vertical wipe,
100% detail, and focus/region comparison. It exercised Keep/Reject/undo/redo,
JPEG-to-RAF linkage, default-selected video, copying five selected assets,
identical import retry, and comparison/session resume. The final recorded run:

- 117 warm UI CPU frame samples: median 0.251 ms, p95 0.442 ms, max 0.569 ms.
- Approximately 491 MiB canvas GPU textures and 1866 MiB cached CPU images.
- Native framebuffer: 1024x768, DPI 1; requested 2560x1440 was limited by the
  automation desktop. Actual 1440p/DPI scaling still needs hardware testing.

These CPU timings exclude GPU submission/presentation and input-to-photon latency.
They do not establish a 30 ms end-to-end guarantee. Full native uploads, cache
misses, camera transfer and NAS writes must be measured with actual camera JPEGs.

## Bounds and tradeoffs

- Two to six decode workers; at most 128 queued demands and four completed buffers.
  A navigation jump replaces pending demands; in-flight decodes finish safely and
  obsolete generations are discarded.
- CPU residency defaults to 8 GiB: 5% thumbnail, 20% fit, 75% native. Active images
  are pinned and can exceed an undersized tier. Worker/decoder temporaries and
  completion buffers add to resident memory; 64 GB leaves ample headroom.
- Native prefetch adapts to the cache budget. Focus maps are requested separately
  from ordinary 100% viewing, with nearby analyzed frames prepared when enabled.
- Canvas GPU residency defaults to 512 MiB. Filmstrip textures have a separate
  256-entry LRU (roughly 25 MiB at 192 px), virtualized to the visible strip.
- At most two canvas uploads per frame, with a 32 MiB allowance. One oversized
  native upload is permitted to make progress; the other pane defers. Cold native
  upload may exceed a frame target. Active textures can exceed a small GPU budget.
- Whole JPEG originals stage to disk with a global 32 GiB default quota; old
  inactive sessions can be evicted. Session selections remain in SQLite.
- Focus uses fixed-scale source-resolution gradients, a noise floor and spatial
  support. Flat/low-amplitude noise and blur/texture fixtures are tested. High ISO
  noise, repetitive detail, sharpening and unlike crops can mislead the scores.
- Optional bursts combine EXIF capture time and dHash similarity, without automatic
  decisions. Missing timestamps remain ungrouped.

## Repeatable checks

```console
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo test --release generate_review_fixtures -- --ignored --nocapture
cargo test --release decode_benchmark -- --ignored --nocapture
cargo test --release --features turbo decode_benchmark -- --ignored --nocapture
```

Set `MTP_CULL_BENCH_SOURCE` to a local directory of real JPEGs to replace the
generated benchmark set. On Windows, set `CMAKE_TOOLCHAIN_FILE` to the repository's
`build-support/turbo-windows.cmake` and put NASM on PATH for direct turbo cargo
commands; `scripts/build-windows.ps1 -Test` configures those for the main build.

For the opt-in native UI smoke harness, build with
`scripts/build-windows.ps1 -Test -Smoke`, generate fixtures, then:

```powershell
$env:MTP_CULL_SMOKE_DIR = "$PWD/target/smoke-new-run"
$env:MTP_CULL_DATA_DIR = "$env:MTP_CULL_SMOKE_DIR/data"
./target/release/mtp-cull.exe ui --source "$PWD/target/review-fixtures"
```

Use a fresh output directory: the harness expects the first import to copy and
the second to skip. `PASS.txt`/`FAILED.txt` and PNG captures record results, then
the app exits. The harness requires its database inside its output directory.
Without `ui-smoke` and its environment variables, no scripted UI work runs.

The September verification suite passed 40 tests (two heavy fixture/benchmark tests
are opt-in). The Linux backend/API harness passes 14 tests and strict Clippy on
the Windows host. That harness is a local verification aid under ignored target/;
native Linux CI and physical USB integration remain separate checks.

## October 2026 UI refresh verification

The new presentation keeps the viewer, decode queue and image-cache implementations
unchanged. Native GPU smoke now runs at full 2560x1440 (DPI 1), and separately at
1024x768 to check smaller-window layouts. It also captures the welcome screen and
Review/Performance/Shortcuts settings tabs. Set `MTP_CULL_SMOKE_SIZE=1024x768` to
repeat the compact-window run; the default remains 2560x1440.

The functional suite now includes the merged CLI request/retry path, safe-copy
regressions and distinct persisted shortcut IDs. Windows has 53 passing tests
and two opt-in fixture/benchmark tests; the Windows-hosted Linux adapter/API
harness has 21 passing tests. Formatting and strict Clippy pass.

The physical X-T5 has staged 232 JPEGs successfully. The user reports smooth
animated zoom and pan on a 240 Hz display. NAS import, disconnect/reconnect,
high-DPI scaling and instrumented presentation latency still require validation.

Final refresh runs used 117 warm frame samples each, with generated 40 MP fixtures:

| Native viewport (DPI 1) | Median UI CPU | p95 | Maximum |
| --- | --- | --- | --- |
| 2560x1440 | 0.266 ms | 0.435 ms | 0.584 ms |
| 1024x768 | 0.232 ms | 0.333 ms | 0.482 ms |

These are application/UI CPU times, excluding GPU presentation and input-to-photon
latency. Both runs pass comparison/focus, linked selections, import/retry and resume.
Settings screenshots confirm visible Save/Cancel controls without binding conflicts.

The recent-source update adds persistence/backwards-compatibility, bounded MRU and
missing/ambiguous reconnect-resolution regression coverage. Native smoke reopens
an actual fixture folder through its recent entry and checks its saved linked
keep/reject decisions. Welcome screenshots at both sizes include local and camera
suggestions and confirm centered actions. The camera entry in those captures is
synthetic; no physical camera was contacted by this smoke run. Fresh MTP reconnect
resolution still needs a physical-device check. Warm UI CPU medians/p95 were
0.264/0.373 ms at 2560x1440 and 0.264/0.351 ms at 1024x768 in these runs.

The default-launch/preset suite additionally verifies CLI parsing, JSON round trips
and malformed-file preservation, date resolution per run, shared CLI destination
planning, named-device ambiguity, and a complete preset/list/copy/retry round trip
through a simulated device worker without staging. Native smoke launches the
binary with no arguments in isolated empty configuration and captures the home
page and new-preset editor. Full fixture runs at both window sizes load six
profiles from an isolated JSON file to exercise bounded preset lists and the editor.
Profile fixtures contain generic sample paths and never contact a physical device.
Save controls stay outside the editor scroll area. Physical phone/NAS preset
imports still need verification with real devices and destinations.

## Windows package verification

Application branding and packaging retain the existing viewer/decoder behavior.
The production SIMD suite passes 53 tests (two fixture/benchmark tests remain
opt-in), formatting and strict Clippy. The native 2560x1440 fixture smoke passes
comparisons, focus/ROI, selections, import/retry and resume; 117 warm frame samples
have median/p95/maximum UI CPU times of 0.271/0.470/0.705 ms. These exclude GPU
presentation and input-to-photon latency.

A separate Explorer-style no-argument process launch verifies that its owned
console is detached and the home page/preset editor work in isolated configuration.
Package checks validate embedded icons/version information, bundled runtime and
licenses, CLI success/errors and checksums. An isolated installer lifecycle test
passes per-user install, upgrade, default/optional shortcuts, uninstall and
preservation of unmanaged user files. Repeatable packaging commands and remaining
clean-machine/release checks are in [windows-packaging.md](windows-packaging.md).

## Reel selection and mipmapped sampling verification

The default and SIMD functional suite passes 60 tests, with two fixture/benchmark
tests opt-in. Formatting and strict Clippy pass, including the native smoke feature.
New coverage checks batch selection/ranges, pruning filtered photos, JPEG/RAW
linking with batch undo, single-shot auto-advance, active-A precedence, old shortcut
migration and mip-level memory accounting.

Native input tests at 2560x1440 and 1024x768 use a synthetic 500-shot session backed
by three generated JPEG/RAF pairs. They exercise real egui keyboard/mouse events
for arrow-key follow, Ctrl-click, Ctrl+A, batch decisions, reel/canvas context
actions, ordinary wheel scrolling in row/grid and dragging the panel resize edge.
This verifies session-scale layout/selection behavior, not camera throughput or
500 independent cold image decodes. GPU readback verifies that an 8x8 pixel
checkerboard averages to gray in its generated 1x1 mip level.

The full native viewer/import/resume smoke also passes with Smooth sampling:

| Native viewport (DPI 1) | Median UI CPU | p95 | Maximum |
| --- | --- | --- | --- |
| 2560x1440 | 0.247 ms | 0.343 ms | 2.135 ms |
| 1024x768 | 0.268 ms | 0.428 ms | 1.759 ms |

Each run has 117 warm frame samples. GPU presentation, input-to-photon latency and
cold uploads are excluded. Real-photo moiré still needs visual assessment; the GPU
check establishes that mip reduction works, rather than predicting every scene.

Mipmaps are generated once per image upload on WGPU, with trilinear sampling on
subsequent frames. Their levels count against the canvas GPU cache; the original
native/focus pixels and analysis remain unchanged. Thumbnail uploads are capped
at four textures/8 MiB per frame, and their separate cache is limited to 256
textures or one quarter of the configured canvas budget (minimum 16 MiB).
Visible thumbnails remain pinned. Row/grid rendering is virtualized.

To repeat the reel check after building with `scripts/build-windows.ps1 -Smoke`,
generate fixtures as above and use a fresh directory:

```powershell
$env:MTP_CULL_SMOKE_DIR = "$PWD/target/reel-smoke-new-run"
$env:MTP_CULL_DATA_DIR = "$env:MTP_CULL_SMOKE_DIR/data"
$env:MTP_CULL_SMOKE_REEL = '1'
./target/release/mtp-cull.exe ui --source "$PWD/target/review-fixtures"
```

Use `MTP_CULL_SMOKE_SIZE=1024x768` for the compact run. Remove
`MTP_CULL_SMOKE_REEL` for the full viewer/import smoke. Production package builds
omit `ui-smoke`; these synthetic input events never run in normal use.

## GPU prefetch budget correction

Speculative uploads previously checked only whether the cache was already full,
allowing one more native image to exceed it. The following frame evicted that
image, then prefetch uploaded it again. Mipmaps made this easier to trigger by
adding approximately one third to image storage. Admission now checks the entire
image pyramid against the remaining budget before uploading. Visible images can
still exceed an undersized user budget so both comparison panes make progress.

The regression suite passes 63 tests (two opt-in), including filtered grid row
navigation, reuse of review destination presets and mip-aware upload admission.
Native input verifies Up/Down in the 500-shot grid. A 2560x1440 renderer exercise
verifies no new texture uploads over 64 idle frames after native comparison has
warmed, then completes selected import/retry and session/recent-folder resume.
Its 117 warm UI CPU samples have median/p95/maximum 0.254/0.395/2.228 ms; these
exclude GPU presentation and do not establish cold-upload or input latency.

Plain debug builds perform image processing without optimizations. Review with
the SIMD release build; disk staging retains originals across launches, while
decoded preview/thumbnail textures are recreated. A 1-2 GiB GPU cache can retain
more native neighbors when GPU memory allows; the corrected budget check also
works with the existing 512 MiB default.

The reel smoke also exercises left/right vertical strips, Ctrl+Shift+G placement,
side-grid arrow navigation, ordinary vertical wheel scrolling, and the actual
sidebar resize edge. It verifies that the bottom height stays independent and
that left/right share their resized width. The October docking checks ran on a
1024x768 Windows desktop (larger requested windows were clamped to that display).
Only visible strip/grid cells are drawn, including in a 500-photo session.

The fitted-grid follow-up passes 65 tests (two opt-in), strict Clippy and native
500-shot input at 1024x768. Side-grid rows reach the viewport's right edge; the
column count minimizes thumbnail-width differences from the saved preference.
The native exercise changes density with the Size slider and shortcuts, resizes
the sidebar, and verifies active-photo visibility and row navigation after reflow.
Mouse release returns focus from the size slider to the culling shortcuts.
Thumbnail decode sizes still use bounded 128-pixel buckets; GPU upload and cache
limits are unchanged.

## Continuous reel resize verification

The SIMD suite passes 67 tests (two opt-in), formatting and strict Clippy with
the native smoke feature. Regression coverage checks viewport anchoring during
strip resizing and grid column changes, preserving manual scrolling when the
active photo is offscreen, and keyboard follow for oversized thumbnails.

Native 500-shot input checks pass at actual 2560x1440 and 1024x768 viewports.
Each run checks 258 frames during grow/shrink drags across Bottom, Left and Right
placements in both strip and grid modes, including sidebar column transitions.
The active thumbnail's center stays at its original relative viewport position;
the largest measured drift is 0.502 UI points in either run. This verifies live
geometry stability, rather than camera throughput or presentation latency.

Scroll corrections now affect painting and saved scroll state in the same frame.
Resizing no longer requests keyboard follow, which previously aligned the active
photo with opposite scroll edges depending on drag direction. The preferred
thumbnail width now ranges from 100 to 390 pixels. Rendering remains virtualized;
thumbnail decode buckets, upload caps and cache budgets are unchanged.

Bottom grids now use the same fitted columns as side grids. The six reel tests
and strict Clippy pass; closest-size/full-width coverage includes 1440- and
2560-point widths. Native 500-shot runs at 2560x1440 and 1024x768 verify that the
bottom grid's complete rows reach the viewport edge, then pass keyboard row
navigation and the continuous resize checks for all placements.

## Progressive previews and compact controls

Quick 128-pixel decodes precede larger requests at the same priority. Demand is
sorted before deduplication and the 128-job cap, so visible previews cannot be
crowded out by background burst analysis. Exact cached sizes remain a constant
time lookup; fallback sizes are matched by source path. Reel fallbacks exclude
large/native buffers, retain the existing upload/cache caps and stay pinned while
visible. Resizing draws the previous texture until the new bucket is ready.

The viewer retains original source dimensions with GPU textures, so decoded
preview size does not change zoom, pan or alignment. Both comparison panes get
small uploads before a native upload consumes the frame allowance. Resident
textures remain usable after CPU eviction or when a sharper upload is deferred.
Only the requested native analysis supplies focus overlays and region scores.
First previews still require an available local/staged JPEG; this does not fetch
device thumbnails before camera staging completes.

The SIMD suite passes 71 tests (two opt-in), formatting and strict Clippy with
the native smoke feature. Native 2560x1440 and 1024x768 runs pass progressive
side-by-side/wipe previews, correct photo identity, native upgrades, CPU eviction
and old-to-new reel textures during resizing. A separate compact run verifies
the 260-point minimum sidebar. The progressive smoke temporarily holds larger
decode requests to verify the low-resolution frames deterministically:

```powershell
$env:MTP_CULL_SMOKE_DIR = "$PWD/target/progressive-smoke-new-run"
$env:MTP_CULL_DATA_DIR = "$env:MTP_CULL_SMOKE_DIR/data"
$env:MTP_CULL_SMOKE_PREVIEWS = '1'
./target/release/mtp-cull.exe ui --source "$PWD/target/review-fixtures"
```

Build with `scripts/build-windows.ps1 -Smoke` first; production packages omit
this test machinery. The 500-shot reel inputs pass at both viewport sizes with
the compact controls, including continuous resize anchoring. The full 1440p
viewer/focus/import/retry/resume smoke also passes and confirms 64 warmed native
comparison frames without repeated texture uploads. Its 117 warm UI CPU samples
have median/p95/maximum 0.282/0.458/2.672 ms; presentation and input latency are
excluded.

Sidebar media/decision filters and the strip/grid toggle share one row; position
and size share another. Review counts live in the footer. Preview retry appears
after failures, and cache/frame diagnostics are in Settings > Performance.
