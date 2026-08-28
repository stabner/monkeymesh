' MonkeyMesh silent GUI launcher (no console window).
' Usage: wscript //nologo _start-gui.vbs App.exe
Option Explicit
Dim fso, sh, dir, exeName, exe, f, foundCuda
If WScript.Arguments.Count < 1 Then
  MsgBox "Usage: _start-gui.vbs App.exe", 16, "MonkeyMesh"
  WScript.Quit 1
End If
Set fso = CreateObject("Scripting.FileSystemObject")
Set sh = CreateObject("Wscript.Shell")
dir = fso.GetParentFolderName(WScript.ScriptFullName)
exeName = WScript.Arguments(0)
exe = dir & "\" & exeName
If Not fso.FileExists(exe) Then
  MsgBox "Missing " & exeName & vbCrLf & vbCrLf & "Re-stage this pack from Launchers.", 16, "MonkeyMesh"
  WScript.Quit 1
End If
If LCase(exeName) = "monkeymesh-gpuminer.exe" Or LCase(exeName) = "monkeymesh-miner.exe" Then
  If Not fso.FileExists(dir & "\vcruntime140.dll") Then
    MsgBox "Missing vcruntime140.dll. Re-stage this pack.", 16, "MonkeyMesh"
    WScript.Quit 1
  End If
  foundCuda = False
  For Each f In fso.GetFolder(dir).Files
    If LCase(Left(f.Name, 9)) = "cudart64_" Then
      If LCase(Right(f.Name, 4)) = ".dll" Then foundCuda = True
    End If
  Next
  If Not foundCuda Then
    MsgBox "Missing cudart64_*.dll next to the miner. Re-stage this pack.", 16, "MonkeyMesh"
    WScript.Quit 1
  End If
End If
sh.CurrentDirectory = dir
' 1 = normal GUI window; False = do not wait (no console is created).
sh.Run """" & exe & """", 1, False
