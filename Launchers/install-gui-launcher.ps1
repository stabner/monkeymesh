# Writes a silent GUI launcher pair (Start-*.vbs + Start-*.bat) into a ship pack.
function Install-GuiLauncher {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$DestDir,
        [Parameter(Mandatory = $true)][string]$ExeName,
        [Parameter(Mandatory = $true)][string]$StartBase
    )
    $helperSrc = Join-Path $RepoRoot "Launchers\windows-start-gui.vbs"
    if (-not (Test-Path $helperSrc)) { throw "Missing $helperSrc" }
    Copy-Item -Force $helperSrc (Join-Path $DestDir "_start-gui.vbs")
    $vbs = @"
Set fso = CreateObject("Scripting.FileSystemObject")
dir = fso.GetParentFolderName(WScript.ScriptFullName)
CreateObject("Wscript.Shell").Run "wscript //nologo """ & dir & "\_start-gui.vbs"" ""$ExeName""", 0, False
"@
    Set-Content -Path (Join-Path $DestDir "$StartBase.vbs") -Value $vbs -Encoding ascii
    $bat = @"
@echo off
cd /d "%~dp0"
wscript //nologo "%~dp0_start-gui.vbs" "$ExeName"
"@
    Set-Content -Path (Join-Path $DestDir "$StartBase.bat") -Value $bat -Encoding ascii
}
