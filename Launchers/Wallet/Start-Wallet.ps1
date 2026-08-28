# MonkeyMesh Wallet — silent GUI launch (no console hang)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$configPath = Join-Path $PSScriptRoot "config.json"
$exe = Join-Path $PSScriptRoot "bin\mesh-wallet.exe"
$binDir = Join-Path $PSScriptRoot "bin"
$netPath = Join-Path $PSScriptRoot "..\network.json"

if (-not (Test-Path $exe)) {
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.MessageBox]::Show(
        "mesh-wallet.exe not found.`nRun .\Launchers\build-release.ps1 first.",
        "MonkeyMesh Wallet"
    ) | Out-Null
    exit 1
}

$config = Get-Content $configPath -Raw | ConvertFrom-Json
$rpcList = @($config.rpc -split ',' | ForEach-Object { $_.Trim().TrimEnd('/') } | Where-Object { $_ })
$lanRpc = $null
if (Test-Path $netPath) {
    $net = Get-Content $netPath -Raw | ConvertFrom-Json
    if ($net.lan_rpc) { $lanRpc = [string]$net.lan_rpc.TrimEnd("/") }
    if ($rpcList.Count -eq 0 -and $net.rpc) {
        $rpcList = @($net.rpc -split ',' | ForEach-Object { $_.Trim().TrimEnd('/') } | Where-Object { $_ })
    }
}

function Test-MeshRpc([string]$Base) {
    try {
        Invoke-RestMethod "$Base/v1/getnodeinfo" -TimeoutSec 2 | Out-Null
        return $true
    } catch {
        return $false
    }
}

$rpc = $null
foreach ($c in $rpcList) {
    if (Test-MeshRpc $c) { $rpc = $c; break }
}
if (-not $rpc -and $lanRpc -and (Test-MeshRpc $lanRpc)) { $rpc = $lanRpc }
if (-not $rpc) { $rpc = if ($rpcList.Count -gt 0) { $rpcList[0] } else { "http://seednode.hashmonkeys.cloud:18080" } }

$env:MESH_RPC = $rpc

if ($config.wallet_key) {
    $env:MESH_WALLET_KEY = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot $config.wallet_key))
}

Copy-Item -Force $configPath (Join-Path $binDir "config.json")
# Persist chosen RPC into bin config so GUI picks it up
$binCfg = Get-Content (Join-Path $binDir "config.json") -Raw | ConvertFrom-Json
$binCfg.rpc = $rpc
($binCfg | ConvertTo-Json -Depth 5) | Set-Content -Encoding utf8 (Join-Path $binDir "config.json")

Start-Process -FilePath $exe -WorkingDirectory $binDir
exit 0
