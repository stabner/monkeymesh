# Smoke: research scenario enqueue → GPU worker → deterministic verify → status.
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

$data = Join-Path $env:TEMP ("mm_research_" + [guid]::NewGuid().ToString("n"))
New-Item -ItemType Directory -Force -Path $data | Out-Null
$rpc = "http://127.0.0.1:18083"
$orch = "http://127.0.0.1:18103"

Write-Host "==> start node"
$node = Start-Process -FilePath $nodeExe -ArgumentList @(
    "--chain", (Join-Path $data "chain.bin"),
    "serve", "--listen", "127.0.0.1:39113",
    "--rpc", "127.0.0.1:18083",
    "--wallet", (Join-Path $data "wallet.key"),
    "--p2p-key", (Join-Path $data "p2p.key"),
    "--miner-key", (Join-Path $data "wallet.key")
) -PassThru -WindowStyle Hidden

Write-Host "==> start orchestrator"
$env:MESH_ORCH_BIND = "127.0.0.1:18103"
$env:MESH_NODE_RPC = $rpc
$env:MESH_ORCH_REQUIRE_NODE = "1"
$orchProc = Start-Process -FilePath $orchExe -PassThru -WindowStyle Hidden

function Wait-Http([string]$Url, [int]$Tries = 30) {
    for ($i = 0; $i -lt $Tries; $i++) {
        try { Invoke-RestMethod $Url -TimeoutSec 2 | Out-Null; return }
        catch { Start-Sleep -Seconds 1 }
    }
    throw "timeout waiting for $Url"
}

try {
    Wait-Http "$rpc/v1/getnodeinfo"
    Wait-Http "$orch/v1/health"

    Write-Host "==> research scenarios"
    $cat = Invoke-RestMethod "$orch/v1/research/scenarios"
    if (-not $cat.scenarios -or $cat.scenarios.Count -lt 8) { throw "expected expanded research catalog (>=8)" }

    Write-Host "==> enqueue scale_throughput"
    $enq = Invoke-RestMethod -Method Post "$orch/v1/research/enqueue" -ContentType "application/json" -Body (@{
        scenario = "scale_throughput"
        height = 12
        pulse_signal = 0.15
    } | ConvertTo-Json)
    $jobId = $enq.job_id
    Write-Host "    job $jobId scenario=$($enq.scenario)"

    Write-Host "==> start worker (2 jobs max)"
    $worker = Start-Process -FilePath $workerExe -ArgumentList @(
        "--orch", $orch, "--jobs", "2", "--poll-ms", "200",
        "--keyfile", (Join-Path $data "gpu-worker.key")
    ) -PassThru -WindowStyle Hidden

    $ok = $false
    for ($i = 0; $i -lt 40; $i++) {
        Start-Sleep -Milliseconds 500
        $st = Invoke-RestMethod "$orch/v1/research/status"
        if ([int]$st.protocol_eval_ok -ge 1 -and [int]$st.research_scenarios_touched -ge 1) {
            Write-Host "==> research status"
            $st | ConvertTo-Json -Depth 4
            $ok = $true
            break
        }
    }
    if (-not $ok) { throw "research job not verified in time" }

    Write-Host "==> gpu scores (research credit)"
    Invoke-RestMethod "$rpc/v1/gpuscores" | ConvertTo-Json -Depth 5

    Write-Host "==> meshpulse research fields"
    $pulse = Invoke-RestMethod "$rpc/v1/meshpulse"
    Write-Host "    research_eval_receipts=$($pulse.markets.research_eval_receipts) progress=$($pulse.markets.research_progress) primary=$($pulse.markets.research_scores.mean_primary)"
    if ([double]$pulse.markets.research_scores.mean_primary -le 0) {
        throw "expected score_primary on protocol_eval receipt via MeshPulse"
    }

    if ($worker -and -not $worker.HasExited) {
        Stop-Process -Id $worker.Id -Force -ErrorAction SilentlyContinue
    }
    Write-Host "OK - adaptive research path works (protocol sim v2)"
} finally {
    if ($orchProc -and -not $orchProc.HasExited) { Stop-Process -Id $orchProc.Id -Force -ErrorAction SilentlyContinue }
    if ($node -and -not $node.HasExited) { Stop-Process -Id $node.Id -Force -ErrorAction SilentlyContinue }
}
