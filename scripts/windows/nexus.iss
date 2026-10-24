; ==============================================================================
; Inno Setup Script for NEXUS Archiver
; Compiles standard Windows setup executable (NEXUS-Setup.exe)
; Compile using Inno Setup Compiler: iscc scripts\windows\nexus.iss
; ==============================================================================

#define MyAppName "NEXUS Archiver"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "NEXUS Core Team"
#define MyAppURL "https://github.com/nexus-archiver/nexus"
#define MyAppExeName "nexus-gui.exe"

[Setup]
AppId={{9E811F3A-9B4B-4E83-A52C-21BDC4D0DE63}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={localappdata}\Programs\NEXUS
DisableDirPage=no
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\..\dist
OutputBaseFilename=NEXUS-v{#MyAppVersion}-Setup
SetupIconFile=..\..\assets\nexus.ico
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\..\target\release\nexus-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\nexus.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\nexus.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\nexus_logo.png"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\nexus.ico"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\nexus.ico"; Tasks: desktopicon

[Registry]
; Context menu for files
Root: HKCU; Subkey: "Software\Classes\*\shell\NexusCompress"; ValueType: string; ValueData: "Compress with NEXUS..."; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\*\shell\NexusCompress"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\nexus.ico"""
Root: HKCU; Subkey: "Software\Classes\*\shell\NexusCompress\command"; ValueType: string; ValueData: """{app}\nexus-gui.exe"" compress ""%1"""

; Context menu for directories
Root: HKCU; Subkey: "Software\Classes\Directory\shell\NexusCompress"; ValueType: string; ValueData: "Compress with NEXUS..."; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Directory\shell\NexusCompress"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\nexus.ico"""
Root: HKCU; Subkey: "Software\Classes\Directory\shell\NexusCompress\command"; ValueType: string; ValueData: """{app}\nexus-gui.exe"" compress ""%1"""

; Context menu for archives (.nxar)
Root: HKCU; Subkey: "Software\Classes\SystemFileAssociations\.nxar\shell\NexusExtractGui"; ValueType: string; ValueData: "Extract with NEXUS..."; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\SystemFileAssociations\.nxar\shell\NexusExtractGui"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\nexus.ico"""
Root: HKCU; Subkey: "Software\Classes\SystemFileAssociations\.nxar\shell\NexusExtractGui\command"; ValueType: string; ValueData: """{app}\nexus-gui.exe"" decompress ""%1"""

Root: HKCU; Subkey: "Software\Classes\SystemFileAssociations\.nxar\shell\NexusExtractHere"; ValueType: string; ValueData: "Extract Here"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\SystemFileAssociations\.nxar\shell\NexusExtractHere"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\nexus.ico"""
Root: HKCU; Subkey: "Software\Classes\SystemFileAssociations\.nxar\shell\NexusExtractHere\command"; ValueType: string; ValueData: """{app}\nexus-gui.exe"" decompress ""%1"" --auto-start"

; File association for .nxar
Root: HKCU; Subkey: "Software\Classes\.nxar"; ValueType: string; ValueData: "Nexus.Archive"; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\Nexus.Archive"; ValueType: string; ValueData: "NEXUS Compressed Archive"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Nexus.Archive\DefaultIcon"; ValueType: string; ValueData: """{app}\nexus.ico"",0"
Root: HKCU; Subkey: "Software\Classes\Nexus.Archive\shell\open\command"; ValueType: string; ValueData: """{app}\nexus-gui.exe"" decompress ""%1"""

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
