# mtp-cull

Review camera JPEGs on the PC, choose the keepers, then copy their JPEG and RAW
originals to your NAS. Source files remain intact. The MTP CLI
copy command keeps its original workflow.

## Run

The SIMD release build is recommended for camera JPEGs. On Windows, install
Rust (the repository pins its toolchain), Visual Studio C++ build tools, CMake,
and NASM, then run:

```powershell
./scripts/build-windows.ps1
./target/release/mtp-cull.exe ui
```

A portable build using the current pure Rust `image`/`zune-jpeg` decoder needs
no CMake/NASM:

```console
cargo run --release -- ui
cargo run --release -- ui --source "C:/Photos/session" --raw-source "C:/Photos/RAW"
```

Linux direct MTP setup is covered in [docs/linux.md](docs/linux.md). With CMake
and NASM available, use `cargo run --release --features turbo -- ui`.
Direct MTP supports Windows and Linux; local-folder review also targets macOS.
The MTP CLI commands support Windows and Linux through the same device worker.

## Review workflow

1. Open **Camera**, choose a device and folder, then start review. JPEG originals
   stage in the background, prioritizing the current image and its neighbors.
   Alternatively, open a local JPEG folder and optionally a separate RAW folder.
2. Mark images **Keep**, **Reject**, or **Unreviewed**. JPEG/RAW linking defaults
   to on: choosing a JPEG also chooses its matching RAF. Pairing uses exact stems
   within the same relative source folder. Ambiguous pairs are flagged and excluded
   from import; missing companions are reported in the import summary.
3. Pin A and browse B. Use side-by-side, vertical wipe, or hold-to-blink comparison.
   Green/red/gray borders show Keep/Reject/Unreviewed in the viewer and filmstrip.
   Both images share zoom and pan. 100% maps a source pixel to a physical screen
   pixel, including Windows scaling. Alt-drag B for manual alignment.
4. Open **Focus** and enable **Sharpness overlay** for native-resolution green peaking. Draw or create a shared
   comparison region to see both region scores. Adjust threshold and opacity.
   These are inspection aids; noise, texture and JPEG processing affect scores.
5. Open **Import selected**, choose JPEG/RAW/video destinations, date and album,
   and copy. Videos default to selected and included. Unreviewed JPEGs are excluded.
   Destination layout is `root/Out-of-camera|Undeveloped|Video/year/date album/name`.

The toolbar groups decisions, pin/swap, comparison mode, zoom and focus tools.
Media and decision filters sit above the filmstrip. **More** opens Settings,
keyboard help and the command palette; **Performance** in the footer shows cache
and frame diagnostics. Settings has separate Review, Performance and Shortcuts tabs.

The welcome screen suggests the five most recently opened sources; **Recent** in
the toolbar lists up to twelve. Folder entries also remember a separate RAW folder.
Camera entries remember the device and folder path: clicking one reopens it after
checking a fresh MTP listing. **Camera** preselects the last used location so you
can change it before starting. If the camera or folder is missing or ambiguous,
choose a current location in the picker; reconnect and **Refresh** to try again.
Hover a suggestion to see its full path. **Clear recent sources** clears the list
without removing cached photos or review decisions. History starts recording with
this version and persists across restarts.

JPEG/RAW/Video and decision filters change visibility independently of selection.
Disabling linking restores independent RAW decisions. RAW and video entries use
JPEG companions where available; native RAF decoding and video playback
are outside this version.

Review decisions, destinations, shortcuts, cache settings and comparison position
persist locally. Reopen the same source to resume. Changed files require review
again. Camera assets without trustworthy timestamps deliberately require fresh
decisions on reconnect. Old fingerprint-based decisions from the retired UI stay
in the database but are not automatically migrated to metadata-based sessions.

## Main shortcuts

All bindings are configurable in Settings. F1 shows the full list; Ctrl+P opens
the command palette. Tab/Enter/Space also operate focused UI controls.

| Action | Default |
| --- | --- |
| Previous / next / next unreviewed | Left / Right / N |
| Reject / keep / clear / toggle keep | 1 / 2 / 0 / Space |
| Undo / redo | Ctrl+Z / Ctrl+Shift+Z |
| Pin A / compare mode / swap / choose pane | P / C / S / Tab |
| Hold A/B blink / fit-100% | B / Z |
| Move / center wipe divider | [ / ] / Backslash |
| Shared pan | Ctrl+arrows |
| Align B / reset alignment | Ctrl+Alt+arrows / Alt+R |
| Focus / threshold / opacity | H / Minus-Equals / Shift+Minus-Equals |
| Draw region / centered region / clear | R / Shift+R / Ctrl+Shift+R |
| Move region / resize | Shift+arrows / Ctrl+Minus-Equals |
| JPEG / RAW / video / all media | J / F / V / A |
| Unreviewed / kept / rejected / all decisions | U / K / X / Shift+A |
| Keep / reject visible images | Ctrl+A / Ctrl+Backspace |
| Burst grouping / previous-next burst | Ctrl+B / Ctrl+PageUp-PageDown |
| Open folder / camera / import | Ctrl+O / Ctrl+M / Ctrl+I |
| Pause staging / retry / cancel | Ctrl+Space / F5 / Escape |

Global shortcuts yield to text input and dialogs. Escape closes an idle dialog;
outside a dialog it cancels an operation or pauses staging.

## Performance and development

The default CPU cache is 8 GiB (thumbnail, fit and native tiers); canvas GPU cache
is 512 MiB and staged originals have a 32 GiB disk quota. Active images stay pinned.
Decode queues are bounded and replaced on navigation. Background work does not
read every RAW file or hash the whole camera before review can begin.

See [measured performance and repeatable checks](docs/performance.md),
[worker/import behavior](docs/direct-mtp-culling.md), and the
[implementation checklist](TODO.md). Camera and NAS performance still require
hardware validation; cold decoding is slower than cached navigation.

```console
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

`scripts/build-windows.ps1 -Test` checks and builds the SIMD version. The former
`src/ui.rs` is retained as historical source; `src/app.rs` is the active UI.
