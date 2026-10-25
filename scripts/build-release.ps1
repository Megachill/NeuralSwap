# Builds a shareable release into .\release:
#   NeuralSwap-Setup.exe, NeuralSwap.exe, NeuralSwap-payload.zip, update.json
#
#   .\scripts\build-release.ps1 -Repo owner/NeuralSwap -Notes "What changed" [-NoPayload]
#
# -Repo is baked in as the auto-update source. Local paths (user profile,
# cargo home, project folder) are remapped so no personal path ends up in
# the executable.
param(
    [Parameter(Mandatory = $true)][string]$Repo,
    [string]$Notes = "",
    # Publish without the DLSS 5 files. A full setup for private sharing is
    # still written to release\private (never upload that folder).
    [switch]$NoPayload
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }

$env:NEURALSWAP_REPO = $Repo
$env:RUSTFLAGS = "--remap-path-prefix=$cargoHome=cargo --remap-path-prefix=$root=neuralswap --remap-path-prefix=$($env:USERPROFILE)=home"
cargo build --release --manifest-path "$root\Cargo.toml"
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$exe = Join-Path $root 'target\release\NeuralSwap.exe'
$text = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($exe))
foreach ($p in @($env:USERPROFILE, $cargoHome, $root)) {
    if ($text.IndexOf($p, [StringComparison]::OrdinalIgnoreCase) -ge 0) { Write-Warning "The executable still contains the path '$p' - check before publishing." }
}

$out = Join-Path $root 'release'
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
if ($NoPayload) { & $exe pack-release $out --notes $Notes --no-payload } else { & $exe pack-release $out --notes $Notes }
Write-Host "`nUpload the files in $out (not the private folder) to a GitHub release tagged v<version>."
