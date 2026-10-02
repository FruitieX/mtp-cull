# mtp-cull

Review camera JPEGs on the PC, choose the keepers, then copy their JPEG and RAW
originals to your NAS. Source files remain intact. The MTP CLI
copy command keeps its original workflow.

## Run

For Windows, use the installer or extract the complete portable ZIP from a release.
The installer adds a Start-menu shortcut and an optional desktop shortcut, without
requiring administrator privileges. Both packages include the runtime DLLs and
application icon. See [Windows packaging](docs/windows-packaging.md) for building
packages, verification and release automation.

The SIMD release build is recommended for camera JPEGs. On Windows, install
Rust (the repository pins its toolchain), Visual Studio C++ build tools, CMake,
and NASM, then run:

```powershell
./scripts/build-windows.ps1
./target/release/mtp-cull.exe
```

Use a release build for photo review. Plain `cargo run` builds unoptimized code;
decoding and resizing camera JPEGs can then take long enough to look stuck.
Staged camera originals and review decisions persist, but decoded CPU/GPU
previews and thumbnails are rebuilt in memory after each launch.

Running the binary without arguments opens the UI, including when double-clicked
in Windows. UI launch closes its Explorer-created console; launches from an
existing terminal keep that terminal attached. Explicit CLI subcommands and
`--help` still work.

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
   **Settings > Review > Colourblind-friendly colours** switches decisions to
   blue/orange/gray and editing selection to white. Decision labels and button
   icons also distinguish states; the preference is remembered between launches.
   Both images share zoom and pan. 100% maps a source pixel to a physical screen
   pixel, including Windows scaling. Alt-drag B for manual alignment.
4. Open **Focus** and enable **Sharpness overlay** for native-resolution green peaking. Draw or create a shared
   comparison region to see both region scores. Adjust threshold and opacity.
   These are inspection aids; noise, texture and JPEG processing affect scores.
5. Open **Import selected**, choose JPEG/RAW/video destinations, date and album,
   and copy. Videos default to selected and included. Unreviewed JPEGs are excluded.
   Each media type uses its own destination root: `root/year/date album/name`.
   **Destination preset** fills the roots and album from a saved import preset,
   preserving the session date and all review decisions. **Save as preset** opens
   the editor with these settings and the current camera filled in; give it a
   camera/photographer name and save. Saving returns to the import dialog.

The toolbar groups decisions, pin/swap, comparison mode, zoom and focus tools.
Media and decision filters sit above the filmstrip. **More** opens Settings,
keyboard help and the command palette; **Performance** in the footer shows cache
and frame diagnostics. Settings has separate Review, Performance and Shortcuts tabs.

The reel follows the active photo when navigating with the arrow keys. Ctrl-click
toggles photos in a batch; Shift-click selects a range (Ctrl+Shift adds a range).
Ctrl+A selects all visible photos without changing their decisions. **Keep**,
**Reject** and **Unreviewed** (Clear) apply to the batch as one undoable action.
A normal click or previous/next navigation returns to one photo. Blue outlines and
check marks show the editing selection; green/red/gray borders show decisions.
Filtering drops hidden photos from the editing selection. When comparison pane A
is active, decision commands apply to A instead of the reel batch. Import still
copies all Keep decisions, independently of the current editing selection.

Choose **Reel > Bottom / Left / Right** above the thumbnails to move the reel;
**Ctrl+Shift+G** cycles the three positions. Settings > Review also controls placement.
A single strip runs left/right at the bottom and top/bottom on either side.
Drag its edge facing the viewer to resize it; strip thumbnails grow with the panel.
**Grid** (Ctrl+G) switches to a grid with vertical scrolling; **Row** (bottom) or
**Strip** (side) returns to a single strip. A normal wheel scrolls along the strip,
or vertically in a grid; Shift is optional. Up/Down moves between grid rows in
the same column (the last photo is used in an incomplete row); Left/Right moves
one photo. Up/Down moves one photo in a single strip. Navigation stops at the
first/last row, and the reel follows the active photo.
Grids at all reel positions spread their columns across the full reel width,
choosing the column count that keeps thumbnails closest to your preferred size.
Use the **Size** slider above the grid to adjust that preference;
**Ctrl+Alt+Minus/Equals** makes
them smaller/larger. Resizing the sidebar reflows the grid and keeps the active
photo at the same relative viewport position while dragging. Both strip and grid
layouts preserve this position during resizing, without moving the photo to
opposite scroll edges. If you have scrolled away from the active photo, resizing
retains the photos you are viewing. The size preference ranges from 100 to 390 px
and is remembered. It is also available in Settings > Review, alongside scroll
speed.
Placement, bottom height, sidebar width and layout preferences persist.
Right-click a thumbnail for decision/selection/pin actions, or the canvas for
decisions and viewing tools.

Settings > Performance offers **Smooth (mipmaps)**, **Linear** and **Nearest
neighbor** image sampling. Smooth is the default and reduces zoomed-out moiré with
a GPU-generated low-pass pyramid and interpolation between levels. It uses about
one-third more image texture memory, counted in the canvas cache budget. Linear
blends neighboring pixels using less memory, but can show moiré when zoomed out;
nearest neighbor shows pixels without interpolation.
Full-resolution focus analysis remains independent of display sampling.

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

## Import presets (without culling)

On the home page, **Add import preset** saves a device/source and destinations for
one-click copying. You can also use **More > Import presets** or Ctrl+Shift+I.
Set a name, device name, recursive MTP source path, photo/video/RAW destination
roots, optional date/album, and whether to continue after individual file errors.
A blank device uses the first connected device, a blank source uses the device
root, and a blank date means today at the time the preset runs. Duplicate a preset
to reuse device settings for another photographer or destination.

Saved presets appear under **Import without reviewing**. Click one to copy all
supported photos, RAWs and videos through the CLI copy worker, with progress,
cancellation, collision checks and identical-file skipping. This does not stage
JPEGs or apply review picks. Close an active review session to return home and run
a preset. Completed destination files survive cancellation; source files remain
intact. The output layout is exactly `destination/year/date album/filename`.

Presets start empty and are stored separately from the review database in
`import-presets.json` under the platform user configuration directory. On Windows:
`%APPDATA%/fruit/mtp-cull/config/import-presets.json`. The editor shows the exact
path and has **Copy path** and **Reload file** controls for external editing. Linux
uses `$XDG_CONFIG_HOME/mtp-cull` (normally `~/.config/mtp-cull`); macOS uses the
application's directory under `~/Library/Application Support`. Saving atomically
replaces valid JSON; malformed/unsupported configuration is reported and preserved.
`MTP_CULL_DATA_DIR` redirects this file too for development and smoke isolation.

## Main shortcuts

All bindings are configurable in Settings. F1 shows the full list; Ctrl+P opens
the command palette. Tab/Enter/Space also operate focused UI controls.

| Action | Default |
| --- | --- |
| Previous / next / next unreviewed | Left / Right / N |
| Keep / reject / clear / toggle keep | 1 / 2 / 0 / Space |
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
| Select all visible / clear reel selection | Ctrl+A / Ctrl+Shift+A |
| Toggle active photo in reel selection | Insert |
| Strip / grid reel | Ctrl+G |
| Move reel: Bottom / Left / Right | Ctrl+Shift+G |
| Smaller / larger grid thumbnails | Ctrl+Alt+Minus / Ctrl+Alt+Equals |
| Keep / reject visible images | Ctrl+Shift+K / Ctrl+Backspace |
| Burst grouping / previous-next burst | Ctrl+B / Ctrl+PageUp-PageDown |
| Open folder / camera / import | Ctrl+O / Ctrl+M / Ctrl+I |
| Pause staging / retry / cancel | Ctrl+Space / F5 / Escape |

Global shortcuts yield to text input and dialogs. Escape closes an idle dialog;
outside a dialog it cancels an operation or pauses staging.
Existing old default Keep/Reject and bulk-Keep bindings migrate to the new defaults;
custom bindings are preserved.

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
