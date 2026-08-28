<#
.SYNOPSIS
  Build clean Windows zip packs (no keys, vaults, or payout configs) for a GitHub Release.
#>
[CmdletBinding()]
param(
    [string]$Version = "0.1.0-testnet",
    [string]$OutRoot = ""
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$Win = Join-Path $Root "Releases\Windows"
if ([string]::IsNullOrWhiteSpace($OutRoot)) {
    $OutRoot = Join-Path $Root "Releases\_github-dist"
}

function Ensure-Dir([string]$Path) {
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
}

if (-not (Test-Path $Win)) { throw "Missing $Win - stage Windows packs first." }

Add-Type -AssemblyName System.IO.Compression.FileSystem

if (Test-Path $OutRoot) { Remove-Item -Recurse -Force $OutRoot }
Ensure-Dir $OutRoot
$stage = Join-Path $OutRoot "stage"
Ensure-Dir $stage
$zipDir = Join-Path $OutRoot "zips"
Ensure-Dir $zipDir

$cfgMiner = @'
{
  "rpc": "https://eu.hashmonkeys.cloud",
  "address": "",
  "worker_name": "",
  "batch": 0,
  "max_nonces": 5000000,
  "selected": [],
  "ai_research": true
}
'@
$cfgNode = @'
{
  "listen": "0.0.0.0:39011",
  "rpc": "0.0.0.0:18080",
  "connect": [
    "seednode.hashmonkeys.cloud:39001"
  ],
  "mine": false,
  "mine_blocks": 0,
  "chain": "data/chain.bin",
  "wallet": "data/wallet.key",
  "p2p_key": "data/p2p.key",
  "miner_key": "data/wallet.key",
  "operator_address": "",
  "operator_vault": "",
  "orch": "http://seednode.hashmonkeys.cloud:18080"
}
'@
$cfgWallet = @'
{
  "rpc": "http://seednode.hashmonkeys.cloud:18080,http://seednode.hashmonkeys.cloud:18081",
  "wallet_vault": "data/wallet.vault.json"
}
'@

$packs = @(
    @{ Name = "Miner"; Require = @("MonkeyMesh-Miner.exe"); Config = $cfgMiner }
    @{ Name = "Node"; Require = @("MonkeyMesh-Node.exe", "mesh-node.exe"); Config = $cfgNode }
    @{ Name = "Wallet"; Require = @("MonkeyMesh-Wallet.exe"); Config = $cfgWallet }
)

$zips = @()
foreach ($pack in $packs) {
    $src = Join-Path $Win $pack.Name
    if (-not (Test-Path $src)) { throw "Missing pack folder $src" }
    foreach ($req in $pack.Require) {
        if (-not (Test-Path (Join-Path $src $req))) { throw "Missing $req in $src" }
    }
    $folderAs = if ($pack.FolderAs) { $pack.FolderAs } else { $pack.Name }
    $zipAs = if ($pack.ZipAs) { $pack.ZipAs } else { $pack.Name }
    $dst = Join-Path $stage $folderAs
    Ensure-Dir $dst
    Get-ChildItem $src -File | Where-Object {
        $_.Name -notmatch '\.new$' -and $_.Name -ne "config.json"
    } | Copy-Item -Destination $dst -Force
    Ensure-Dir (Join-Path $dst "data")
    Set-Content -Path (Join-Path $dst "data\.gitkeep") -Value "" -Encoding ascii
    if ($null -ne $pack.Config) {
        [System.IO.File]::WriteAllText((Join-Path $dst "config.json"), ($pack.Config.Trim() + "`n"))
    }
    $zipName = "MonkeyMesh-Windows-$zipAs-v$Version.zip"
    $zipPath = Join-Path $zipDir $zipName
    if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
    [System.IO.Compression.ZipFile]::CreateFromDirectory(
        $dst,
        $zipPath,
        [System.IO.Compression.CompressionLevel]::Optimal,
        $true
    )
    $item = Get-Item $zipPath
    $zips += $item
    $mb = [math]::Round($item.Length / 1MB, 1)
    Write-Host ("  {0}  {1} MB" -f $zipName, $mb)
}

$sumPath = Join-Path $zipDir "SHA256SUMS.txt"
$lines = @("# MonkeyMesh Windows GitHub Release v$Version", "# Generated $(Get-Date -Format o)")
$zips | Sort-Object Name | ForEach-Object {
    $h = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $lines += "$h  $($_.Name)"
}
$lines | Set-Content -Path $sumPath -Encoding ascii

$notesPath = Join-Path $OutRoot "RELEASE_NOTES.md"
$notes = @"
Public testnet - **not mainnet**.

Mine over HTTPS: ``https://eu.hashmonkeys.cloud``
Site / explorer: https://hashmonkeys.cloud

## Which zip?

| Pack | What it is |
|------|------------|
| **Miner** | Miner GUI (CPU + NVIDIA + AMD) |
| **Node** | Node GUI |
| **Wallet** | Wallet GUI + CLI |

Unzip each archive as a folder. Keep every DLL next to the exe.

**Windows GUIs:** double-click the ``.exe`` or ``Start-*.vbs`` (no Command Prompt).

Set your ``mesh01...`` address in the GUI. Packs ship with empty payout fields.

Fusion v4 is live from height 80 (45% seal / 45% GPU work / 10% nodes).
"@
[System.IO.File]::WriteAllText($notesPath, $notes)

Write-Host ""
Write-Host "Zips: $zipDir"
Write-Host "Notes: $notesPath"
Write-Host "Checksums: $sumPath"
