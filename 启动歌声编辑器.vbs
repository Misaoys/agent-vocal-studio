Option Explicit
Dim fso, shell, root, executable
Set fso = CreateObject("Scripting.FileSystemObject")
Set shell = CreateObject("WScript.Shell")
root = fso.GetParentFolderName(WScript.ScriptFullName)
executable = fso.BuildPath(root, "bin\agent-vocal-desktop.exe")
If fso.FileExists(executable) Then
  shell.Run Chr(34) & executable & Chr(34) & " --home " & Chr(34) & root & Chr(34), 1, False
Else
  MsgBox "Please build Agent Vocal Studio first.", 48, "Agent Vocal Studio"
End If
