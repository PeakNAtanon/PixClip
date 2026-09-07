Option Explicit
Dim shell, files, folder, shortcut, destination
Set shell = CreateObject("WScript.Shell")
Set files = CreateObject("Scripting.FileSystemObject")
folder = files.GetParentFolderName(WScript.ScriptFullName)
destination = files.BuildPath(folder, "PixClip.lnk")
Set shortcut = shell.CreateShortcut(destination)
shortcut.TargetPath = files.BuildPath(folder, "PixClip.bat")
shortcut.WorkingDirectory = folder
shortcut.IconLocation = files.BuildPath(folder, "assets\pixclip.ico") & ",0"
shortcut.Description = "PixClip - Download, Record, Convert, Cut and Join"
shortcut.WindowStyle = 7
shortcut.Save
