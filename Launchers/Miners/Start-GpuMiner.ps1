# MonkeyMesh NVIDIA GPU miner — CUDA mix + Node RPC
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$config = Get-Content (Join-Path $PSScriptRoot "config.json") -Raw | ConvertFrom-Json
$miner = Join-Path $PSScriptRoot "bin\mesh-miner-gpu.exe"
$cli = Join-Path $PSScriptRoot "bin\mesh-wallet-cli.exe"
$key = Join-Path $PSScriptRoot $config.wallet_key

if (-not (Test-Path $miner)) {
    Write-Host "mesh-miner-gpu.exe missing in Miners\bin\"
    Write-Host "Run .\Launchers\build-release.ps1 from the repo root (needs NVIDIA CUDA / nvcc)."
    Read-Host "Press Enter to exit"
    exit 1
}

New-Item -ItemType Directory -Force -Path (Join-Path $PSScriptRoot "data") | Out-Null

$address = "$($config.address)".Trim()
if (-not $address) {
    if (-not (Test-Path $cli)) {
        Write-Host "Set config.json `"address`" to your wallet address."
        Read-Host "Press Enter to exit"
        exit 1
    }
    if (-not (Test-Path $key)) {
        Write-Host "Creating miner wallet key at $key"
        & $cli --wallet $key --rpc $config.rpc new
    }
    $address = (& $cli --wallet $key --rpc $config.rpc address).Trim()
}

$device = 0
if ($null -ne $config.device) { $device = [int]$config.device }
$batch = 256
if ($null -ne $config.batch) { $batch = [int]$config.batch }

Write-Host "MonkeyMesh NVIDIA GPU Miner"
Write-Host "  rpc    : $($config.rpc)"
Write-Host "  address: $address"
Write-Host "  device : $device"
Write-Host "  batch  : $batch"
Write-Host "  blocks : $($config.blocks)  (0 = until Ctrl+C)"
Write-Host "  (Node must be running; CUDA optional — falls back to CPU mix)"
Write-Host ""

# B4: pick up sticky AI token (env → local data → Node data)
if (-not $env:MESH_AI_TOKEN -or -not "$($env:MESH_AI_TOKEN)".Trim()) {
    $candidates = @(
        (Join-Path $PSScriptRoot "data\ai.token"),
        (Join-Path $PSScriptRoot "..\Node\data\ai.token"),
        (Join-Path $PSScriptRoot "..\..\Launchers\Node\data\ai.token")
    )
    foreach ($p in $candidates) {
        if (Test-Path $p) {
            $tok = (Get-Content -Raw $p).Trim()
            if ($tok) {
                $env:MESH_AI_TOKEN = $tok
                Write-Host "  ai tok : loaded from $p"
                break
            }
        }
    }
}

$argsList = @(
    "--rpc", $config.rpc,
    "--address", $address,
    "--blocks", "$($config.blocks)",
    "--max-nonces", "$($config.max_nonces)",
    "--device", "$device",
    "--batch", "$batch"
)

try {
    & $miner @argsList
    if ($LASTEXITCODE -ne 0) { throw "miner exited $LASTEXITCODE" }
} catch {
    Write-Host ""
    Write-Host $_
    Read-Host "Press Enter to close"
    exit 1
}
