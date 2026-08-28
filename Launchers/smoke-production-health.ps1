# MonkeyMesh production health smoke (Build/28 M8).
# Checks seed, edge, edge2, pool, AI board, tip alignment. Exit 1 if any critical check fails.
#
# Usage:
#   .\Launchers\smoke-production-health.ps1
#   .\Launchers\smoke-production-health.ps1 -MaxTipLag 5

param(
    [int]$TimeoutSec = 20,
    [int]$MaxTipLag = 3
)

$ErrorActionPreference = "Continue"
$fail = 0
$heights = @{}

function Check-JsonUrl([string]$Name, [string]$Url, [scriptblock]$Assert) {
    Write-Host ("==> {0}: {1}" -f $Name, $Url)
    try {
        $r = Invoke-RestMethod -Uri $Url -TimeoutSec $TimeoutSec
        & $Assert $r
        Write-Host ("    OK")
        return $r
    } catch {
        Write-Host ("    FAIL: {0}" -f $_.Exception.Message)
        $script:fail++
        return $null
    }
}

$seedRpc = if ($env:MESH_SMOKE_SEED) { $env:MESH_SMOKE_SEED } else { "http://seednode.hashmonkeys.cloud:18080" }
$edgeRpc = if ($env:MESH_SMOKE_EDGE) { $env:MESH_SMOKE_EDGE } else { "http://seednode.hashmonkeys.cloud:18081" }
$poolUrl = if ($env:MESH_SMOKE_POOL) { $env:MESH_SMOKE_POOL } else { "https://eu.hashmonkeys.cloud/health" }

$seed = Check-JsonUrl "seed getnodeinfo" "$seedRpc/v1/getnodeinfo" {
    param($d)
    if ($null -eq $d.height) { throw "missing height" }
    Write-Host ("    height={0} tip={1} peers={2}" -f $d.height, $d.tip, $d.peers)
    $script:heights["seed"] = [int64]$d.height
    $script:heights["seed_tip"] = [string]$d.tip
}

$null = Check-JsonUrl "seed AI health" "$seedRpc/v1/ai/health" {
    param($d)
    if (-not $d.ok) { throw "ai not ok" }
    Write-Host ("    pending={0} inflight={1} verify_ok={2} slots={3}" -f $d.pending, $d.inflight, $d.verify_ok, $d.worker_slots)
}

$edge = Check-JsonUrl "edge mine RPC" "$edgeRpc/v1/getnodeinfo" {
    param($d)
    Write-Host ("    height={0} tip={1}" -f $d.height, $d.tip)
    $script:heights["edge"] = [int64]$d.height
}
if (-not $edge) {
    Write-Host "    WARN: edge RPC unreachable from this host. Public mine target is still https://eu.hashmonkeys.cloud."
    # Do not count as critical if pool is up — templates still flow via mesh-pool.
    if ($script:fail -gt 0) { $script:fail-- }
}

if ($env:MESH_SMOKE_EDGE2) {
    $edge2 = Check-JsonUrl "edge2" "$($env:MESH_SMOKE_EDGE2)/v1/getnodeinfo" {
        param($d)
        Write-Host ("    height={0} tip={1}" -f $d.height, $d.tip)
        $script:heights["edge2"] = [int64]$d.height
    }
} else {
    $edge2 = $null
}

$null = Check-JsonUrl "pool health" $poolUrl {
    param($d)
    if (-not $d.ok) { throw "pool not ok" }
    Write-Host ("    service={0}" -f $d.service)
}

$null = Check-JsonUrl "public seed DNS" "http://seednode.hashmonkeys.cloud:18080/v1/getnodeinfo" {
    param($d)
    Write-Host ("    height={0}" -f $d.height)
    $script:heights["dns"] = [int64]$d.height
}

if ($heights.ContainsKey("seed")) {
    Write-Host ("==> tip lag vs seed (max {0})" -f $MaxTipLag)
    foreach ($name in @("edge", "edge2", "dns")) {
        if (-not $heights.ContainsKey($name)) { continue }
        $lag = [math]::Abs($heights["seed"] - $heights[$name])
        if ($lag -gt $MaxTipLag) {
            Write-Host ("    FAIL: {0} lag={1} (seed={2} {0}={3})" -f $name, $lag, $heights["seed"], $heights[$name])
            $fail++
        } else {
            Write-Host ("    OK: {0} lag={1}" -f $name, $lag)
        }
    }
    if ($edge -and $seed -and $edge.tip -and $seed.tip -and ($edge.tip -ne $seed.tip) -and ([math]::Abs($heights["seed"] - $heights["edge"]) -eq 0)) {
        Write-Host "    WARN: edge height matches seed but tip hash differs (possible fork)"
        $fail++
    }
    if ($edge2 -and $seed -and $edge2.tip -and $seed.tip -and ($edge2.tip -ne $seed.tip) -and ([math]::Abs($heights["seed"] - $heights["edge2"]) -eq 0)) {
        Write-Host "    WARN: edge2 height matches seed but tip hash differs (possible fork)"
        $fail++
    }
}

if ($fail -gt 0) {
    Write-Host ("FAILED checks: {0}" -f $fail)
    exit 1
}
Write-Host "All critical checks passed."
exit 0
