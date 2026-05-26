param(
    [string]$Version = "",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..\..\..")
$firmwareDir = Join-Path $repoRoot "firmware\pico-hid"
$cargoToml = Join-Path $firmwareDir "Cargo.toml"
$elfPath = Join-Path $firmwareDir "target\thumbv6m-none-eabi\release\pico-hid"

if ([string]::IsNullOrWhiteSpace($Version)) {
    $versionLine = Select-String -Path $cargoToml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if ($null -eq $versionLine) {
        throw "Could not read firmware package version from $cargoToml"
    }
    $Version = $versionLine.Matches[0].Groups[1].Value
}

$elf2uf2 = Get-Command elf2uf2-rs -ErrorAction SilentlyContinue
if ($null -eq $elf2uf2) {
    throw "elf2uf2-rs is required. Install with: cargo install elf2uf2-rs"
}

if (-not $SkipBuild) {
    Push-Location $firmwareDir
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build --release failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $elfPath)) {
    throw "Firmware ELF was not found at $elfPath"
}

$outDir = Join-Path $repoRoot "scripts\release\firmware"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$uf2Path = Join-Path $outDir "pico-hid-$Version.uf2"

& $elf2uf2.Source $elfPath $uf2Path
if ($LASTEXITCODE -ne 0) {
    throw "elf2uf2-rs failed with exit code $LASTEXITCODE"
}

$uf2 = Get-Item -LiteralPath $uf2Path
$hash = Get-FileHash -LiteralPath $uf2Path -Algorithm SHA256

[PSCustomObject]@{
    FirmwareElf = $elfPath
    Uf2Path = $uf2.FullName
    Version = $Version
    Bytes = $uf2.Length
    Sha256 = $hash.Hash
}
