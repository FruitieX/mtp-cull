mtp-cull for Windows x64

Extract the entire folder, then double-click mtp-cull.exe to open the UI.
Keep the bundled DLLs beside the executable. Rust, CMake and Visual Studio
are build tools; they are not needed to run this package.

For command-line copying, open PowerShell in this folder and run:
  ./mtp-cull.exe --help
  ./mtp-cull.exe copy --help

Review decisions and cached camera photos stay in your Windows user AppData.
Import presets start empty and live separately in:
  %APPDATA%\fruit\mtp-cull\config\import-presets.json
Moving/extracting this folder does not move or reset those settings. Portable
refers to the program files; user data is still stored in AppData by default.

Source, instructions and updates:
  https://github.com/FruitieX/mtp-cull

LICENSE covers mtp-cull. THIRD-PARTY-LICENSES.json and licenses/ contain
dependency notices. RUNTIME-NOTICE.txt covers the bundled Microsoft runtime.
