Set fso = CreateObject("Scripting.FileSystemObject")
dir = fso.GetParentFolderName(WScript.ScriptFullName)
CreateObject("Wscript.Shell").Run "wscript //nologo """ & dir & "\_start-gui.vbs"" ""MonkeyMesh-Miner.exe""", 0, False
