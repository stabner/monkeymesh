<#
.SYNOPSIS
  Stage fully self-contained CpuMiner / GpuMiner / Miner folders under Releases\Windows\
#>

[CmdletBinding()]
param(
    [switch]$SkipBuild,
    # Destination root for packs (default: Releases\Windows)
    [string]$OutRoot = ""
)

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $Root
. (Join-Path $PSScriptRoot "install-gui-launcher.ps1")

function Ensure-Dir([string]$Path) {
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
}

if ([string]::IsNullOrWhiteSpace($OutRoot)) {
    $OutRoot = Join-Path $Root "Releases\Windows"
} else {
    if (-not [System.IO.Path]::IsPathRooted($OutRoot)) {
        $OutRoot = Join-Path $Root $OutRoot
    }
    $OutRoot = [System.IO.Path]::GetFullPath($OutRoot)
}
Ensure-Dir $OutRoot
Write-Host "==> Miner packs -> $OutRoot"
Write-Host ""

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
    if ($copied.Count -eq 0) {
        throw "No MSVC runtime DLLs found in System32 - install VC++ Redistributable"
    }
    return $copied
}

function Preserve-Config([string]$DestConfig, [string]$TemplateConfig, [string[]]$ForceKeys = @()) {
    if (Test-Path $DestConfig) {
        try {
            $existing = Get-Content -Raw $DestConfig | ConvertFrom-Json
            $tmpl = Get-Content -Raw $TemplateConfig | ConvertFrom-Json
            # Keep user fields; fill any new template keys.
            foreach ($p in $tmpl.PSObject.Properties) {
                if ($null -eq $existing.PSObject.Properties[$p.Name]) {
                    $existing | Add-Member -NotePropertyName $p.Name -NotePropertyValue $p.Value
                }
            }
            # Force-refresh operational defaults (e.g. edge-first rpc) without wiping address.
            foreach ($k in $ForceKeys) {
                if ($null -ne $tmpl.PSObject.Properties[$k]) {
                    if ($null -eq $existing.PSObject.Properties[$k]) {
                        $existing | Add-Member -NotePropertyName $k -NotePropertyValue $tmpl.$k
                    } else {
                        $existing.$k = $tmpl.$k
                    }
                }
            }
            ($existing | ConvertTo-Json -Depth 5) + "`n" |
                Set-Content -Path $DestConfig -Encoding utf8 -NoNewline
            return
        } catch {
            Write-Host "  warning: could not merge existing config - keeping template"
        }
    }
    Copy-Item -Force $TemplateConfig $DestConfig
}

function Copy-TemplateFiles([string]$Name, [string]$DestDir) {
    # Pack contents come from runtime\ (templates\ root only has redirectors).
    $src = Join-Path $Root "Launchers\miner-release-templates\$Name\runtime"
    Ensure-Dir $DestDir
    Ensure-Dir (Join-Path $DestDir "data")
    if (-not (Test-Path (Join-Path $DestDir "data\.gitkeep"))) {
        Set-Content -Path (Join-Path $DestDir "data\.gitkeep") -Value ""
    }
    Copy-Item -Force (Join-Path $src "Start-*.bat") $DestDir
    Get-ChildItem -Path $src -Filter "Start-*.vbs" -ErrorAction SilentlyContinue |
        ForEach-Object { Copy-Item -Force $_.FullName $DestDir }
    Get-ChildItem -Path $src -Filter "_start-gui.vbs" -ErrorAction SilentlyContinue |
        ForEach-Object { Copy-Item -Force $_.FullName $DestDir }
    Copy-Item -Force (Join-Path $src "README.txt") $DestDir
    Preserve-Config (Join-Path $DestDir "config.json") (Join-Path $src "config.json") -ForceKeys @("rpc")
}

function Write-Manifest([string]$DestDir, [string[]]$Required) {
    $lines = @(
        "MonkeyMesh miner release pack",
        "Generated: $(Get-Date -Format o)",
        "",
        "Required files in this folder:"
    ) + ($Required | ForEach-Object { "  - $_" }) + @(
        "",
        "This folder is portable. Keep these files together."
    )
    Set-Content -Path (Join-Path $DestDir "FILES.txt") -Value ($lines -join "`r`n") -Encoding utf8
}

Write-Host "==> MonkeyMesh miner releases (self-contained)"
Write-Host ""

if (-not $SkipBuild) {
    Write-Host "==> Building release miners (CUDA required)"
    if ($env:CUDA_PATH) {
        $env:Path = (Join-Path $env:CUDA_PATH "bin") + ";" + $env:Path
    }
    $env:MESH_REQUIRE_CUDA = "1"
    cargo build --release -p mesh-miner-cpu -p mesh-miner-gpu
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed (CUDA required for GpuMiner/Miner)" }

    $cudaOut = Get-ChildItem (Join-Path $Root "target\release\build\mesh-miner-gpu-*\output") -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if (-not $cudaOut) { throw "mesh-miner-gpu build output missing" }
    $outText = Get-Content -Raw $cudaOut.FullName
    if ($outText -notmatch "rustc-cfg=mesh_cuda") {
        throw "mesh_cuda cfg not set - nvcc did not enable CUDA (see $($cudaOut.FullName))"
    }
    $lib = Get-ChildItem (Join-Path $Root "target\release\build\mesh-miner-gpu-*\out\meshhash_cuda_mix.lib") -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if (-not $lib) { throw "meshhash_cuda_mix.lib missing after CUDA build" }
    Write-Host "  CUDA OK: mesh_cuda cfg + $($lib.Name) ($([math]::Round($lib.Length/1KB)) KB)"
}

$cpuSrc = Join-Path $Root "target\release\mesh-miner-cpu.exe"
$gpuCliSrc = Join-Path $Root "target\release\mesh-miner-gpu.exe"
$gpuGuiSrc = Join-Path $Root "target\release\mesh-miner-gpu-gui.exe"
if (-not (Test-Path $cpuSrc)) { throw "Missing $cpuSrc - build first" }
if (-not (Test-Path $gpuCliSrc)) { throw "Missing $gpuCliSrc - build first" }
if (-not (Test-Path $gpuGuiSrc)) { throw "Missing $gpuGuiSrc - build first" }

# Prefer payout address from Launchers\Miners\config.json when creating a fresh pack
$seedAddress = $null
$seedCfg = Join-Path $Root "Launchers\Miners\config.json"
if (Test-Path $seedCfg) {
    try {
        $seedAddress = ((Get-Content -Raw $seedCfg | ConvertFrom-Json).address)
        if ([string]::IsNullOrWhiteSpace($seedAddress)) { $seedAddress = $null }
    } catch { }
}

# -------- CPU release --------
$cpuDir = Join-Path $OutRoot "CpuMiner"
Copy-TemplateFiles "CpuMiner" $cpuDir
Copy-Item -Force $cpuSrc (Join-Path $cpuDir "MonkeyMesh-CpuMiner.exe")
$cpuRuntime = Copy-MsvcRuntime $cpuDir
foreach ($n in $cpuRuntime) { Write-Host "  CPU bundled $n" }

function Set-ConfigAddress($cfg, [string]$path, [string]$addr) {
    if (-not $addr) { return $false }
    $has = $null -ne $cfg.PSObject.Properties["address"]
    $cur = if ($has) { [string]$cfg.address } else { "" }
    if (-not [string]::IsNullOrWhiteSpace($cur)) { return $false }
    if ($has) { $cfg.address = $addr }
    else { $cfg | Add-Member -NotePropertyName address -NotePropertyValue $addr }
    ($cfg | ConvertTo-Json -Depth 5) + "`n" | Set-Content -Path $path -Encoding utf8 -NoNewline
    return $true
}

# Seed address into config if still empty
$cpuCfgPath = Join-Path $cpuDir "config.json"
$cpuCfg = Get-Content -Raw $cpuCfgPath | ConvertFrom-Json
[void](Set-ConfigAddress $cpuCfg $cpuCfgPath $seedAddress)
Write-Manifest $cpuDir (@(
    "MonkeyMesh-CpuMiner.exe",
    "Start-CpuMiner.bat",
    "config.json"
) + $cpuRuntime)
Write-Host "  staged Releases\CpuMiner\MonkeyMesh-CpuMiner.exe"

# -------- All-in-one Miner release (CPU + NVIDIA + AMD) --------
$minerDir = Join-Path $OutRoot "Miner"
Copy-TemplateFiles "Miner" $minerDir
Install-GuiLauncher -RepoRoot $Root -DestDir $minerDir -ExeName "MonkeyMesh-Miner.exe" -StartBase "Start-Miner"
Copy-Item -Force $gpuGuiSrc (Join-Path $minerDir "MonkeyMesh-Miner.exe")
Copy-Item -Force $gpuCliSrc (Join-Path $minerDir "mesh-miner-cli.exe")
$minerRuntime = Copy-MsvcRuntime $minerDir
foreach ($n in $minerRuntime) { Write-Host "  Miner bundled $n" }

$cudaDll = Find-CudaRuntimeDll
if (-not $cudaDll) {
    throw "cudart64_*.dll not found under CUDA_PATH - install CUDA toolkit or set CUDA_PATH"
}
$cudaName = Split-Path -Leaf $cudaDll
Copy-Item -Force $cudaDll (Join-Path $minerDir $cudaName)
Write-Host "  Miner bundled $cudaName"

$minerCfgPath = Join-Path $minerDir "config.json"
$minerCfg = Get-Content -Raw $minerCfgPath | ConvertFrom-Json
[void](Set-ConfigAddress $minerCfg $minerCfgPath $seedAddress)
Write-Manifest $minerDir (@(
    "MonkeyMesh-Miner.exe",
    "mesh-miner-cli.exe",
    "Start-Miner.vbs",
    "Start-Miner.bat",
    "_start-gui.vbs",
    "config.json",
    $cudaName
) + $minerRuntime)
Write-Host "  staged Releases\Miner\MonkeyMesh-Miner.exe (all-in-one GUI)"

# -------- GPU release (same GUI, legacy name) --------
$gpuDir = Join-Path $OutRoot "GpuMiner"
Copy-TemplateFiles "GpuMiner" $gpuDir
Install-GuiLauncher -RepoRoot $Root -DestDir $gpuDir -ExeName "MonkeyMesh-GpuMiner.exe" -StartBase "Start-GpuMiner"
Copy-Item -Force $gpuGuiSrc (Join-Path $gpuDir "MonkeyMesh-GpuMiner.exe")
Copy-Item -Force $gpuCliSrc (Join-Path $gpuDir "mesh-miner-gpu-cli.exe")
$gpuRuntime = Copy-MsvcRuntime $gpuDir
foreach ($n in $gpuRuntime) { Write-Host "  GPU bundled $n" }

Copy-Item -Force $cudaDll (Join-Path $gpuDir $cudaName)
Write-Host "  GPU bundled $cudaName"

$gpuCfgPath = Join-Path $gpuDir "config.json"
$gpuCfg = Get-Content -Raw $gpuCfgPath | ConvertFrom-Json
[void](Set-ConfigAddress $gpuCfg $gpuCfgPath $seedAddress)
# Migrate old backend configs: ensure selected exists
if ($null -eq $gpuCfg.PSObject.Properties["selected"]) {
    $gpuCfg | Add-Member -NotePropertyName selected -NotePropertyValue @()
    ($gpuCfg | ConvertTo-Json -Depth 5) + "`n" | Set-Content -Path $gpuCfgPath -Encoding utf8 -NoNewline
}
Write-Manifest $gpuDir (@(
    "MonkeyMesh-GpuMiner.exe",
    "mesh-miner-gpu-cli.exe",
    "Start-GpuMiner.vbs",
    "Start-GpuMiner.bat",
    "_start-gui.vbs",
    "config.json",
    $cudaName
) + $gpuRuntime)
Write-Host "  staged Releases\GpuMiner\MonkeyMesh-GpuMiner.exe (GUI)"
Write-Host "  staged Releases\GpuMiner\mesh-miner-gpu-cli.exe"

# Keep Launchers\Miners\bin in sync for lab scripts
$legacyBin = Join-Path $Root "Launchers\Miners\bin"
Ensure-Dir $legacyBin
Copy-Item -Force $cpuSrc (Join-Path $legacyBin "mesh-miner-cpu.exe")
Copy-Item -Force $gpuCliSrc (Join-Path $legacyBin "mesh-miner-gpu.exe")
Copy-Item -Force $gpuGuiSrc (Join-Path $legacyBin "mesh-miner-gpu-gui.exe")
Copy-Item -Force $cudaDll (Join-Path $legacyBin $cudaName)
Copy-MsvcRuntime $legacyBin | Out-Null

Write-Host ""
Write-Host "==> Done (portable miner folders under $OutRoot)"
Write-Host "  $cpuDir"
Get-ChildItem $cpuDir -File | ForEach-Object { Write-Host ("    {0,-28} {1,10:N0} bytes" -f $_.Name, $_.Length) }
Write-Host "  $minerDir   (recommended all-in-one)"
Get-ChildItem $minerDir -File | ForEach-Object { Write-Host ("    {0,-28} {1,10:N0} bytes" -f $_.Name, $_.Length) }
Write-Host "  $gpuDir"
Get-ChildItem $gpuDir -File | ForEach-Object { Write-Host ("    {0,-28} {1,10:N0} bytes" -f $_.Name, $_.Length) }
Write-Host ""
exit 0
