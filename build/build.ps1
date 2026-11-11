# Baut claude-tui im Release-Modus und legt das Binary in output\ ab.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

Push-Location (Join-Path $root 'source')
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build fehlgeschlagen ($LASTEXITCODE)" }
} finally {
    Pop-Location
}

$out = Join-Path $root 'output'
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item (Join-Path $root 'source\target\release\claude-tui.exe') $out -Force
Write-Host "Fertig: $out\claude-tui.exe"
