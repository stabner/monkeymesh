# Write SHA256SUMS.txt for Windows release packs (Build/28 M6).
param(
    [string]$WinRoot = ""
)
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
if (-not $WinRoot) { $WinRoot = Join-Path $Root "Releases\Windows" }
if (-not (Test-Path $WinRoot)) { throw "missing $WinRoot" }

$out = Join-Path $WinRoot "SHA256SUMS.txt"
$lines = @("# MonkeyMesh Windows release checksums", "# Generated $(Get-Date -Format o)")
Get-ChildItem $WinRoot -Recurse -File |
    Where-Object { $_.Extension -match '\.(exe|dll|json|bat|ps1|txt)$' -and $_.Name -ne 'SHA256SUMS.txt' } |
    Sort-Object FullName |
    ForEach-Object {
        $h = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        $rel = $_.FullName.Substring($WinRoot.Length).TrimStart('\','/') -replace '\\','/'
        $lines += "$h  $rel"
    }
$lines | Set-Content -Path $out -Encoding utf8
Write-Host "Wrote $out ($($lines.Count - 2) files)"
