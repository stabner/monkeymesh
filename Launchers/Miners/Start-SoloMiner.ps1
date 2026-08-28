# Offline / lab solo miner — uses its OWN chain file (never the Node chain)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$exe = Join-Path $PSScriptRoot "bin\mesh-miner-cpu.exe"
$key = Join-Path $PSScriptRoot "data\solo-miner.key"
$chain = Join-Path $PSScriptRoot "data\solo-chain.bin"

if (-not (Test-Path $exe)) {
    Write-Host "mesh-miner-cpu.exe missing. Run .\Launchers\build-release.ps1"
    Read-Host "Press Enter to exit"
    exit 1
}

New-Item -ItemType Directory -Force -Path (Join-Path $PSScriptRoot "data") | Out-Null

$configPath = Join-Path $PSScriptRoot "config.json"
$addressArg = @()
if (Test-Path $configPath) {
    $config = Get-Content $configPath -Raw | ConvertFrom-Json
    $address = "$($config.address)".Trim()
    if ($address) { $addressArg = @("--address", $address) }
}

Write-Host "MonkeyMesh Solo CPU Miner (offline lab)"
Write-Host "  chain: $chain"
if ($addressArg.Count -gt 0) { Write-Host "  address: $($addressArg[1])" }
Write-Host "  WARNING: This does NOT feed the Node. Prefer Start-CpuMiner.bat / Start-GpuMiner.bat in production."
Write-Host ""

& $exe --chain $chain --keyfile $key --blocks 0 @addressArg
$exit = $LASTEXITCODE
if ($exit -ne 0) {
    Write-Host "Solo miner exited with code $exit"
    Read-Host "Press Enter to close"
}
exit $exit
