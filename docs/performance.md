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
