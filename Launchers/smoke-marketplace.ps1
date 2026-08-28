# Smoke: ephemeral node + orchestrator + GPU worker + marketplace job + MESH settle.
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

$data = Join-Path $env:TEMP ("mm_mkt_" + [guid]::NewGuid().ToString("n"))
New-Item -ItemType Directory -Force -Path $data | Out-Null
$rpc = "http://127.0.0.1:18082"
$orch = "http://127.0.0.1:18102"

Write-Host "==> start node (mine for settle funding)"
$node = Start-Process -FilePath $nodeExe -ArgumentList @(
    "--chain", (Join-Path $data "chain.bin"),
    "serve", "--listen", "127.0.0.1:39112",
    "--rpc", "127.0.0.1:18082",
    "--wallet", (Join-Path $data "wallet.key"),
    "--p2p-key", (Join-Path $data "p2p.key"),
    "--miner-key", (Join-Path $data "wallet.key"),
    "--mine"
) -PassThru -WindowStyle Hidden

Write-Host "==> start orchestrator (settle on)"
$env:MESH_ORCH_BIND = "127.0.0.1:18102"
$env:MESH_NODE_RPC = $rpc
$env:MESH_ORCH_REQUIRE_NODE = "1"
$env:MESH_SETTLE = "1"
$env:MESH_SETTLE_BASE_ATOMIC = "100000"
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

    Write-Host "==> wait for mature coinbase (height >= 22)"
    $ready = $false
    for ($i = 0; $i -lt 120; $i++) {
        $info = Invoke-RestMethod "$rpc/v1/getnodeinfo"
        if ([int]$info.height -ge 22) { $ready = $true; break }
        Start-Sleep -Milliseconds 500
    }
    if (-not $ready) { throw "chain did not reach height 22" }
    Write-Host "    height=$($info.height)"

    Write-Host "==> submit marketplace llm job"
    $sub = Invoke-RestMethod -Method Post "$orch/v1/marketplace/jobs" -ContentType "application/json" -Body (@{
        service = "llm"
        prompt = "MonkeyMesh settle hello"
    } | ConvertTo-Json)
    $id = $sub.job.id
    Write-Host "    job $id worker_job=$($sub.job.worker_job_id)"

    Write-Host "==> start worker (3 jobs max)"
    $worker = Start-Process -FilePath $workerExe -ArgumentList @(
        "--orch", $orch, "--jobs", "3", "--poll-ms", "200",
        "--keyfile", (Join-Path $data "gpu-worker.key")
    ) -PassThru -WindowStyle Hidden

    $done = $false
    for ($i = 0; $i -lt 60; $i++) {
        Start-Sleep -Milliseconds 500
        $st = Invoke-RestMethod "$orch/v1/marketplace/jobs/$id"
        if ($st.job.status -eq "done") {
            Write-Host "==> job done settle=$($st.job.settlement_status) amount=$($st.job.settlement_amount) txid=$($st.job.settlement_txid)"
            if ($st.job.settlement_status -ne "paid" -or -not $st.job.settlement_txid) {
                throw "expected paid settlement, got $($st.job.settlement_status) err=$($st.job.settlement_error)"
            }
            $done = $true
            break
        }
        if ($st.job.status -eq "failed") { throw "job failed: $($st.job.error)" }
    }
    if (-not $done) { throw "job not done in time" }

    Write-Host "==> gpu scores"
    Invoke-RestMethod "$rpc/v1/gpuscores" | ConvertTo-Json -Depth 5

    if ($worker -and -not $worker.HasExited) {
        Stop-Process -Id $worker.Id -Force -ErrorAction SilentlyContinue
    }
    Write-Host "OK - marketplace settle path works"
} finally {
    if ($orchProc -and -not $orchProc.HasExited) { Stop-Process -Id $orchProc.Id -Force -ErrorAction SilentlyContinue }
    if ($node -and -not $node.HasExited) { Stop-Process -Id $node.Id -Force -ErrorAction SilentlyContinue }
    Remove-Item Env:MESH_ORCH_BIND,Env:MESH_NODE_RPC,Env:MESH_ORCH_REQUIRE_NODE,Env:MESH_SETTLE,Env:MESH_SETTLE_BASE_ATOMIC -ErrorAction SilentlyContinue
}
