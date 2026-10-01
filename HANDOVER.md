# Handover

## Implemented September 2026

The camera-first UI has been replaced. `main.rs` maps `ui` to `app.rs`; the old
`ui.rs` is retained as historical source, and `culling.rs` only compiles for its
legacy database/orientation regression tests. Pre-existing working-tree edits were
preserved; no commit was created during this redesign.

Use [README.md](README.md) for workflow/build/shortcuts and [TODO.md](TODO.md) for
the implementation checklist. [docs/performance.md](docs/performance.md) records
measurements and their limits. The original design remains in
[docs/culling-redesign.md](docs/culling-redesign.md).

## Modules

| File | Responsibility |
| --- | --- |
| `app.rs` | UI orchestration, source picker, filters, filmstrip and dialogs |
| `review.rs` | Metadata index, pairing, linking, decisions and undo/redo |
| `review_commands.rs` | Configurable registry, exact shortcut matching/conflicts |
| `review_store.rs` | SQLite settings, decisions and comparison resume state |
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
[docs/linux.md](docs/linux.md). CLI MTP copying remains Windows-only.

RAW rendering, video playback, color-managed previews, automatic image registration,
face/eye suggestions, offline camera manifest browsing, and source deletion are
outside this implementation. Focus scores remain hints, particularly with high
ISO noise or unlike textures. Native full-image GPU uploads can still exceed a
frame budget on cold navigation; tiling is a possible follow-up after real JPEG
and presentation measurements.
