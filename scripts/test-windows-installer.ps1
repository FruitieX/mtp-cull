param([string]$IsccPath)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/..").Path
if (-not $IsccPath) {
    $compiler = Get-Command ISCC -ErrorAction SilentlyContinue
    if ($compiler) { $IsccPath = $compiler.Source }
    else {
        foreach ($candidate in @("$env:ProgramFiles/Inno Setup 7/ISCC.exe", "${env:ProgramFiles(x86)}/Inno Setup 7/ISCC.exe", "${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe", (Join-Path $repo 'target/tools/inno/compiler/{app}/ISCC.exe'))) {
            if (Test-Path -LiteralPath $candidate) { $IsccPath = $candidate; break }
        }
    }
}
if (-not $IsccPath) { throw 'Pass -IsccPath pointing to the Inno Setup compiler.' }
Push-Location $repo
try {
    $metadata = (cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $version = ($metadata.packages | Where-Object { $_.name -eq 'mtp-cull' } | Select-Object -First 1).version
    $identity = 'mtp-cull-packaging-test-' + [Guid]::NewGuid().ToString('N')
    $sandbox = Join-Path $repo "target/installer-tests/$identity"
    New-Item -ItemType Directory -Force -Path $sandbox | Out-Null
    Expand-Archive -LiteralPath (Join-Path $repo "target/dist/mtp-cull-$version-windows-x64-portable.zip") -DestinationPath (Join-Path $sandbox 'portable')
    $payload = Join-Path $sandbox 'portable/mtp-cull'
    $installDir = Join-Path $sandbox 'installed'
    $setup = Join-Path $sandbox "mtp-cull-$version-windows-x64-setup.exe"
    # Compile the actual installer rules with a separate test identity/shortcut,
    # so an existing user installation and its uninstall registration stay intact.
    & $IsccPath '/Qp' "/DAppVersion=$version" "/DPayloadDir=$payload" "/DOutputDir=$sandbox" "/DRepoDir=$repo" "/DPackageAppId=$identity" "/DShortcutName=$identity" (Join-Path $repo 'packaging/windows/installer.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Test installer compilation failed.' }
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/NOCLOSEAPPLICATIONS', "/DIR=`"$installDir`"", "/LOG=`"$(Join-Path $sandbox 'install.log')`"")
    $process = Start-Process -FilePath $setup -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installation failed ($($process.ExitCode)); see $sandbox/install.log. Close running mtp-cull instances normally before testing." }
    $binary = Join-Path $installDir 'mtp-cull.exe'
    if (-not (Test-Path -LiteralPath $binary)) { throw 'Installed executable missing.' }
    $shell = New-Object -ComObject WScript.Shell
    $shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) "$identity.lnk"
    if ($shell.CreateShortcut($shortcutPath).TargetPath -ne $binary) { throw 'Start-menu shortcut points to the wrong executable.' }
    $desktopPath = Join-Path ([Environment]::GetFolderPath('Desktop')) "$identity.lnk"
    if (Test-Path -LiteralPath $desktopPath) { throw 'Desktop shortcut should be opt-in.' }
    $registration = "HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/${identity}_is1"
    if (-not (Test-Path -LiteralPath $registration)) { throw 'Per-user uninstall registration missing.' }
    # Represent user-managed settings inside the installation as well as outside
    # it: uninstall must delete only files the installer actually owns.
    $sentinel = Join-Path $installDir 'user-presets-do-not-remove.json'
    [System.IO.File]::WriteAllText($sentinel, '{"presets":[]}', [System.Text.UTF8Encoding]::new($false))
    $userData = Join-Path $sandbox 'user-data'
    New-Item -ItemType Directory -Force -Path $userData | Out-Null
    $review = Join-Path $userData 'review-sentinel.txt'
    [System.IO.File]::WriteAllText($review, 'saved review choices')
    $binaryHash = (Get-FileHash -LiteralPath $binary).Hash
    # Repeat installation through the same identity to exercise upgrade behavior.
    $process = Start-Process -FilePath $setup -ArgumentList ($arguments + '/TASKS=desktopicon') -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0 -or (Get-FileHash -LiteralPath $binary).Hash -ne $binaryHash) { throw 'Upgrade failed.' }
    if (-not (Test-Path -LiteralPath $desktopPath) -or $shell.CreateShortcut($desktopPath).TargetPath -ne $binary) { throw 'Opt-in desktop shortcut missing or incorrect.' }
    if (-not (Test-Path -LiteralPath $sentinel)) { throw 'Upgrade removed an unmanaged user file.' }
    & $binary --version
    if ($LASTEXITCODE -ne 0) { throw 'Installed application cannot launch.' }
    $uninstaller = Join-Path $installDir 'unins000.exe'
    if (-not (Test-Path -LiteralPath $uninstaller)) { throw 'Uninstaller missing.' }
    # Only invoke an uninstaller inside this fresh workspace-owned sandbox.
    $resolved = (Resolve-Path -LiteralPath $uninstaller).Path
    if (-not $resolved.StartsWith((Resolve-Path -LiteralPath $sandbox).Path + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Uninstaller escaped its test sandbox.'
    }
    $process = Start-Process -FilePath $resolved -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw 'Uninstall failed.' }
    if ((Test-Path -LiteralPath $binary) -or (Test-Path -LiteralPath $shortcutPath) -or (Test-Path -LiteralPath $desktopPath) -or (Test-Path -LiteralPath $registration)) { throw 'Uninstall left program/shortcut/registration behind.' }
    if (-not (Test-Path -LiteralPath $sentinel) -or (Get-Content -LiteralPath $review -Raw) -ne 'saved review choices') { throw 'Uninstall changed user data.' }
    [System.IO.File]::WriteAllText((Join-Path $sandbox 'PASS.txt'), 'PASS: isolated per-user install, shortcut, upgrade, CLI, uninstall and user-data preservation.')
    Write-Host "PASS: installer lifecycle; evidence in $sandbox"
} finally { Pop-Location }
