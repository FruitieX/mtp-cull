param(
    [switch]$SkipBuild,
    [switch]$Test,
    [string]$IsccPath,
    [string]$RedistDirectory,
    [string]$LicenseBundler
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/..").Path
if ($env:OS -ne 'Windows_NT') { throw 'Windows packaging must run on Windows.' }

if (-not $IsccPath) {
    $compiler = Get-Command ISCC -ErrorAction SilentlyContinue
    if ($compiler) { $IsccPath = $compiler.Source }
    foreach ($candidate in @(
        "$env:ProgramFiles/Inno Setup 7/ISCC.exe",
        "${env:ProgramFiles(x86)}/Inno Setup 7/ISCC.exe",
        "${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe",
        "$env:USERPROFILE/scoop/apps/inno-setup/current/ISCC.exe",
        (Join-Path $repo 'target/tools/inno/compiler/{app}/ISCC.exe')
    )) {
        if (-not $IsccPath -and (Test-Path -LiteralPath $candidate)) { $IsccPath = $candidate }
    }
}
if (-not $IsccPath -or -not (Test-Path -LiteralPath $IsccPath)) {
    throw 'Install Inno Setup 6.3+ or 7, or pass -IsccPath pointing to ISCC.exe.'
}
if (-not $LicenseBundler) {
    $bundler = Get-Command cargo-bundle-licenses -ErrorAction SilentlyContinue
    if ($bundler) { $LicenseBundler = $bundler.Source }
    else { $LicenseBundler = Join-Path $repo 'target/tools/license-bundler/bin/cargo-bundle-licenses.exe' }
}
if (-not (Test-Path -LiteralPath $LicenseBundler)) {
    throw 'Install cargo-bundle-licenses 4.2.0 (see docs/windows-packaging.md), or pass -LicenseBundler.'
}

Push-Location $repo
try {
    if (-not $SkipBuild) {
        & "$PSScriptRoot/build-windows.ps1" -Test:$Test
    } elseif ($Test) { throw '-Test requires a fresh build; omit -SkipBuild.' }
    $metadataText = & cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $metadata = ($metadataText -join "`n") | ConvertFrom-Json
    $package = $metadata.packages | Where-Object { $_.name -eq 'mtp-cull' } | Select-Object -First 1
    $version = $package.version
    if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'Installer builds require a numeric major.minor.patch Cargo version.' }
    $binary = Join-Path $repo 'target/release/mtp-cull.exe'
    if (-not (Test-Path -LiteralPath $binary)) { throw 'Release binary missing.' }
    $info = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($binary)
    if ($info.ProductVersion -ne $version -or $info.ProductName -ne 'mtp-cull') {
        throw 'Binary version/resources do not match Cargo.toml; rebuild it before packaging.'
    }
    # Fail rather than labeling another architecture's executable as x64.
    $bytes = [System.IO.File]::ReadAllBytes($binary)
    $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
    if ([BitConverter]::ToUInt16($bytes, $peOffset + 4) -ne 0x8664) { throw 'Expected an x64 MSVC executable.' }

    if (-not $RedistDirectory) {
        $vswhere = "${env:ProgramFiles(x86)}/Microsoft Visual Studio/Installer/vswhere.exe"
        if (-not (Test-Path -LiteralPath $vswhere)) { throw 'vswhere missing; pass -RedistDirectory for the x64 Visual C++ CRT redist folder.' }
        $vsRoot = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if (-not $vsRoot) { throw 'Visual Studio C++ tools not found.' }
        $redistVersion = (Get-Content -LiteralPath (Join-Path $vsRoot 'VC/Auxiliary/Build/Microsoft.VCRedistVersion.default.txt')).Trim()
        $redistRoot = Join-Path $vsRoot "VC/Redist/MSVC/$redistVersion/x64"
        $RedistDirectory = (Get-ChildItem -LiteralPath $redistRoot -Directory -Filter 'Microsoft.VC*.CRT' | Select-Object -First 1).FullName
    }
    if (-not $RedistDirectory -or -not (Test-Path -LiteralPath (Join-Path $RedistDirectory 'vcruntime140.dll'))) {
        throw 'Redistributable x64 VC runtime DLLs not found.'
    }
    # Every invocation uses a fresh payload; no recursive deletion of computed paths.
    $stage = Join-Path $repo ("target/package-staging/" + [Guid]::NewGuid().ToString('N'))
    $payload = Join-Path $stage 'mtp-cull'
    $output = Join-Path $repo 'target/dist'
    New-Item -ItemType Directory -Force -Path $payload, $output | Out-Null
    Copy-Item -LiteralPath $binary -Destination $payload
    Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination $payload
    Copy-Item -LiteralPath (Join-Path $repo 'assets/icon.ico') -Destination $payload
    Get-ChildItem -LiteralPath $RedistDirectory -Filter '*.dll' | Copy-Item -Destination $payload
    Copy-Item -LiteralPath (Join-Path $repo 'packaging/windows/PORTABLE-README.txt') -Destination (Join-Path $payload 'README.txt')
    Copy-Item -LiteralPath (Join-Path $repo 'packaging/windows/RUNTIME-NOTICE.txt') -Destination $payload

    # Windows PowerShell treats redirected native stderr as ErrorRecords. Collect
    # the bundler's advisory messages without mistaking warnings for process failure.
    $savedPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & $LicenseBundler --features turbo --format json --output (Join-Path $payload 'THIRD-PARTY-LICENSES.json') 2> (Join-Path $stage 'license-collection.log')
        $licenseExit = $LASTEXITCODE
    } finally { $ErrorActionPreference = $savedPreference }
    if ($licenseExit -ne 0) { throw "License collection failed; see $stage/license-collection.log." }
    # libjpeg-turbo's native distribution includes IJG notices beyond Cargo metadata.
    $dependencyText = & cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc --features turbo
    if ($LASTEXITCODE -ne 0) { throw 'Dependency metadata failed.' }
    $dependencies = ($dependencyText -join "`n") | ConvertFrom-Json
    # Restrict the bundle to the Windows normal/build dependency graph, excluding
    # test-only and other platforms. Crate archives sometimes omit workspace
    # license files; reviewed, version-specific upstream supplements cover those.
    $nodes = @{}
    foreach ($node in $dependencies.resolve.nodes) { $nodes[$node.id] = $node }
    $seen = [System.Collections.Generic.HashSet[string]]::new()
    $pending = [System.Collections.Generic.Stack[string]]::new()
    $pending.Push($dependencies.resolve.root)
    while ($pending.Count -gt 0) {
        $id = $pending.Pop()
        if (-not $seen.Add($id)) { continue }
        foreach ($dependency in $nodes[$id].deps) {
            if (@($dependency.dep_kinds | Where-Object { $_.kind -ne 'dev' }).Count -gt 0) {
                $pending.Push($dependency.pkg)
            }
        }
    }
    $active = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($dependency in $dependencies.packages) {
        if ($seen.Contains($dependency.id)) { $active.Add("$($dependency.name)@$($dependency.version)") | Out-Null }
    }
    $bundlePath = Join-Path $payload 'THIRD-PARTY-LICENSES.json'
    $bundle = Get-Content -LiteralPath $bundlePath -Raw -Encoding UTF8 | ConvertFrom-Json
    $supplements = Get-Content -LiteralPath (Join-Path $repo 'packaging/windows/license-supplements.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    $bundle.third_party_libraries = @($bundle.third_party_libraries | Where-Object { $active.Contains("$($_.package_name)@$($_.package_version)") })
    foreach ($library in $bundle.third_party_libraries) {
        $supplement = $supplements.packages | Where-Object { $_.package_name -eq $library.package_name -and $_.package_version -eq $library.package_version } | Select-Object -First 1
        if ($supplement) {
            $library.license = $supplement.license
            $library.licenses = @($supplement.licenses | ForEach-Object {
                @{ license = $_.license; text = [System.IO.File]::ReadAllText((Join-Path $repo "packaging/windows/licenses/$($_.file)"), [System.Text.Encoding]::UTF8) }
            })
        }
        foreach ($notice in $library.licenses) {
            if (-not $notice.text -or $notice.text -eq 'NOT FOUND') {
                throw "Missing license text for $($library.package_name) $($library.package_version); update version-specific license supplements."
            }
        }
    }
    [System.IO.File]::WriteAllText($bundlePath, ($bundle | ConvertTo-Json -Depth 20), [System.Text.UTF8Encoding]::new($false))
    Copy-Item -LiteralPath (Join-Path $repo 'packaging/windows/licenses') -Destination (Join-Path $payload 'licenses') -Recurse
    Copy-Item -LiteralPath (Join-Path $repo 'packaging/windows/license-supplements.json') -Destination (Join-Path $payload 'licenses')
    $jpeg = $dependencies.packages | Where-Object { $_.name -eq 'turbojpeg-sys' } | Select-Object -First 1
    $jpegRoot = Join-Path (Split-Path $jpeg.manifest_path) 'libjpeg-turbo'
    $notices = Join-Path $payload 'licenses/libjpeg-turbo'
    New-Item -ItemType Directory -Force -Path $notices | Out-Null
    foreach ($name in @('LICENSE.md', 'README.ijg')) {
        Copy-Item -LiteralPath (Join-Path $jpegRoot $name) -Destination $notices
    }

    & $IsccPath '/Qp' "/DAppVersion=$version" "/DPayloadDir=$payload" "/DOutputDir=$output" "/DRepoDir=$repo" (Join-Path $repo 'packaging/windows/installer.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed.' }
    $zip = Join-Path $output "mtp-cull-$version-windows-x64-portable.zip"
    Compress-Archive -LiteralPath $payload -DestinationPath $zip -Force
    $installer = Join-Path $output "mtp-cull-$version-windows-x64-setup.exe"
    $checksums = @($installer, $zip) | ForEach-Object {
        $hash = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $(Split-Path $_ -Leaf)"
    }
    [System.IO.File]::WriteAllLines((Join-Path $output 'SHA256SUMS.txt'), $checksums, [System.Text.UTF8Encoding]::new($false))
    Write-Host "Installer: $installer"
    Write-Host "Portable:  $zip"
} finally { Pop-Location }
