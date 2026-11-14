[Setup]
AppId=FloatingTray.Mohmmadshabrani
AppName=Floating Tray
AppVersion={#AppVersion}
AppPublisher=Floating Tray contributors
AppPublisherURL=https://github.com/Mohmmadshabrani/Floating-Tray
AppSupportURL=https://github.com/Mohmmadshabrani/Floating-Tray/issues
AppUpdatesURL=https://github.com/Mohmmadshabrani/Floating-Tray/releases
DefaultDirName={localappdata}\Programs\Floating Tray
DefaultGroupName=Floating Tray
DisableWelcomePage=yes
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
MinVersion=10.0.22000
AppMutex=yasb_reborn
CloseApplications=no
WizardStyle=modern
UninstallDisplayIcon={app}\floating-tray.exe
OutputDir={#OutputDirectory}
OutputBaseFilename=Floating-Tray-Setup-x64

[Files]
Source: "{#PayloadDirectory}\floating-tray.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDirectory}\YASBTrayHook.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDirectory}\WINDOWS.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDirectory}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDirectory}\HOOK-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Floating Tray"; Filename: "{app}\floating-tray.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\floating-tray.exe"; Description: "Launch Floating Tray"; Flags: nowait postinstall skipifsilent
