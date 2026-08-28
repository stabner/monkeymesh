# Publish lab tip-freeze artifact from live seed getnodeinfo (Build/28 M4 / Build/29).
# Usage:
#   .\Launchers\publish-tip-freeze.ps1
#   .\Launchers\publish-tip-freeze.ps1 -Network mesh-public-testnet -OutFile Build\genesis-mesh-public-testnet.json
param(
    [string]$Rpc = "http://seednode.hashmonkeys.cloud:18080",
    [string]$Edge2Rpc = "http://seednode.hashmonkeys.cloud:18081",
    [string]$Network = "mesh-public-testnet",
    [string]$OutFile = ""
)
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
if (-not $OutFile) {
    $OutFile = Join-Path $Root "Build\genesis-$Network.json"
}

$seed = Invoke-RestMethod -Uri "$Rpc/v1/getnodeinfo" -TimeoutSec 15
$ai = $null
try { $ai = Invoke-RestMethod -Uri "$Rpc/v1/ai/health" -TimeoutSec 15 } catch {}
$edge2 = $null
try { $edge2 = Invoke-RestMethod -Uri "$Edge2Rpc/v1/getnodeinfo" -TimeoutSec 15 } catch {}

if ($edge2 -and ($edge2.tip -ne $seed.tip -or [int64]$edge2.height -ne [int64]$seed.height)) {
    throw ("REFUSED: edge2 tip mismatch (seed h={0} tip={1}; edge2 h={2} tip={3}). Align first." -f `
        $seed.height, $seed.tip, $edge2.height, $edge2.tip)
}

$doc = [ordered]@{
    network            = $Network
    status             = "public-testnet"
    note               = "Public endpoints only. Not geographic mainnet M1. Soft AI never changes BPS/consensus."
    frozen_at_utc      = (Get-Date).ToUniversalTime().ToString("o")
    genesis            = [string]$seed.genesis
    height             = [int64]$seed.height
    tip                = [string]$seed.tip
    hosts              = [ordered]@{
        seed_rpc  = "$Rpc"
        edge_rpc  = "$Edge2Rpc"
        seed_p2p  = "seednode.hashmonkeys.cloud:39001"
        pool      = "https://eu.hashmonkeys.cloud"
    }
    brain_standby      = [ordered]@{
        shared_digest    = $(if ($ai -and $ai.brain) { [string]$ai.brain.digest_hex } else { $null })
        shared_epoch     = $(if ($ai -and $ai.brain) { $ai.brain.epoch } else { $null })
        shared_v2_digest = $(if ($ai -and $ai.brain_v2) { [string]$ai.brain_v2.digest_hex } else { $null })
        replication      = "Launchers/testnet/sync-brains-to-hashserver.ps1"
    }
    wipe_policy        = "MESH_ALLOW_WIPE=1 and -WipeConfirm DELETE_PUBLIC_TIP required on the operator deploy script"
    soft_ai_policy     = "Soft knobs only - never BPS, subsidy, fork choice, or block validity from AI"
    next_geographic_m1 = "Add off-site VPS seed when credentials work; then re-run ceremony for mainnet"
}

$dir = Split-Path -Parent $OutFile
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
($doc | ConvertTo-Json -Depth 6) | Set-Content -Path $OutFile -Encoding utf8
Write-Host ("Wrote {0}" -f $OutFile)
Write-Host ("genesis={0}" -f $seed.genesis)
Write-Host ("height={0} tip={1}" -f $seed.height, $seed.tip)
