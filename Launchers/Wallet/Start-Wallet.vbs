' Silent lab launcher for Launchers\Wallet (exe lives in bin\).
Option Explicit
Dim fso, sh, dir, exe, cfg, binCfg
Set fso = CreateObject("Scripting.FileSystemObject")
Set sh = CreateObject("Wscript.Shell")
dir = fso.GetParentFolderName(WScript.ScriptFullName)
exe = dir & "\bin\mesh-wallet.exe"
If Not fso.FileExists(exe) Then
  MsgBox "mesh-wallet.exe not found in Launchers\Wallet\bin\" & vbCrLf & "Build first: Launchers\build-release.ps1", 16, "MonkeyMesh Wallet"
  WScript.Quit 1
End If
cfg = dir & "\config.json"
binCfg = dir & "\bin\config.json"
If fso.FileExists(cfg) Then
  fso.CopyFile cfg, binCfg, True
End If
sh.CurrentDirectory = dir & "\bin"
sh.Run """" & exe & """", 1, False
