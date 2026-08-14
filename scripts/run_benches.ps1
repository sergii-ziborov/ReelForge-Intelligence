# Run Intelligence pipeline benches and print Criterion summary lines.
# Usage: pwsh scripts/run_benches.ps1

$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

Write-Host "=== reelforge-intelligence-core pipeline benches ==="
cargo bench -p reelforge-intelligence-core --bench pipeline -- --warm-up-time 1 --measurement-time 2 --sample-size 30 2>&1 |
  Select-String -Pattern "time:|Benchmarking|error|failed|Finished|resolve/|compile/|bridge/|pipeline/|mask_timeline/|serde/"
