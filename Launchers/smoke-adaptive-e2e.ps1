# Full adaptive smoke: start ephemeral node if needed, propose/activate, optional worker.
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$rpc = if ($env:MESH_NODE_RPC) { $env:MESH_NODE_RPC.TrimEnd('/') } else { "http://127.0.0.1:18080" }

function Test-Rpc {
    try {
        Invoke-RestMethod "$rpc/v1/getnodeinfo" -TimeoutSec 2 | Out-Null
        return $true
    } catch { return $false }
}

$started = $false
$proc = $null
$data = Join-Path $env:TEMP ("mm_smoke_" + [guid]::NewGuid().ToString("n"))
New-Item -ItemType Directory -Force -Path $data | Out-Null

if (-not (Test-Rpc)) {
    $exe = Join-Path $Root "target\release\mesh-node.exe"
    if (-not (Test-Path $exe)) { $exe = Join-Path $Root "Launchers\Node\bin\mesh-node.exe" }
    if (-not (Test-Path $exe)) { throw "mesh-node.exe not found - build release first" }
    Write-Host "==> starting ephemeral node in $data"
    $proc = Start-Process -FilePath $exe -ArgumentList @(
        "--chain", (Join-Path $data "chain.bin"),
        "serve",
        "--listen", "127.0.0.1:39111",
        "--rpc", "127.0.0.1:18080",
        "--wallet", (Join-Path $data "wallet.key"),
        "--p2p-key", (Join-Path $data "p2p.key"),
        "--miner-key", (Join-Path $data "wallet.key")
    ) -PassThru -WindowStyle Hidden
    $started = $true
    $ok = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Seconds 1
        if (Test-Rpc) { $ok = $true; break }
    }
    if (-not $ok) { throw "node RPC did not come up" }
}

try {
    & (Join-Path $PSScriptRoot "smoke-adaptive.ps1")

    $orch = "http://127.0.0.1:18100"
    try {
        Invoke-RestMethod "$orch/v1/health" -TimeoutSec 2 | Out-Null
        Write-Host "==> orchestrator up - enqueue echo (run worker separately if desired)"
        Invoke-RestMethod -Method Post "$orch/v1/enqueue" -ContentType "application/json" -Body (@{ kind = "echo" } | ConvertTo-Json) | Out-Null
        Write-Host "    worker: Launchers\Orchestrator\bin\mesh-gpu-worker.exe --jobs 1"
    } catch {
        Write-Host "==> orchestrator not running (skip worker check)"
    }
} finally {
    if ($started -and $proc -and -not $proc.HasExited) {
        Write-Host "==> stopping ephemeral node"
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    }
}

Write-Host "DONE"
