$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/..").Path
if (-not (Get-Command magick -ErrorAction SilentlyContinue)) {
    throw 'Install ImageMagick to regenerate the checked-in icon assets.'
}
$assets = Join-Path $repo 'assets'
& magick -background none (Join-Path $assets 'icon.svg') -resize 256x256 "PNG32:$(Join-Path $assets 'icon.png')"
if ($LASTEXITCODE -ne 0) { throw 'PNG rendering failed' }
& magick (Join-Path $assets 'icon.png') -define icon:auto-resize=256,128,96,64,48,32,24,16 (Join-Path $assets 'icon.ico')
if ($LASTEXITCODE -ne 0) { throw 'ICO generation failed' }
# Fixed-size RGBA pixels avoid adding a PNG decoder to normal JPEG-only builds.
& magick (Join-Path $assets 'icon.png') -resize 64x64 -depth 8 "RGBA:$(Join-Path $assets 'icon.rgba')"
if ($LASTEXITCODE -ne 0) { throw 'Window icon generation failed' }
