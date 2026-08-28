<#
.SYNOPSIS
  Build production release binaries and stage them into Launchers\*\bin\

.DESCRIPTION
  Builds mesh-node, mesh-node-gui, mesh-orchestrator, mesh-gpu-worker, mesh-miner-cpu, mesh-miner-gpu, mesh-wallet-cli, and the native mesh-wallet GUI,
  then copies executables into the standalone launcher folders.
#>

[CmdletBinding()]
param(
    [switch]$SkipWalletGui,
    [switch]$UseTauriWallet,
    [switch]$SkipGpuMiner
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $Root

Write-Host "==> MonkeyMesh release build"
Write-Host "    root: $Root"
Write-Host ""

function Ensure-Dir([string]$Path) {
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
}

function Copy-Binary([string]$Name, [string]$DestDir) {
    $src = Join-Path $Root "target\release\$Name"
    if (-not (Test-Path $src)) {
        throw "Missing build output: $src"
    }
    Ensure-Dir $DestDir
    Copy-Item -Force $src (Join-Path $DestDir $Name)
    Write-Host "  staged $Name -> $DestDir"
}

Write-Host "==> cargo build --release (node, node-gui, orchestrator, worker, miners, wallet-cli)"
$packages = @("-p", "mesh-node", "-p", "mesh-node-gui", "-p", "mesh-orchestrator", "-p", "mesh-gpu-worker", "-p", "mesh-miner-cpu", "-p", "mesh-wallet-cli")
if (-not $SkipGpuMiner) {
    $packages += @("-p", "mesh-miner-gpu")
}
cargo build --release @packages
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$nodeDir = Join-Path $Root "Launchers\Node"
Copy-Binary "mesh-node.exe" (Join-Path $nodeDir "bin")
$guiSrc = Join-Path $Root "target\release\mesh-node-gui.exe"
if (-not (Test-Path $guiSrc)) { throw "Missing build output: $guiSrc" }
Copy-Item -Force $guiSrc (Join-Path $nodeDir "MonkeyMesh-Node.exe")
Write-Host "  staged MonkeyMesh-Node.exe -> $nodeDir"
foreach ($name in @("vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll", "concrt140.dll")) {
    $src = Join-Path $env:SystemRoot "System32\$name"
    if (Test-Path $src) {
        Copy-Item -Force $src (Join-Path $nodeDir $name)
    }
}

$orchDir = Join-Path $Root "Launchers\Orchestrator"
Copy-Binary "mesh-orchestrator.exe" (Join-Path $orchDir "bin")
Copy-Binary "mesh-gpu-worker.exe" (Join-Path $orchDir "bin")

Copy-Binary "mesh-wallet-cli.exe" (Join-Path $Root "Launchers\Wallet\bin")
Copy-Binary "mesh-wallet-cli.exe" (Join-Path $Root "Launchers\Miners\bin")

if (-not $SkipWalletGui) {
    $dest = Join-Path $Root "Launchers\Wallet\bin"
    Ensure-Dir $dest

    if ($UseTauriWallet) {
        $walletApp = Join-Path $Root "apps\mesh-wallet"
        Write-Host ""
        Write-Host "==> Tauri wallet release (legacy)"
        Push-Location $walletApp
        try {
            npm install
            if ($LASTEXITCODE -ne 0) { throw "npm install failed" }
            npm run tauri build
            if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
        } finally {
            Pop-Location
        }
        $candidates = @(
            (Join-Path $Root "target\release\mesh-wallet.exe"),
            (Join-Path $Root "apps\mesh-wallet\src-tauri\target\release\mesh-wallet.exe")
        )
        $walletExe = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
        if (-not $walletExe) { throw "mesh-wallet.exe not found after tauri build" }
        Copy-Item -Force $walletExe (Join-Path $dest "mesh-wallet.exe")
    } else {
        Write-Host ""
        Write-Host "==> Native wallet GUI release (egui)"
        cargo build --release -p mesh-wallet-gui
        if ($LASTEXITCODE -ne 0) { throw "mesh-wallet-gui build failed" }
        Copy-Binary "mesh-wallet.exe" $dest
    }

    Write-Host "  staged mesh-wallet.exe -> $dest"
    Copy-Item -Force (Join-Path $Root "Launchers\Wallet\config.json") (Join-Path $dest "config.json")

    # CUDA runtime for in-wallet NVIDIA mining
    $cudaDll = $null
    if ($env:CUDA_PATH) {
        $cudaDll = Get-Item (Join-Path $env:CUDA_PATH "bin\x64\cudart64_*.dll") -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending | Select-Object -First 1
        if (-not $cudaDll) {
            $cudaDll = Get-Item (Join-Path $env:CUDA_PATH "bin\cudart64_*.dll") -ErrorAction SilentlyContinue |
                Sort-Object FullName -Descending | Select-Object -First 1
        }
    }
    if (-not $cudaDll) {
        $cudaDll = Get-Item "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\*\bin\x64\cudart64_*.dll" -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending | Select-Object -First 1
    }
    if ($cudaDll) {
        Copy-Item -Force $cudaDll.FullName (Join-Path $dest $cudaDll.Name)
        Write-Host "  bundled $($cudaDll.Name) -> Wallet\bin (for NVIDIA mining)"
    }
    foreach ($name in @("vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll", "concrt140.dll")) {
        $src = Join-Path $env:SystemRoot "System32\$name"
        if (Test-Path $src) {
            Copy-Item -Force $src (Join-Path $dest $name)
        }
    }
}

# Ensure runtime data dirs exist
Ensure-Dir (Join-Path $Root "Launchers\Node\data")
Ensure-Dir (Join-Path $Root "Launchers\Wallet\data")
Ensure-Dir (Join-Path $Root "Launchers\Miners\data")

# Placeholder so empty bin folders stay intentional
@(
    "Launchers\Node\bin",
    "Launchers\Wallet\bin",
    "Launchers\Miners\bin"
) | ForEach-Object {
    $keep = Join-Path $Root "$_\.gitkeep"
    if (-not (Test-Path (Join-Path $Root $_))) { Ensure-Dir (Join-Path $Root $_) }
    if (-not (Test-Path $keep)) { Set-Content -Path $keep -Value "" }
}

# Portable packs last (needs wallet + miners already built)
Write-Host ""
Write-Host "==> Staging Releases\Windows and Releases\Ubuntu"
& (Join-Path $PSScriptRoot "stage-platform-releases.ps1") -SkipBuild -SkipGpuMiner:$SkipGpuMiner
if ($LASTEXITCODE -ne 0) { throw "stage-platform-releases failed" }

Write-Host ""
Write-Host "==> Done"
Write-Host "Lab (in-repo):"
Write-Host "  1. Launchers\Node\Start-Node.bat"
Write-Host "  2. Launchers\Orchestrator\Start-Orchestrator.bat"
Write-Host "Ship (portable):"
Write-Host "  Releases\Windows\Node\"
Write-Host "  Releases\Windows\Wallet\"
Write-Host "  Releases\Windows\Miner\"
Write-Host "  Releases\Windows\MonkeyMesh\  (Desktop)"
Write-Host "  Releases\Ubuntu\Node\  (+ Orchestrator, CpuMiner)"
Write-Host ""
Write-Host "Refresh portable packs only:"
Write-Host "  .\Launchers\stage-platform-releases.ps1"
Write-Host ""
Write-Host "Note: Build/31 shared pot + MeshHash-Evo. Wipe Node\data\chain.bin if upgrading an old chain."
Write-Host ""
