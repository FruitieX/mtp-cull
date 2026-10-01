# Linux Support

The direct culling UI supports Linux MTP-mode devices through the pure-Rust `mtp-rs` backend. It is intended for modern Android phones, including Google Pixel, and cameras that explicitly expose MTP mode. PTP/PictBridge-only cameras are not supported yet.

## Development

Use the Nix development shell on Ubuntu, Fedora, Arch, NixOS, or another Nix-supported Linux distribution:

```console
nix develop
cargo run --release --features turbo -- ui
```

The flake supplies Rust, a C compiler for bundled SQLite, CMake/NASM for SIMD JPEGs,
and the Linux windowing/USB development libraries. The default Nix package enables
`turbo`; the MTP backend itself does not require `libmtp`.

## Device Access

The application opens the USB MTP interface directly. Your user must have access to the connected device, and a desktop MTP daemon must not already own it.

1. Eject or unmount the device in the file manager before opening it in mtp-cull.
2. Add a narrowly scoped udev rule for each supported vendor, using `TAG+="uaccess"`. Do not use an unrestricted `MODE="0666"` USB rule.
3. Reconnect the device after installing or reloading the udev rule.

For NixOS, place the equivalent rule in `services.udev.extraRules`. The appropriate vendor IDs depend on the devices you choose to support; validate Pixel and Fujifilm rules against the output of `lsusb`.

The first hardware validation targets are a Google Pixel and a Fujifilm camera in MTP mode. Test source-folder enumeration, JPEG preview caching, JPEG+RAW pairing, Keep-only import, and disconnect recovery.

## Known Limits

- Recursive MTP enumeration is intentionally manual for Android compatibility and may take time on large phone trees.
- Linux support currently covers the direct UI workflow. The older `list`, `list-content`, and `copy` CLI MTP commands still use the Windows-only backend.
- Flatpak support requires an explicit raw USB permission such as `--device=usb`; it is not packaged yet.
