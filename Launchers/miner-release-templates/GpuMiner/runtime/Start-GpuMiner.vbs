Set fso = CreateObject("Scripting.FileSystemObject")
dir = fso.GetParentFolderName(WScript.ScriptFullName)
CreateObject("Wscript.Shell").Run "wscript //nologo """ & dir & "\_start-gui.vbs"" ""MonkeyMesh-GpuMiner.exe""", 0, False
