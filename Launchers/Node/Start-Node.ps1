# MonkeyMesh Node launcher — prefers the native GUI; falls back to headless CLI.
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

# Live seed retarget is 15. Old default 20 forks testers at height 150.
if (-not $env:MESH_FORCE_RETARGET_INTERVAL -or -not "$($env:MESH_FORCE_RETARGET_INTERVAL)".Trim()) {
    $env:MESH_FORCE_RETARGET_INTERVAL = "15"
}

New-Item -ItemType Directory -Force -Path (Join-Path $PSScriptRoot "data") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $PSScriptRoot "bin") | Out-Null

$gui = Join-Path $PSScriptRoot "MonkeyMesh-Node.exe"
if (Test-Path $gui) {
    # B4: arm token in parent env before GUI/child spawn
    $dataDir = Join-Path $PSScriptRoot "data"
    New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
    $aiTokenPath = Join-Path $dataDir "ai.token"
    $rpcTokenPath = Join-Path $dataDir "rpc.token"
    $autoAi = if ($null -ne $env:MESH_AI_TOKEN_AUTO) { $env:MESH_AI_TOKEN_AUTO } else { "1" }
    if (-not $env:MESH_AI_TOKEN -or -not "$($env:MESH_AI_TOKEN)".Trim()) {
        if ($autoAi -eq "1" -or $autoAi -eq "true") {
            if (-not (Test-Path $aiTokenPath)) {
                $bytes = New-Object byte[] 32
                [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
                ($bytes | ForEach-Object { $_.ToString("x2") }) -join "" | Set-Content -NoNewline -Path $aiTokenPath
            }
            $env:MESH_AI_TOKEN = (Get-Content -Raw $aiTokenPath).Trim()
        }
    }
    if (-not $env:MESH_RPC_TOKEN -or -not "$($env:MESH_RPC_TOKEN)".Trim()) {
        if (-not (Test-Path $rpcTokenPath)) {
            $bytes = New-Object byte[] 32
            [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
            ($bytes | ForEach-Object { $_.ToString("x2") }) -join "" | Set-Content -NoNewline -Path $rpcTokenPath
        }
        $env:MESH_RPC_TOKEN = (Get-Content -Raw $rpcTokenPath).Trim()
    }
    Start-Process -FilePath $gui -WorkingDirectory $PSScriptRoot
    exit 0
}

$configPath = Join-Path $PSScriptRoot "config.json"
if (-not (Test-Path $configPath)) {
    Write-Error "Missing config.json next to this script."
}
$config = Get-Content $configPath -Raw | ConvertFrom-Json

$exe = Join-Path $PSScriptRoot "bin\mesh-node.exe"
if (-not (Test-Path $exe)) {
    Write-Host ""
    Write-Host "MonkeyMesh-Node.exe / mesh-node.exe not found."
    Write-Host "Build first from the repo root:"
    Write-Host "  .\Launchers\build-release.ps1"
    Write-Host ""
    Read-Host "Press Enter to exit"
    exit 1
}

$chain = Join-Path $PSScriptRoot $config.chain
$wallet = Join-Path $PSScriptRoot $config.wallet
$p2pKey = Join-Path $PSScriptRoot $config.p2p_key
$minerKey = Join-Path $PSScriptRoot $config.miner_key

$args = @(
    "--chain", $chain,
    "serve",
    "--listen", $config.listen,
    "--rpc", $config.rpc,
    "--wallet", $wallet,
    "--p2p-key", $p2pKey,
    "--miner-key", $minerKey
)

foreach ($peer in @($config.connect)) {
    if ($peer) { $args += @("--connect", "$peer") }
}

if ($config.mine) {
    $args += "--mine"
    if ($config.mine_blocks -gt 0) {
        $args += @("--mine-blocks", "$($config.mine_blocks)")
    }
}

# N10: cold operator payout (address-only; vault never unlocked)
$opAddr = ""
if ($null -ne $config.PSObject.Properties["operator_address"]) {
    $opAddr = [string]$config.operator_address
}
$opVault = ""
if ($null -ne $config.PSObject.Properties["operator_vault"]) {
    $opVault = [string]$config.operator_vault
}
if ($opAddr.Trim()) {
    $env:MESH_OPERATOR_ADDRESS = $opAddr.Trim()
    $args += @("--operator-address", $opAddr.Trim())
}
if ($opVault.Trim()) {
    $vaultPath = Join-Path $PSScriptRoot $opVault.Trim()
    $env:MESH_OPERATOR_VAULT = $vaultPath
    $args += @("--operator-vault", $vaultPath)
}

# B4: sticky AI board token (parity with Linux MESH_AI_TOKEN_AUTO)
$dataDir = Join-Path $PSScriptRoot "data"
$aiTokenPath = Join-Path $dataDir "ai.token"
$rpcTokenPath = Join-Path $dataDir "rpc.token"
$autoAi = if ($null -ne $env:MESH_AI_TOKEN_AUTO) { $env:MESH_AI_TOKEN_AUTO } else { "1" }
if (-not $env:MESH_AI_TOKEN -or -not "$($env:MESH_AI_TOKEN)".Trim()) {
    if ($autoAi -eq "1" -or $autoAi -eq "true") {
        if (-not (Test-Path $aiTokenPath)) {
            $bytes = New-Object byte[] 32
            [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
            ($bytes | ForEach-Object { $_.ToString("x2") }) -join "" | Set-Content -NoNewline -Path $aiTokenPath
            Write-Host "generated AI board token -> $aiTokenPath"
        }
        $env:MESH_AI_TOKEN = (Get-Content -Raw $aiTokenPath).Trim()
        Write-Host "mesh-node: MESH_AI_TOKEN armed (MESH_AI_TOKEN_AUTO=$autoAi)"
    } else {
        Write-Host "mesh-node: AI board open (MESH_AI_TOKEN_AUTO=0)"
    }
}
if (-not $env:MESH_RPC_TOKEN -or -not "$($env:MESH_RPC_TOKEN)".Trim()) {
    if (-not (Test-Path $rpcTokenPath)) {
        $bytes = New-Object byte[] 32
        [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
        ($bytes | ForEach-Object { $_.ToString("x2") }) -join "" | Set-Content -NoNewline -Path $rpcTokenPath
        Write-Host "generated wallet RPC cookie -> $rpcTokenPath"
    }
    $env:MESH_RPC_TOKEN = (Get-Content -Raw $rpcTokenPath).Trim()
    Write-Host "mesh-node: MESH_RPC_TOKEN armed (fail-closed wallet/gov RPC)"
}

Write-Host "MonkeyMesh Node (headless — GUI not staged)"
Write-Host "  listen : $($config.listen)"
Write-Host "  rpc    : http://$($config.rpc)"
Write-Host "  explorer: http://$($config.rpc)/"
Write-Host "  chain  : $chain"
if ($opAddr.Trim()) {
    Write-Host "  operator: $($opAddr.Trim())"
} elseif ($opVault.Trim()) {
    Write-Host "  operator vault: $vaultPath"
}
Write-Host ""

& $exe @args
$exit = $LASTEXITCODE
if ($exit -ne 0) {
    Write-Host ""
    Write-Host "Node exited with code $exit"
    Read-Host "Press Enter to close"
    exit $exit
}
