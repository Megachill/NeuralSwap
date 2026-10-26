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
# Only what pack-release writes; anything else kept in release\ stays.
$generated = @('NeuralSwap-Setup.exe', 'NeuralSwap.exe', 'NeuralSwap-payload.zip', 'update.json',
    'private\NeuralSwap-Setup-Full.exe', 'private\NeuralSwap-payload.zip', 'private\NeuralSwap-dlss5-core.zip')
foreach ($f in $generated) { Remove-Item -Force (Join-Path $out $f) -ErrorAction SilentlyContinue }
# NeuralSwap.exe is a GUI-subsystem program, so PowerShell would not wait
# for it on its own.
$packArgs = @('pack-release', "`"$out`"", '--notes', "`"$Notes`"")
if ($NoPayload) { $packArgs += '--no-payload' }
Start-Process -FilePath $exe -ArgumentList $packArgs -Wait -NoNewWindow
if (-not (Test-Path (Join-Path $out 'update.json'))) { throw "pack-release did not finish (no update.json)" }
Write-Host "`nUpload the files in $out (not the private folder) to a GitHub release tagged v<version>."
