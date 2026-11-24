[Setup]
AppName=Tilik
AppVersion=0.2.0
AppPublisher=Tilik
DefaultDirName={autopf}\Tilik
DefaultGroupName=Tilik
OutputDir=..\dist
OutputBaseFilename=tilik-setup
Compression=lzma
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64
UninstallDisplayIcon={app}\tilik-ui.exe

[Files]
Source: "..\target\release\tilik-ui.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Tilik"; Filename: "{app}\tilik-ui.exe"
Name: "{commondesktop}\Tilik"; Filename: "{app}\tilik-ui.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional icons:"

[Run]
Filename: "{app}\tilik-ui.exe"; Description: "Launch Tilik"; Flags: nowait postinstall skipifsilent