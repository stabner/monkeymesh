# Smoke: node + orchestrator + GPU worker → auto research → soft envelopes auto-apply.
# No marketplace submit. No manual research enqueue. No GUI vote.
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")

$nodeExe = Join-Path $Root "target\release\mesh-node.exe"
$orchExe = Join-Path $Root "target\release\mesh-orchestrator.exe"
$workerExe = Join-Path $Root "target\release\mesh-gpu-worker.exe"
if (-not (Test-Path $nodeExe)) { $nodeExe = Join-Path $Root "Launchers\Node\bin\mesh-node.exe" }
if (-not (Test-Path $orchExe)) { $orchExe = Join-Path $Root "Launchers\Orchestrator\bin\mesh-orchestrator.exe" }
if (-not (Test-Path $workerExe)) { $workerExe = Join-Path $Root "Launchers\Orchestrator\bin\mesh-gpu-worker.exe" }
foreach ($p in @($nodeExe, $orchExe, $workerExe)) {
    if (-not (Test-Path $p)) { throw "missing $p - build release first" }
}

$data = Join-Path $env:TEMP ("mm_auto_" + [guid]::NewGuid().ToString("n"))
New-Item -ItemType Directory -Force -Path $data | Out-Null
$rpc = "http://127.0.0.1:18084"
$orch = "http://127.0.0.1:18104"

Write-Host "==> start node"
$node = Start-Process -FilePath $nodeExe -ArgumentList @(
    "--chain", (Join-Path $data "chain.bin"),
    "serve", "--listen", "127.0.0.1:39114",
    "--rpc", "127.0.0.1:18084",
    "--wallet", (Join-Path $data "wallet.key"),
    "--p2p-key", (Join-Path $data "p2p.key"),
    "--miner-key", (Join-Path $data "wallet.key")
) -PassThru -WindowStyle Hidden

Write-Host "==> start orchestrator (research tick)"
$env:MESH_ORCH_BIND = "127.0.0.1:18104"
$env:MESH_NODE_RPC = $rpc
$env:MESH_ORCH_REQUIRE_NODE = "1"
$env:MESH_SETTLE = "0"
$orchProc = Start-Process -FilePath $orchExe -PassThru -WindowStyle Hidden

function Wait-Http([string]$Url, [int]$Tries = 40) {
    for ($i = 0; $i -lt $Tries; $i++) {
        try { Invoke-RestMethod $Url -TimeoutSec 2 | Out-Null; return }
        catch { Start-Sleep -Seconds 1 }
    }
    throw "timeout waiting for $Url"
}

try {
    Wait-Http "$rpc/v1/getnodeinfo"
    Wait-Http "$orch/v1/health"

    $health = Invoke-RestMethod "$orch/v1/health"
    if (-not $health.adaptive_research) { throw "orchestrator missing adaptive_research flag" }

    Write-Host "==> start GPU worker (no manual enqueue)"
    $worker = Start-Process -FilePath $workerExe -ArgumentList @(
        "--orch", $orch, "--jobs", "8", "--poll-ms", "200",
        "--keyfile", (Join-Path $data "gpu-worker.key")
    ) -PassThru -WindowStyle Hidden

    $ok = $false
    for ($i = 0; $i -lt 90; $i++) {
        Start-Sleep -Milliseconds 500
        $st = Invoke-RestMethod "$orch/v1/research/status"
        $env = Invoke-RestMethod "$rpc/v1/envelopes"
        $evalOk = [int]$st.protocol_eval_ok
        $autoId = [string]$env.last_auto_adapt_proposal_id
        if ($evalOk -ge 3 -and $autoId.Length -gt 0) {
            Write-Host "==> research status"
            $st | ConvertTo-Json -Depth 4
            Write-Host "==> envelopes (auto-adapted)"
            $env | ConvertTo-Json -Depth 5
            $epoch = [int64]$env.param_epoch
            if ($epoch -lt 1) { throw "expected param_epoch >= 1 after auto-adapt" }
            Write-Host "    param_epoch=$epoch"
            $ok = $true
            break
        }
        if (($i % 10) -eq 0) {
            Write-Host "    waiting… protocol_eval_ok=$evalOk auto='$autoId'"
        }
    }
    if (-not $ok) { throw "auto research / soft auto-adapt did not complete in time" }

    $pulse = Invoke-RestMethod "$rpc/v1/meshpulse"
    Write-Host "    pulse research_eval=$($pulse.markets.research_eval_receipts) progress=$($pulse.markets.research_progress)"

    if ($worker -and -not $worker.HasExited) {
        Stop-Process -Id $worker.Id -Force -ErrorAction SilentlyContinue
    }
    Write-Host "OK - self-adaptive mining loop works (no marketplace, no votes)"
} finally {
    if ($orchProc -and -not $orchProc.HasExited) { Stop-Process -Id $orchProc.Id -Force -ErrorAction SilentlyContinue }
    if ($node -and -not $node.HasExited) { Stop-Process -Id $node.Id -Force -ErrorAction SilentlyContinue }
}
