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

## Development

The repository pins its tested Rust toolchain. Run the standard checks with:

```console
cargo fmt --all -- --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```
