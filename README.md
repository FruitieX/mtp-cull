# mtp-cull

This program aims to automate as much as possible of a fast, safe photo backup and culling routine.

The command-line MTP backend currently works on Windows. The application and media-source boundary compile on Windows, Linux, and macOS so platform MTP backends can be added without coupling the culling UI to one protocol implementation.

- [x] List all files under some given path on an MTP device
- [x] Copy the files to a location such as `X:/Pictures/Out-of-camera/2023/2023-12-28 Album name/DSCF1234.JPG` where:

  - `X:/Pictures` is a configurable base path
  - `Out-of-camera` is used for JPEG files, `Undeveloped` for RAW files or `Video` for video files
  - `2023` is the current year (defaults to timestamp when the program is run, but can be overridden)
  - `2023-12-28` is the current date (defaults to timestamp when the program is run, but can be overridden)
  - `Album name` is a configurable album name

- [ ] Select which photos to keep (so that both JPEG and RAW files are deleted if the JPEG is deleted)
- [ ] Optionally delete the files from the MTP device after copying
- [ ] Upload the resulting album to Google Photos

## Local culling

The `ui` command opens a local culling session. Select an album folder, then optionally choose a separate RAW folder. Files are paired by exact filename stem, so `DSCF0001.JPG` and `DSCF0001.RAF` receive one decision and are recycled together.

- Decisions are stored in the platform's per-user application-data SQLite database by BLAKE3 content fingerprint, so they survive album moves and renames.
- JPEG companions are used for fast preview. RAF, DNG, HEIF, and videos remain included in pairing and decisions; their native preview backends are not implemented yet.
- `1` rejects, `2` keeps, and `0` clears the decision. These and navigation, fit/100%, and A/B shortcuts are configurable in the UI.
- Sharpness is a background Tenengrad-style score for JPEG companions. It is only a sortable hint, never an automatic decision.
- Rejections are applied only from an explicit review screen. Every affected file is re-fingerprinted before it is sent to the platform recycle bin/trash.

On Windows, `ui` also supports direct MTP culling: choose a device and source folder, cull temporary cached JPEG companions, then import only explicit Keep pairs. Device files are never deleted; rejected and unrated files remain on the device. The worker design and verification constraints are documented in [`docs/direct-mtp-culling.md`](docs/direct-mtp-culling.md).

## Development

The repository pins its tested Rust toolchain. Run the standard checks with:

```console
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```
