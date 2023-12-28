# mtp-cull

This is a program that aims to automate as much as possible of my photo backup routine:

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

Windows only due to the MTP crate I'm using only supporting Windows. Porting to Linux may be possible in the future.