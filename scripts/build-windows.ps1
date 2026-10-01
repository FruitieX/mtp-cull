param([switch]$Test, [switch]$Smoke)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/..").Path
$originalPath = $env:PATH
$originalToolchain = $env:CMAKE_TOOLCHAIN_FILE
try {
    if (-not (Get-Command nasm -ErrorAction SilentlyContinue)) {
        $localNasm = Join-Path $repo 'target/tools/nasm/nasm-3.02'
        if (Test-Path -LiteralPath (Join-Path $localNasm 'nasm.exe')) {
            $env:PATH = "$localNasm;$env:PATH"
        } else { throw 'Install NASM and add it to PATH to build the SIMD decoder.' }
    }
    $env:CMAKE_TOOLCHAIN_FILE = Join-Path $repo 'build-support/turbo-windows.cmake'
    $features = if ($Smoke) { 'turbo,ui-smoke' } else { 'turbo' }
    Push-Location $repo
    try {
        if ($Test) {
            cargo test --locked --features $features
            if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
            cargo clippy --locked --all-targets --features $features -- -D warnings
            if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
        }
        cargo build --locked --release --features $features
        if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
    } finally { Pop-Location }
} finally {
    $env:PATH = $originalPath
    $env:CMAKE_TOOLCHAIN_FILE = $originalToolchain
}
