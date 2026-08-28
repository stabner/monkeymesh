# MonkeyMesh Wallet CLI helper (points at Node RPC)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$config = Get-Content (Join-Path $PSScriptRoot "config.json") -Raw | ConvertFrom-Json
$cli = Join-Path $PSScriptRoot "bin\mesh-wallet-cli.exe"
$wallet = Join-Path $PSScriptRoot "data\wallet.key"

if (-not (Test-Path $cli)) {
    Write-Host "mesh-wallet-cli.exe missing. Run .\Launchers\build-release.ps1"
    Read-Host "Press Enter to exit"
    exit 1
}

New-Item -ItemType Directory -Force -Path (Join-Path $PSScriptRoot "data") | Out-Null

if (-not $args -or $args.Count -eq 0) {
    Write-Host "Usage examples:"
    Write-Host "  Start-WalletCli.bat info"
    Write-Host "  Start-WalletCli.bat balance"
    Write-Host "  Start-WalletCli.bat mine --blocks 1"
    Write-Host "  Start-WalletCli.bat send <address> 1.5"
    Write-Host ""
    & $cli --wallet $wallet --rpc $config.rpc info
    exit $LASTEXITCODE
}

& $cli --wallet $wallet --rpc $config.rpc @args
exit $LASTEXITCODE
