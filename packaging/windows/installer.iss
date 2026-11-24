; Windows Setup Installer Script (Inno Setup 6)
; Build via: .\packaging\build-release.ps1

#ifndef AppName
  #define AppName "Ledge Grab"
#endif
#ifndef AppFileSlug
  #define AppFileSlug "LedgeGrab"
#endif
#ifndef AppExeName
  #define AppExeName "ledge-grab.exe"
#endif
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef AppPublisher
  #define AppPublisher "Ledge Grab"
#endif
#ifndef AppDescription
  #define AppDescription "Screenshots that live on the ledge."
#endif
#ifndef AppUrl
  #define AppUrl "https://github.com/Nabin-me/ledge-grab"
#endif
#ifndef AppIcon
  #define AppIcon "assets\ledge-grab.ico"
#endif
#ifndef AppMutexName
  #define AppMutexName "Local\LedgeGrab"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\ledge-grab.exe"
#endif
#ifndef OutputDirPath
  #define OutputDirPath "..\..\dist"
#endif

[Setup]
AppId={{8F3B9C1A-4D2E-4E6B-9A73-2C1F0E5D8B41}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
AppComments={#AppDescription}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir={#OutputDirPath}
OutputBaseFilename={#AppFileSlug}-Setup-{#AppVersion}-x64
SetupIconFile=..\..\{#AppIcon}
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
AppMutex={#AppMutexName}
CloseApplications=yes
RestartApplications=yes
ChangesEnvironment=no
MinVersion=10.0.19041
#ifdef EnableSigning
SignTool=signtool
SignedUninstaller=yes
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start {#AppName} automatically when I sign in to Windows"; GroupDescription: "Startup:"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "{#AppExeName}"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\assets\OFL.txt"; DestDir: "{app}"; DestName: "LICENSE-FONT.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Comment: "{#AppDescription}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Comment: "{#AppDescription}"; Tasks: desktopicon

[Registry]
; Matches the app's built-in "Launch at login" registry key so toggling it in-app stays in sync
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\{#AppExeName}"""; Flags: uninsdeletevalue; Tasks: autostart
; Ensure any login entry created later from inside the app is also cleaned up on uninstall
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "{#AppName}"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(AppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
