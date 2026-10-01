param([string]$DistDirectory)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/..").Path
if (-not $DistDirectory) { $DistDirectory = Join-Path $repo 'target/dist' }
Push-Location $repo
try {
    $metadata = (cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $version = ($metadata.packages | Where-Object { $_.name -eq 'mtp-cull' } | Select-Object -First 1).version
    $installer = Join-Path $DistDirectory "mtp-cull-$version-windows-x64-setup.exe"
    $zip = Join-Path $DistDirectory "mtp-cull-$version-windows-x64-portable.zip"
    if (-not (Test-Path -LiteralPath $installer)) { throw 'Installer missing.' }
    $scratch = Join-Path $repo ("target/package-verification/" + [Guid]::NewGuid().ToString('N'))
    Expand-Archive -LiteralPath $zip -DestinationPath $scratch
    $payload = Join-Path $scratch 'mtp-cull'
    $binary = Join-Path $payload 'mtp-cull.exe'
    $info = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($binary)
    if ($info.ProductVersion -ne $version -or $info.ProductName -ne 'mtp-cull') { throw 'Executable metadata mismatch.' }
    foreach ($file in @('vcruntime140.dll', 'vcruntime140_1.dll', 'LICENSE', 'README.txt', 'THIRD-PARTY-LICENSES.json', 'licenses/libjpeg-turbo/LICENSE.md', 'licenses/libjpeg-turbo/README.ijg')) {
        if (-not (Test-Path -LiteralPath (Join-Path $payload $file))) { throw "Package file missing: $file" }
    }
    $notices = Get-Content -LiteralPath (Join-Path $payload 'THIRD-PARTY-LICENSES.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($notices.third_party_libraries.Count -lt 1) { throw 'Dependency license bundle is empty.' }
    foreach ($library in $notices.third_party_libraries) {
        foreach ($license in $library.licenses) {
            if (-not $license.text -or $license.text -eq 'NOT FOUND') { throw "License text missing: $($library.package_name)" }
        }
    }
    if (-not ('MtpCullPackageIcons' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class MtpCullPackageIcons {
    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    public static extern uint ExtractIconEx(string file, int index, IntPtr[] large, IntPtr[] small, uint count);
    [DllImport("user32.dll")]
    public static extern bool DestroyIcon(IntPtr icon);
}
'@
    }
    foreach ($file in @($binary, $installer)) {
        if ([MtpCullPackageIcons]::ExtractIconEx($file, -1, $null, $null, 0) -lt 1) { throw "Embedded icon missing: $file" }
    }
    $large = [IntPtr[]]::new(1)
    $small = [IntPtr[]]::new(1)
    if ([MtpCullPackageIcons]::ExtractIconEx($binary, 0, $large, $small, 1) -lt 1) { throw 'Cannot load executable icon.' }
    foreach ($handle in @($large[0], $small[0])) { if ($handle -ne [IntPtr]::Zero) { [MtpCullPackageIcons]::DestroyIcon($handle) | Out-Null } }
    foreach ($command in @('--help', '--version')) {
        $text = (& $binary $command) -join "`n"
        if ($LASTEXITCODE -ne 0 -or $text -notmatch 'mtp-cull') { throw "Packaged CLI failed: $command" }
    }
    $invalidOutput = Join-Path $scratch 'invalid-command.log'
    $process = Start-Process -FilePath $binary -ArgumentList 'invalid-subcommand' -WindowStyle Hidden -Wait -PassThru -RedirectStandardError $invalidOutput
    if ($process.ExitCode -eq 0 -or -not (Select-String -LiteralPath $invalidOutput -SimpleMatch 'unrecognized subcommand')) { throw 'CLI errors must retain nonzero exit status and stderr.' }
    foreach ($line in Get-Content -LiteralPath (Join-Path $DistDirectory 'SHA256SUMS.txt')) {
        $hash, $name = $line -split '  ', 2
        if ((Get-FileHash -LiteralPath (Join-Path $DistDirectory $name) -Algorithm SHA256).Hash.ToLowerInvariant() -ne $hash) { throw "Checksum mismatch: $name" }
    }
    Write-Host "PASS: package $version, embedded icons/metadata, bundled runtime/notices, CLI output/errors and SHA256 checksums."
} finally { Pop-Location }
