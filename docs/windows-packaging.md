# Windows installation and packages

The Windows x64 distribution has an installer and a portable ZIP. Both contain
the SIMD JPEG decoder, embedded application icon/version information, app-local
Visual C++ runtime DLLs and dependency license notices. Users do not need Rust,
Visual Studio, CMake or NASM to run these packages. Windows 10 or newer is required.

## Install and launch

Run `mtp-cull-VERSION-windows-x64-setup.exe`. Installation is per user, without
administrator privileges, under `%LOCALAPPDATA%\Programs\mtp-cull` by default.
The installer adds a Start-menu shortcut, an optional desktop shortcut and a
Windows uninstall entry. Double-clicking the application opens the UI. The window,
taskbar and shortcuts use the same camera icon and application identity.

The executable retains console support for CLI commands. UI launch detaches a
console created for it by Explorer; an existing terminal remains attached. A
brief console startup flash is possible. UI startup errors after detachment appear
in a Windows message box.

Close running reviews/imports normally before upgrading or uninstalling. The
installer checks the application's lifetime mutex rather than terminating it.
Upgrades and uninstall preserve user configuration, review decisions and staged
photos in their existing AppData locations. Import presets remain user-owned and
start empty; the installer does not supply any.

For the portable distribution, extract the entire ZIP and launch `mtp-cull.exe`
inside the `mtp-cull` folder. Keep the included DLLs beside it. "Portable" describes
the program files: configuration and caches still use the normal user directories.
For isolated development, `MTP_CULL_DATA_DIR` redirects application data.

## Build packages

Use Windows with the repository's pinned Rust MSVC toolchain, Visual Studio C++
build tools, CMake and NASM, as for `scripts/build-windows.ps1`. Additionally install
[Inno Setup](https://jrsoftware.org/isinfo.php) 6.3+ or 7 and the license collector:

```powershell
cargo install cargo-bundle-licenses --version 4.2.0 --locked
./scripts/package-windows.ps1 -Test
./scripts/verify-windows-package.ps1
./scripts/test-windows-installer.ps1
```

Packaging builds a fresh production release with `turbo`, runs tests/strict Clippy
when `-Test` is set, and emits these files under `target/dist`:

- `mtp-cull-VERSION-windows-x64-setup.exe`
- `mtp-cull-VERSION-windows-x64-portable.zip`
- `SHA256SUMS.txt`

Optional `-IsccPath`, `-LicenseBundler` and `-RedistDirectory` parameters select
tool locations. `-RedistDirectory` must name the x64 `Microsoft.VC*.CRT` directory
from the Visual Studio redistributable tree. Otherwise the script discovers it
through `vswhere`. Rebuild packages when updating that runtime. The runtime has
its own redistribution terms, included in `RUNTIME-NOTICE.txt`.

`-SkipBuild` packages an already-built production release; it cannot be combined
with `-Test`. Binary architecture, embedded version and product name must match
the package. Versions currently require numeric `major.minor.patch` in Cargo.toml.
Fresh staging directories under ignored `target/` avoid deleting previous build
results or user paths.

Package verification extracts a fresh ZIP and checks resources, runtime files,
license notices, CLI output/error behavior and checksums. Installer verification
compiles the actual installer rules with a unique test identity, installs inside
`target/installer-tests`, tests default/optional shortcuts and upgrade/uninstall,
and confirms unmanaged files survive. Its temporary registration and shortcuts
are removed on success; logs and evidence remain in the isolated directory.

## Branding and notices

`assets/icon.svg` is the editable source. The checked-in ICO contains sizes from
16 to 256 pixels; `icon.rgba` supplies the 64-pixel window icon without adding a
runtime PNG decoder. To regenerate all raster assets, install ImageMagick and run:

```powershell
./scripts/generate-icon.ps1
```

`build.rs` embeds the ICO and Cargo version/product metadata into Windows binaries.
Keep the stable installer AppId and `FruitieX.mtp-cull` application identity when
changing versions or branding, so upgrades and pinned shortcuts remain consistent.

The package includes the active Windows dependency graph's notices and native
libjpeg-turbo notices. Some crate archives omit workspace license files;
`packaging/windows/license-supplements.json` supplies version-specific upstream
texts with their source URLs, including embedded font licenses. Missing license
texts fail packaging. Dependency upgrades may require reviewing and updating these
supplements against the actual upstream version.

## Release automation and remaining validation

The **Windows packages** GitHub Actions workflow can be run manually to produce
downloadable artifacts. Pushing a tag such as `v0.1.0` requires the matching Cargo
version and also creates a **draft** GitHub release with the tested packages.
Published releases are never overwritten by this workflow. Publishing the draft
is a separate action; no release or tag is created by the local packaging script.

Current packages are unsigned; certificate-based signing is not configured.
Local verification covers installation, upgrade/uninstall, native UI launch,
embedded icons and CLI compatibility. A clean Windows machine test and execution
of the GitHub workflow remain separate validation steps.
