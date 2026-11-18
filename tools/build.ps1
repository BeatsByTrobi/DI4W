# Builds the island and the settings app (Rust, release) for one architecture and lays them out the way they
# ship: the folder the zip contains and the installer is built from.
#
#   powershell -ExecutionPolicy Bypass -File tools/build.ps1 -Out artifacts/DI4W
#   powershell -ExecutionPolicy Bypass -File tools/build.ps1 -Arch arm64 -Out artifacts/DI4W-arm64
#
# Layout:
#   DynamicIsland.exe
#   Settings\DynamicIsland.Settings.exe
#   tools\island.ps1        (the Island API helper script)
#   LICENSE
#
# The C runtime is linked statically, so nothing else needs to be installed (no .NET, no VC++ redistributable).
# publish-release.ps1 and the CI build use this script.
param(
    [ValidateSet('x64', 'arm64')][string]$Arch = 'x64',
    [Parameter(Mandatory)][string]$Out
)
# Not 'Stop': Windows PowerShell 5.1 treats cargo's progress on stderr as an error. Exit codes are checked instead.
$ErrorActionPreference = 'Continue'
$root = Split-Path $PSScriptRoot -Parent
$triple = if ($Arch -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }

# The Rust target for the architecture (rustup installs the standard library for it once).
if (Get-Command rustup -ErrorAction SilentlyContinue) {
    if (-not (rustup target list --installed | Where-Object { $_ -eq $triple })) {
        rustup target add $triple
        if ($LASTEXITCODE -ne 0) { throw "Could not add the Rust target $triple (rustup target add $triple)." }
    }
}

$previousFlags = $env:RUSTFLAGS
$env:RUSTFLAGS = (@($previousFlags, '-C target-feature=+crt-static') | Where-Object { $_ }) -join ' '
try {
    cargo build --release --target $triple -p di4w-island -p di4w-settings --manifest-path (Join-Path $root 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) {
        $hint = if ($Arch -eq 'arm64') { ' Building for Arm64 needs the "MSVC ARM64 build tools" from the Visual Studio Installer.' } else { '' }
        throw "The build failed ($Arch).$hint"
    }
} finally {
    $env:RUSTFLAGS = $previousFlags
}

$built = Join-Path $root "target\$triple\release"
if (-not [IO.Path]::IsPathRooted($Out)) { $Out = Join-Path (Get-Location) $Out }
if (Test-Path $Out) { Remove-Item -LiteralPath $Out -Recurse -Force -ErrorAction Stop }
New-Item -ItemType Directory -Force (Join-Path $Out 'Settings'), (Join-Path $Out 'tools') | Out-Null

# Cargo can't name a binary "DynamicIsland.Settings" (no dots), so it's renamed here; the island looks for
# Settings\DynamicIsland.Settings.exe next to itself.
Copy-Item (Join-Path $built 'DynamicIsland.exe') (Join-Path $Out 'DynamicIsland.exe') -ErrorAction Stop
Copy-Item (Join-Path $built 'DynamicIsland-Settings.exe') (Join-Path $Out 'Settings\DynamicIsland.Settings.exe') -ErrorAction Stop
Copy-Item (Join-Path $root 'tools\island.ps1') (Join-Path $Out 'tools\island.ps1') -ErrorAction Stop
Copy-Item (Join-Path $root 'LICENSE') (Join-Path $Out 'LICENSE') -ErrorAction Stop
Write-Host "Built $Arch into $Out"
