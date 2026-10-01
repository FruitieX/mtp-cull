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
| `image_cache.rs` | Bounded decode pool, prioritized demand and CPU tier eviction |
| `viewer.rs` | Shared transforms, side-by-side/wipe/blink, GPU cache and ROI |
| `focus.rs`, `bursts.rs` | Native gradient maps and optional capture/similarity groups |
| `imports.rs`, `safe_copy.rs` | Local import worker and verified no-clobber copies |
| `mtp_worker/runtime.rs` | Single-device scheduling, generations, progress/cancel |
| `mtp_worker/staging.rs` | Persistent originals, manifests, hash checks and quota |
| `mtp_worker/windows.rs`, `linux.rs` | Native backend operations |
| `performance.rs`, `ui_smoke.rs` | Opt-in benchmark fixtures and native UI exercise |

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
