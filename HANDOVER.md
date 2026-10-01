# Handover

## Implemented September 2026

The camera-first UI has been replaced. `main.rs` maps `ui` to `app.rs`; the old
`ui.rs` is retained as historical source, and `culling.rs` only compiles for its
legacy database/orientation regression tests. Pre-existing working-tree edits were
preserved in checkpoint commit `855d1bf`. The remote CLI improvements were merged
into the device worker while retaining the new viewer and persistent staging.

Use [README.md](README.md) for workflow/build/shortcuts and [TODO.md](TODO.md) for
the implementation checklist. [docs/performance.md](docs/performance.md) records
measurements and their limits. The original design remains in
[docs/culling-redesign.md](docs/culling-redesign.md).

## Modules

| File | Responsibility |
| --- | --- |
| `app.rs` | UI orchestration, source picker, filters, filmstrip and dialogs |
| `theme.rs` | Dark palette, shared spacing, grouped controls and panel styling |
| `review.rs` | Metadata index, pairing, linking, decisions and undo/redo |
| `review_commands.rs` | Configurable registry, exact shortcut matching/conflicts |
| `review_store.rs` | SQLite settings, decisions and comparison resume state |
| `recent_sources.rs` | Bounded source history and fresh device/path resolution |
| `import_presets.rs`, `preset_ui.rs` | User configuration file and preset editor/home actions |
| `quick_import.rs` | Preset device/list/copy state machine using the existing MTP worker |
| `image_cache.rs` | Bounded decode pool, prioritized demand and CPU tier eviction |
| `viewer.rs` | Shared transforms, side-by-side/wipe/blink, GPU cache and ROI |
| `reel.rs`, `filmstrip.rs` | Transient batch selection, virtualized row/grid, navigation follow and context actions |
| `mipmaps.rs`, `mipmaps.wgsl` | WGPU low-pass pyramid generation, trilinear sampling and deferred texture retirement |
| `focus.rs`, `bursts.rs` | Native gradient maps and optional capture/similarity groups |
| `imports.rs`, `safe_copy.rs` | Local import worker and verified no-clobber copies |
| `mtp_worker/runtime.rs` | Single-device scheduling, generations, progress/cancel |
| `mtp_worker/staging.rs` | Persistent originals, manifests, hash checks and quota |
| `mtp_worker/windows.rs`, `linux.rs` | Native backend operations |
| `performance.rs`, `ui_smoke.rs` | Opt-in benchmark fixtures and native UI exercise |
| `windows_app.rs`, `build.rs`, `assets/` | Windows console lifetime, installer mutex, taskbar identity and embedded branding |

## Keep these boundaries

- Camera handles live on the MTP worker; camera sources remain read-only.
- Imports use explicit selected asset IDs and preflight destination collisions.
- Preserve atomic no-clobber writes and cached-JPEG hash verification.
- JPEG/RAW linking changes effective choices without overwriting independent RAW
  decisions. Media/decision filters only affect visibility.
- Metadata/source locators are not cryptographic content identities. Missing camera
  timestamps must not inherit previous decisions through opaque object IDs.
- Cleanup stays inside recognized application cache directories, protects the active
  session and never recursively deletes user paths.
- Current images remain pinned; UI work never performs image decode or camera IO.
  Running decodes finish at safe boundaries; obsolete results are discarded.

## Recent sources (October 2026)

Successful session installs update a twelve-entry MRU list in the existing SQLite
settings record, mirrored into the settings draft to preserve history when saving
preferences. Local entries include the canonical JPEG and optional RAW folders.
Camera entries retain friendly names/paths and IDs; reopening resolves against a
fresh listing, falling back from device ID to a unique name and from folder ID to
an exact, unique path. Missing/ambiguous matches remain in the picker. Stale camera
folder IDs are never reused. This history is separate from cache identity and
review choices; clearing it does not clear either. Older settings default to an
empty list. Metadata listing runs on the existing device worker; local rescans run
on the indexer thread. No history-related filesystem probing runs during paint.

The welcome screen shows five suggestions; the toolbar menu shows twelve. Its
camera button suggests the last camera path without starting review automatically;
clicking a specific recent source does start review after successful resolution.
Welcome action buttons use their actual text/padding width for centered alignment.

## No-argument launch and copy presets

CLI subcommands are optional; `main` defaults to `Ui(UiArgs::default())`. Presets
are user-owned JSON under ProjectDirs.config_dir, starting empty. Development
isolation redirects them into MTP_CULL_DATA_DIR. They deliberately do not live in
SQLite Settings or have baked-in defaults. Atomic config saves preserve malformed
external files instead of overwriting them. The editor supports add/edit/duplicate/
delete, recent camera paths, external reload and a copyable config path.

Preset imports run only without an active review. They close any retained camera
session and use the existing worker's ListDevices, ListFiles and CopyFiles requests
without staging JPEGs. The shared CLI plan_copy function preserves destination
layout and rejects flattened collisions before writes (including case collisions).
Today is resolved when each run starts; named devices must match uniquely. The
state machine ignores duplicate or unrelated events. Cancellation closes the
worker session, invalidating in-flight results by generation, and completed files
remain intact. Progress and per-file failures are presented by the UI. Review and
image decoding remain independent of presets.

## Windows distribution (October 2026)

`build.rs` embeds a multi-resolution ICO and Cargo product/version metadata using
winresource. `app.rs` uses the matching checked-in raw RGBA window icon. The
console subsystem remains enabled: `windows_app.rs` detaches only UI-owned
consoles, gives detached startup errors a message box, and aligns taskbar identity
with installer shortcuts. A lifetime mutex covers UI and CLI work for the Inno
Setup install/uninstall guard; it does not restrict multiple application instances.

`scripts/package-windows.ps1` builds the production SIMD executable, bundles the
installed MSVC x64 redistributable DLLs and Windows dependency/native/font notices,
then produces an Inno Setup per-user installer, portable ZIP and SHA256 checksums.
Keep the installer AppId stable. No installer rule touches user AppData or
recursively deletes unmanaged files. Reviewed license supplements are tied to
specific dependency versions; missing notices block packaging.

The release workflow builds/verifies packages manually or on matching version
tags, creating draft releases only. No signing certificate is configured. Package
resource/CLI/checksum checks and an isolated install, upgrade, optional shortcut
and uninstall test pass locally. Native Explorer-style no-argument launch confirms
console detachment. Clean-machine and GitHub workflow execution remain unverified.
See [docs/windows-packaging.md](docs/windows-packaging.md) for repeatable commands.

## Reel selection and image sampling

Editing selection is separate from persisted review decisions and import choices.
Ctrl-click toggles, Shift-click selects a visible range and Ctrl+A selects visible
shots without marking them Keep. Normal navigation collapses to the current shot;
filters prune hidden members. Batch decisions use one History.apply transaction;
auto-advance only applies to a single shot. Active comparison A takes precedence
over a reel batch. Default Keep/Reject is now 1/2; Store migrates old default pairs
and Ctrl+A bulk-Keep while preserving custom shortcuts and recording migration.

The resizable reel virtualizes visible columns/rows, with follow requests only on
navigation/filter/layout changes so manual scrolling stays independent. Row mode
maps ordinary wheel input to horizontal scrolling. Settings persist height, grid
mode, grid cell size and wheel multiplier. Vector button icons avoid missing font
glyphs. Context actions route through the same command registry as shortcuts.
Thumbnail GPU memory and per-frame uploads are bounded independently of the
canvas, with visible thumbnails pinned.

egui-wgpu managed textures contain one level even when TextureOptions requests
mipmaps. Smooth sampling therefore registers native GPU textures, generates the
pyramid once on upload and uses trilinear sampling. Existing linear/nearest modes
use managed textures. Cache accounting includes each mip level; frame upload
limits remain bounded. Native texture retirement waits for the next frame so
current meshes retain valid image IDs. Session reset preserves the GPU backend.
The native reel smoke includes GPU readback proving a pixel checkerboard averages
to gray, plus actual mouse/keyboard input for batch/reel/context/resize behavior.

## Verification

Windows standard and SIMD builds pass tests and strict Clippy. Native GPU smoke
exercises comparisons, focus/ROI, linked decisions, undo/redo, import and identical
retry. The Linux adapter's API harness passes separately; native Linux remains
unverified in this Windows environment. See performance documentation for exact
counts/results and repeatable commands.

`scripts/build-windows.ps1` uses CMake/NASM for SIMD and configures libjpeg-turbo to
match Rust's MSVC runtime. The normal default build remains pure Rust. Environment
variable `MTP_CULL_DATA_DIR` isolates settings/staging for development; run the UI
smoke harness only against fixtures with its own output/database directory.

## Remaining hardware work and future scope

Validate a full X-T5/NAS session, disconnect/reconnect and cancellation behavior on
the physical camera. Native Linux USB/udev checks remain necessary; see
[docs/linux.md](docs/linux.md). CLI MTP listing and copying support Windows and Linux.

RAW rendering, video playback, color-managed previews, automatic image registration,
face/eye suggestions, offline camera manifest browsing, and source deletion are
outside this implementation. Focus scores remain hints, particularly with high
ISO noise or unlike textures. Native full-image GPU uploads can still exceed a
frame budget on cold navigation; tiling is a possible follow-up after real JPEG
and presentation measurements.

## October 2026 checkpoint and UI polish

Checkpoint `855d1bf` was preserved and pushed with the remote CLI integration in
merge `60ffe77`. The CLI now uses the same worker on Windows and Linux; the retired
Windows CLI backend and disposable preview cache were superseded.

The UI has neutral dark panels, mint accents, grouped comparison controls,
filmstrip filters, a compact status bar, and Review/Performance/Shortcuts settings
tabs. Viewer rendering/transforms, image loading and culling semantics remain
unchanged. Command IDs for all-media and all-decisions filters are now distinct;
legacy editor-generated Shift+A bindings migrate to the decision filter without
altering other customizations.

Native smoke checks cover 2560x1440 and 1024x768, including Settings tabs and the
welcome screen. See performance documentation for timings and limitations.
