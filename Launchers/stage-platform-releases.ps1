<#
.SYNOPSIS
  Stage portable MonkeyMesh apps under Releases\Windows\ and Releases\Ubuntu\.

.DESCRIPTION
  Each app folder is standalone: exe/binary + start script + config + runtime DLLs/libs
  needed to run. Launchers\ remains the in-repo lab; Releases\ is what you zip and ship.

  Windows packs are built locally.   Ubuntu packs are pulled from MESH_NAS_HOST when that env is set and SSH works;
  otherwise the folder skeleton + scripts are still written.
#>

[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$SkipGpuMiner,
    [switch]$SkipUbuntu,
    [string]$NasHost = $(if ($env:MESH_NAS_HOST) { $env:MESH_NAS_HOST } else { "" }),
    [string]$NasBin = "~/monkeymesh-testnet/bin"
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $Root
. (Join-Path $PSScriptRoot "install-gui-launcher.ps1")

$WinRoot = Join-Path $Root "Releases\Windows"
$LinRoot = Join-Path $Root "Releases\Ubuntu"

function Ensure-Dir([string]$Path) {
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
}

function Copy-MsvcRuntime([string]$DestDir) {
    $copied = @()
    foreach ($name in @(
            "vcruntime140.dll",
            "vcruntime140_1.dll",
            "msvcp140.dll",
            "concrt140.dll"
        )) {
        $src = Join-Path $env:SystemRoot "System32\$name"
        if (Test-Path $src) {
            Copy-Item -Force $src (Join-Path $DestDir $name)
            $copied += $name
        }
    }
    return $copied
}

function Find-CudaRuntimeDll {
    $patterns = @()
    if ($env:CUDA_PATH) {
        $patterns += (Join-Path $env:CUDA_PATH "bin\x64\cudart64_*.dll")
        $patterns += (Join-Path $env:CUDA_PATH "bin\cudart64_*.dll")
    }
    $patterns += "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\*\bin\x64\cudart64_*.dll"
    $patterns += "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\*\bin\cudart64_*.dll"
    foreach ($pattern in $patterns) {
        $hit = Get-Item $pattern -ErrorAction SilentlyContinue |
            Sort-Object FullName -Descending |
            Select-Object -First 1
        if ($hit) { return $hit.FullName }
    }
    return $null
}

function Write-Manifest([string]$DestDir, [string]$Title, [string[]]$Required) {
    $lines = @(
        $Title,
        "Generated: $(Get-Date -Format o)",
        "",
        "Keep everything in this folder together (portable pack).",
        "",
        "Required:"
    ) + ($Required | ForEach-Object { "  - $_" })
    Set-Content -Path (Join-Path $DestDir "FILES.txt") -Value ($lines -join "`r`n") -Encoding utf8
}

function Preserve-Config([string]$DestConfig, [string]$TemplateConfig, [string[]]$ForceKeys = @()) {
    if (-not (Test-Path $TemplateConfig)) { return }
    if (Test-Path $DestConfig) {
        try {
            $existing = Get-Content -Raw $DestConfig | ConvertFrom-Json
            $tmpl = Get-Content -Raw $TemplateConfig | ConvertFrom-Json
            foreach ($p in $tmpl.PSObject.Properties) {
                if ($null -eq $existing.PSObject.Properties[$p.Name]) {
                    $existing | Add-Member -NotePropertyName $p.Name -NotePropertyValue $p.Value
                }
            }
            foreach ($k in $ForceKeys) {
                if ($null -ne $tmpl.PSObject.Properties[$k]) {
                    if ($null -eq $existing.PSObject.Properties[$k]) {
                        $existing | Add-Member -NotePropertyName $k -NotePropertyValue $tmpl.$k
                    } else {
                        $existing.$k = $tmpl.$k
                    }
                }
            }
            ($existing | ConvertTo-Json -Depth 6) + "`n" |
                Set-Content -Path $DestConfig -Encoding utf8 -NoNewline
            return
        } catch {
            Write-Host "  warning: could not merge $DestConfig - using template"
        }
    }
    Copy-Item -Force $TemplateConfig $DestConfig
}

function Ensure-DataDir([string]$DestDir) {
    $data = Join-Path $DestDir "data"
    Ensure-Dir $data
    $keep = Join-Path $data ".gitkeep"
    if (-not (Test-Path $keep)) { Set-Content -Path $keep -Value "" }
}

Write-Host "==> MonkeyMesh platform releases"
Write-Host "    Windows -> $WinRoot"
Write-Host "    Ubuntu  -> $LinRoot"
Write-Host ""

# ---------- Build (Windows host) ----------
if (-not $SkipBuild) {
    Write-Host "==> cargo build --release"
    $packages = @(
        "-p", "mesh-node",
        "-p", "mesh-node-gui",
        "-p", "mesh-orchestrator",
        "-p", "mesh-gpu-worker",
        "-p", "mesh-miner-cpu",
        "-p", "mesh-wallet-cli",
        "-p", "mesh-wallet-gui"
    )
    if (-not $SkipGpuMiner) {
        $packages += @("-p", "mesh-miner-gpu")
    }
    cargo build --release @packages
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

$rel = Join-Path $Root "target\release"
function Req([string]$Name) {
    $p = Join-Path $rel $Name
    if (-not (Test-Path $p)) { throw "Missing build output: $p" }
    return $p
}

# ---------- Windows / Node ----------
Write-Host "==> Windows\Node"
$nodeDir = Join-Path $WinRoot "Node"
Ensure-Dir $nodeDir
Ensure-DataDir $nodeDir
Copy-Item -Force (Req "mesh-node-gui.exe") (Join-Path $nodeDir "MonkeyMesh-Node.exe")
Copy-Item -Force (Req "mesh-node.exe") (Join-Path $nodeDir "mesh-node.exe")
Preserve-Config (Join-Path $nodeDir "config.json") (Join-Path $Root "Launchers\Node\config.json")
Install-GuiLauncher -RepoRoot $Root -DestDir $nodeDir -ExeName "MonkeyMesh-Node.exe" -StartBase "Start-Node"
@"
MonkeyMesh Node (Windows)

Double-click MonkeyMesh-Node.exe or Start-Node.vbs (no console window).

Includes:
  MonkeyMesh-Node.exe   desktop GUI (start/stop, rewards, soft settings)
  mesh-node.exe         headless node (scripts / advanced — this one is a console app)
  config.json           P2P / RPC / seed / orchestrator
  data\                 chain + keys (created on first run)

Explorer (when running): http://127.0.0.1:18080/

Keep all DLLs next to the exe. Do not split this folder.
"@ | Set-Content -Path (Join-Path $nodeDir "README.txt") -Encoding utf8
$nodeDlls = Copy-MsvcRuntime $nodeDir
Write-Manifest $nodeDir "MonkeyMesh Node (Windows)" (@(
        "MonkeyMesh-Node.exe",
        "mesh-node.exe",
        "Start-Node.vbs",
        "Start-Node.bat",
        "_start-gui.vbs",
        "config.json"
    ) + $nodeDlls)

# ---------- Windows / Wallet ----------
$walletGui = Join-Path $rel "mesh-wallet.exe"
if (Test-Path $walletGui) {
    Write-Host "==> Windows\Wallet"
    $walletDir = Join-Path $WinRoot "Wallet"
    Ensure-Dir $walletDir
    Ensure-DataDir $walletDir
    Copy-Item -Force $walletGui (Join-Path $walletDir "MonkeyMesh-Wallet.exe")
    Copy-Item -Force (Req "mesh-wallet-cli.exe") (Join-Path $walletDir "mesh-wallet-cli.exe")
    Preserve-Config (Join-Path $walletDir "config.json") (Join-Path $Root "Launchers\Wallet\config.json")
    Install-GuiLauncher -RepoRoot $Root -DestDir $walletDir -ExeName "MonkeyMesh-Wallet.exe" -StartBase "Start-Wallet"
    @"
MonkeyMesh Wallet (Windows)

Double-click MonkeyMesh-Wallet.exe or Start-Wallet.vbs (no console window).

Includes GUI + CLI. Edit config.json for RPC URL to your node/seed.
"@ | Set-Content -Path (Join-Path $walletDir "README.txt") -Encoding utf8
    $walletDlls = Copy-MsvcRuntime $walletDir
    $cudaDll = Find-CudaRuntimeDll
    $walletExtra = @()
    if ($cudaDll) {
        $cudaName = Split-Path -Leaf $cudaDll
        Copy-Item -Force $cudaDll (Join-Path $walletDir $cudaName)
        $walletExtra += $cudaName
        Write-Host "  Wallet bundled $cudaName"
    }
    Write-Manifest $walletDir "MonkeyMesh Wallet (Windows)" (@(
            "MonkeyMesh-Wallet.exe",
            "mesh-wallet-cli.exe",
            "Start-Wallet.vbs",
            "Start-Wallet.bat",
            "_start-gui.vbs",
            "config.json"
        ) + $walletDlls + $walletExtra)
} else {
    Write-Host "==> Windows\Wallet skipped (mesh-wallet.exe not built - run build-release.ps1 or cargo -p mesh-wallet-gui)"
}

# ---------- Windows / MonkeyMesh (all-in-one) ----------
if ((Test-Path $walletGui) -and (Test-Path (Join-Path $rel "mesh-node.exe"))) {
    Write-Host "==> Windows\MonkeyMesh"
    $suiteDir = Join-Path $WinRoot "MonkeyMesh"
    Ensure-Dir $suiteDir
    Ensure-DataDir $suiteDir
    Copy-Item -Force $walletGui (Join-Path $suiteDir "MonkeyMesh.exe")
    Copy-Item -Force (Req "mesh-node.exe") (Join-Path $suiteDir "mesh-node.exe")
    Copy-Item -Force (Req "mesh-wallet-cli.exe") (Join-Path $suiteDir "mesh-wallet-cli.exe")
    Preserve-Config (Join-Path $suiteDir "config.json") (Join-Path $Root "Launchers\MonkeyMesh\config.json")
    Install-GuiLauncher -RepoRoot $Root -DestDir $suiteDir -ExeName "MonkeyMesh.exe" -StartBase "Start-MonkeyMesh"
    Copy-Item -Force (Join-Path $Root "Launchers\MonkeyMesh\README.txt") (Join-Path $suiteDir "README.txt")
    $suiteDlls = Copy-MsvcRuntime $suiteDir
    $cudaDll = Find-CudaRuntimeDll
    $suiteExtra = @()
    if ($cudaDll) {
        $cudaName = Split-Path -Leaf $cudaDll
        Copy-Item -Force $cudaDll (Join-Path $suiteDir $cudaName)
        $suiteExtra += $cudaName
        Write-Host "  MonkeyMesh bundled $cudaName"
    }
    Write-Manifest $suiteDir "MonkeyMesh (Windows all-in-one)" (@(
            "MonkeyMesh.exe",
            "mesh-node.exe",
            "mesh-wallet-cli.exe",
            "Start-MonkeyMesh.vbs",
            "Start-MonkeyMesh.bat",
            "_start-gui.vbs",
            "config.json"
        ) + $suiteDlls + $suiteExtra)
} else {
    Write-Host "==> Windows\MonkeyMesh skipped (need mesh-wallet.exe + mesh-node.exe)"
}

# ---------- Windows / Orchestrator ----------
Write-Host "==> Windows\Orchestrator"
$orchDir = Join-Path $WinRoot "Orchestrator"
Ensure-Dir $orchDir
Ensure-DataDir $orchDir
Copy-Item -Force (Req "mesh-orchestrator.exe") (Join-Path $orchDir "mesh-orchestrator.exe")
Copy-Item -Force (Req "mesh-gpu-worker.exe") (Join-Path $orchDir "mesh-gpu-worker.exe")
@'
@echo off
cd /d "%~dp0"
set "PATH=%~dp0;%PATH%"
if not defined MESH_ORCH_BIND set "MESH_ORCH_BIND=127.0.0.1:18100"
if not defined MESH_NODE_RPC set "MESH_NODE_RPC=http://127.0.0.1:18080"
if not defined MESH_ORCH_REQUIRE_NODE set "MESH_ORCH_REQUIRE_NODE=1"
echo MonkeyMesh Orchestrator
echo   bind : %MESH_ORCH_BIND%
echo   node : %MESH_NODE_RPC%
mesh-orchestrator.exe
if errorlevel 1 pause
'@ | Set-Content -Path (Join-Path $orchDir "Start-Orchestrator.bat") -Encoding ascii
@'
@echo off
cd /d "%~dp0"
set "PATH=%~dp0;%PATH%"
if not defined MESH_ORCH set "MESH_ORCH=http://seednode.hashmonkeys.cloud:18080"
echo MonkeyMesh GPU / AI Worker (protocol + MNIST jobs via node)
echo   orch/node : %MESH_ORCH%
echo   (override MESH_ORCH only if you run a local node)
mesh-gpu-worker.exe --orch %MESH_ORCH% --jobs 0 --poll-ms 400 --keyfile data\gpu-worker.key
if errorlevel 1 pause
'@ | Set-Content -Path (Join-Path $orchDir "Start-GpuWorker.bat") -Encoding ascii
@"
MonkeyMesh Orchestrator + AI worker (Windows)

Seed node embeds the AI job board. On your PC you usually only need the worker:

  Start-GpuWorker.bat
  (default orch/node = http://seednode.hashmonkeys.cloud:18080)

Or run a local stack:
1. Start a Node (Releases\Windows\Node)
2. Start-Orchestrator.bat (optional marketplace UI)
3. Start-GpuWorker.bat with MESH_ORCH=http://127.0.0.1:18080

Prefer the dedicated pack: Releases\Windows\AiWorker
"@ | Set-Content -Path (Join-Path $orchDir "README.txt") -Encoding utf8
$orchDlls = Copy-MsvcRuntime $orchDir
Write-Manifest $orchDir "MonkeyMesh Orchestrator (Windows)" (@(
        "mesh-orchestrator.exe",
        "mesh-gpu-worker.exe",
        "Start-Orchestrator.bat",
        "Start-GpuWorker.bat"
    ) + $orchDlls)

# ---------- Windows / AiWorker (standalone GPU-market client) ----------
Write-Host "==> Windows\AiWorker"
$aiDir = Join-Path $WinRoot "AiWorker"
Ensure-Dir $aiDir
Ensure-DataDir $aiDir
Copy-Item -Force (Req "mesh-gpu-worker.exe") (Join-Path $aiDir "mesh-gpu-worker.exe")
@'
@echo off
cd /d "%~dp0"
set "PATH=%~dp0;%PATH%"
if not defined MESH_ORCH set "MESH_ORCH=http://seednode.hashmonkeys.cloud:18080"
echo.
echo  MonkeyMesh AI Worker
echo  ====================
echo  Pulls blockchain self-improvement + MNIST jobs from the seed NODE.
echo  This is the GPU-market worker — not the PoW block miner.
echo.
echo  orch/node : %MESH_ORCH%
echo  payout key: data\gpu-worker.key  (created on first run)
echo.
echo  Press Ctrl+C to stop.
echo.
mesh-gpu-worker.exe --orch %MESH_ORCH% --jobs 0 --poll-ms 400 --keyfile data\gpu-worker.key
echo.
echo Worker exited.
pause
'@ | Set-Content -Path (Join-Path $aiDir "Start-AiWorker.bat") -Encoding ascii
@"
MonkeyMesh AI Worker (Windows)

MonkeyMesh-Miner  = MeshHash / Fusion blocks (90% pot)
AiWorker (this folder)                 = protocol research + MNIST (same pot)

Double-click Start-AiWorker.bat — seed AI board (http://seednode.hashmonkeys.cloud:18080).
Override: set MESH_ORCH=http://127.0.0.1:18080 for a local node.
"@ | Set-Content -Path (Join-Path $aiDir "README.txt") -Encoding utf8
$aiDlls = Copy-MsvcRuntime $aiDir
Write-Manifest $aiDir "MonkeyMesh AI Worker (Windows)" (@(
        "mesh-gpu-worker.exe",
        "Start-AiWorker.bat"
    ) + $aiDlls)

# ---------- Windows miners via existing packager (into Windows\) ----------
Write-Host "==> Windows miner (Miner GUI: CPU + NVIDIA + AMD)"
& (Join-Path $PSScriptRoot "stage-miner-releases.ps1") -SkipBuild -OutRoot $WinRoot
# PowerShell scripts do not reliably set LASTEXITCODE; trust terminating errors only.
if (-not $?) { throw "stage-miner-releases failed" }

# ---------- Ubuntu skeleton + NAS pull ----------
Write-Host "==> Ubuntu packs"
Ensure-Dir $LinRoot
@"
MonkeyMesh Ubuntu / Linux releases

Layout:
  Node/           mesh-node + start-node.sh + config.json
  Orchestrator/   mesh-orchestrator + mesh-gpu-worker + start scripts
  CpuMiner/       mesh-miner-cpu + start-cpu-miner.sh

Build on Linux (or pull from the seed NAS after mesh-testnet.sh build):
  ssh $NasHost
  ~/src/MonkeyMesh/Launchers/testnet/mesh-testnet.sh build

This staging script copies binaries from ${NasHost}:${NasBin} when reachable.
GPU GUI miners are Windows-focused; Linux uses the CLI CPU miner + GPU research worker.
"@ | Set-Content -Path (Join-Path $LinRoot "README.txt") -Encoding utf8

function Write-UbuntuStart([string]$Path, [string]$Body) {
    # LF scripts for Linux
    [System.IO.File]::WriteAllText($Path, ($Body -replace "`r`n", "`n"))
}

$ubuNode = Join-Path $LinRoot "Node"
$ubuOrch = Join-Path $LinRoot "Orchestrator"
$ubuCpu = Join-Path $LinRoot "CpuMiner"
foreach ($d in @($ubuNode, $ubuOrch, $ubuCpu)) {
    Ensure-Dir $d
    Ensure-DataDir $d
}

# Ubuntu configs (same network defaults as Windows)
Preserve-Config (Join-Path $ubuNode "config.json") (Join-Path $Root "Launchers\Node\config.json")
Preserve-Config (Join-Path $ubuCpu "config.json") (Join-Path $Root "Launchers\miner-release-templates\CpuMiner\runtime\config.json") -ForceKeys @("rpc")

Write-UbuntuStart (Join-Path $ubuNode "start-node.sh") @'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
chmod +x ./mesh-node 2>/dev/null || true
LISTEN=$(python3 -c "import json;print(json.load(open('config.json'))['listen'])")
RPC=$(python3 -c "import json;print(json.load(open('config.json'))['rpc'])")
CONNECT=$(python3 -c "import json;print(' '.join('--connect '+p for p in json.load(open('config.json')).get('connect',[])))")
OP_ADDR=$(python3 -c "import json;print(json.load(open('config.json')).get('operator_address','').strip())")
OP_VAULT=$(python3 -c "import json;print(json.load(open('config.json')).get('operator_vault','').strip())")
OP_ARGS=()
if [[ -n "$OP_ADDR" ]]; then
  export MESH_OPERATOR_ADDRESS="$OP_ADDR"
  OP_ARGS+=(--operator-address "$OP_ADDR")
fi
if [[ -n "$OP_VAULT" ]]; then
  VAULT_PATH="$OP_VAULT"
  if [[ "$VAULT_PATH" != /* ]]; then VAULT_PATH="$(pwd)/$VAULT_PATH"; fi
  export MESH_OPERATOR_VAULT="$VAULT_PATH"
  OP_ARGS+=(--operator-vault "$VAULT_PATH")
fi
# shellcheck disable=SC2086
exec ./mesh-node --chain data/chain.bin serve --listen "$LISTEN" --rpc "$RPC" \
  --wallet data/wallet.key --p2p-key data/p2p.key --miner-key data/wallet.key \
  "${OP_ARGS[@]}" $CONNECT
'@

Write-UbuntuStart (Join-Path $ubuOrch "start-orchestrator.sh") @'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
chmod +x ./mesh-orchestrator 2>/dev/null || true
export MESH_ORCH_BIND="${MESH_ORCH_BIND:-0.0.0.0:18100}"
export MESH_NODE_RPC="${MESH_NODE_RPC:-http://127.0.0.1:18080}"
export MESH_ORCH_REQUIRE_NODE="${MESH_ORCH_REQUIRE_NODE:-1}"
exec ./mesh-orchestrator
'@

Write-UbuntuStart (Join-Path $ubuOrch "start-gpu-worker.sh") @'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
chmod +x ./mesh-gpu-worker 2>/dev/null || true
ORCH="${MESH_ORCH:-http://127.0.0.1:18080}"
mkdir -p data
exec ./mesh-gpu-worker --orch "$ORCH" --jobs 8 --poll-ms 400 --keyfile data/gpu-worker.key
'@

Write-UbuntuStart (Join-Path $ubuCpu "start-cpu-miner.sh") @'
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
chmod +x ./mesh-miner-cpu 2>/dev/null || true
RPC=$(python3 -c "import json;print(json.load(open('config.json'))['rpc'].rstrip('/'))")
ADDR=$(python3 -c "import json;print(json.load(open('config.json')).get('address','').strip())")
BLOCKS=$(python3 -c "import json;print(json.load(open('config.json')).get('blocks',0))")
MAX_NONCES=$(python3 -c "import json;print(json.load(open('config.json')).get('max_nonces',5000000))")
if [[ -z "$ADDR" ]]; then
  echo "Set address in config.json to your wallet payout address."
  exit 1
fi
exec ./mesh-miner-cpu --rpc "$RPC" --address "$ADDR" --blocks "$BLOCKS" --max-nonces "$MAX_NONCES"
'@

@"
MonkeyMesh Node (Ubuntu)

./start-node.sh

Requires: mesh-node binary in this folder (staged from NAS or local Linux cargo build).
"@ | Set-Content -Path (Join-Path $ubuNode "README.txt") -Encoding utf8
@"
MonkeyMesh Orchestrator + GPU research worker (Ubuntu)

./start-orchestrator.sh
./start-gpu-worker.sh
"@ | Set-Content -Path (Join-Path $ubuOrch "README.txt") -Encoding utf8
@"
MonkeyMesh CPU Miner (Ubuntu)

Edit config.json address, then:
./start-cpu-miner.sh
"@ | Set-Content -Path (Join-Path $ubuCpu "README.txt") -Encoding utf8

if (-not $SkipUbuntu) {
    Write-Host "==> Pull Ubuntu binaries from $NasHost ($NasBin)"
    $pulled = $false
    try {
        $remoteList = ssh -o BatchMode=yes -o ConnectTimeout=8 $NasHost "ls -1 $NasBin" 2>$null
        if ($LASTEXITCODE -eq 0 -and $remoteList) {
            $map = @{
                "mesh-node"           = (Join-Path $ubuNode "mesh-node")
                "mesh-orchestrator"   = (Join-Path $ubuOrch "mesh-orchestrator")
                "mesh-gpu-worker"     = (Join-Path $ubuOrch "mesh-gpu-worker")
                "mesh-miner-cpu"      = (Join-Path $ubuCpu "mesh-miner-cpu")
            }
            foreach ($name in $map.Keys) {
                if ($remoteList -match [regex]::Escape($name)) {
                    $dest = $map[$name]
                    scp -o BatchMode=yes -o ConnectTimeout=15 "${NasHost}:${NasBin}/$name" $dest
                    if ($LASTEXITCODE -eq 0) {
                        Write-Host "  pulled $name"
                        $pulled = $true
                    }
                }
            }
        }
    } catch {
        Write-Host "  NAS pull skipped: $_"
    }
    if (-not $pulled) {
        Write-Host "  No binaries pulled - Ubuntu folders have scripts/config only."
        Write-Host "  Build on Linux or fix SSH to $NasHost, then re-run with -SkipBuild."
    } else {
        Write-Manifest $ubuNode "MonkeyMesh Node (Ubuntu)" @("mesh-node", "start-node.sh", "config.json")
        Write-Manifest $ubuOrch "MonkeyMesh Orchestrator (Ubuntu)" @(
            "mesh-orchestrator", "mesh-gpu-worker", "start-orchestrator.sh", "start-gpu-worker.sh"
        )
        Write-Manifest $ubuCpu "MonkeyMesh CPU Miner (Ubuntu)" @("mesh-miner-cpu", "start-cpu-miner.sh", "config.json")
    }
}

# ---------- Legacy redirect stubs (old flat Releases\* paths) ----------
foreach ($legacy in @("CpuMiner", "GpuMiner", "Miner")) {
    $legacyDir = Join-Path $Root "Releases\$legacy"
    Ensure-Dir $legacyDir
    $target = "Miner"
    Get-ChildItem $legacyDir -Force -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -notin @("MOVED.txt", "Open-Windows-Pack.bat") } |
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    @"
This pack moved to:

  Releases\Windows\Miner\

CPU-only and GPU-only ship packs are retired. Use the Miner GUI (CPU + NVIDIA + AMD).

Run staging:
  .\Launchers\stage-platform-releases.ps1
"@ | Set-Content -Path (Join-Path $legacyDir "MOVED.txt") -Encoding utf8
    @"
@echo off
echo Pack moved to Releases\Windows\Miner\
start "" explorer "%~dp0..\Windows\Miner"
"@ | Set-Content -Path (Join-Path $legacyDir "Open-Windows-Pack.bat") -Encoding ascii
}

Write-Host ""
Write-Host "==> Done"
Write-Host "Ship these folders (zip each app folder):"
Write-Host "  Releases\Windows\Node"
Write-Host "  Releases\Windows\Wallet"
Write-Host "  Releases\Windows\Orchestrator"
Write-Host "  Releases\Windows\AiWorker"
Write-Host "  Releases\Windows\Miner"
Write-Host "  Releases\Windows\MonkeyMesh   (Desktop)"
Write-Host "  Releases\Ubuntu\Node"
Write-Host "  Releases\Ubuntu\Orchestrator"
Write-Host ""
