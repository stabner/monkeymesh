# MonkeyMesh CPU miner — mines through Node RPC with a custom payout address
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$config = Get-Content (Join-Path $PSScriptRoot "config.json") -Raw | ConvertFrom-Json
$miner = Join-Path $PSScriptRoot "bin\mesh-miner-cpu.exe"
$cli = Join-Path $PSScriptRoot "bin\mesh-wallet-cli.exe"
$key = Join-Path $PSScriptRoot $config.wallet_key

if (-not (Test-Path $miner)) {
    Write-Host "mesh-miner-cpu.exe missing in Miners\bin\"
    Write-Host "Run .\Launchers\build-release.ps1 from the repo root."
    Read-Host "Press Enter to exit"
    exit 1
}

New-Item -ItemType Directory -Force -Path (Join-Path $PSScriptRoot "data") | Out-Null

$address = "$($config.address)".Trim()
if (-not $address) {
    if (-not (Test-Path $cli)) {
        Write-Host "Set config.json `"address`" to your wallet address, or stage mesh-wallet-cli.exe."
        Read-Host "Press Enter to exit"
        exit 1
    }
    if (-not (Test-Path $key)) {
        Write-Host "Creating miner wallet key at $key"
        & $cli --wallet $key --rpc $config.rpc new
    }
    $address = (& $cli --wallet $key --rpc $config.rpc address).Trim()
}

Write-Host "MonkeyMesh CPU Miner"
Write-Host "  rpc    : $($config.rpc)"
Write-Host "  address: $address"
Write-Host "  blocks : $($config.blocks)  (0 = until Ctrl+C)"
Write-Host "  (Node must be running)"
Write-Host ""

$argsList = @(
    "--rpc", $config.rpc,
    "--address", $address,
    "--blocks", "$($config.blocks)",
    "--max-nonces", "$($config.max_nonces)"
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
