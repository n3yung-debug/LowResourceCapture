; Inno Setup script for LowResourceCapture.
; Machine-wide install to Program Files (x86) (requires admin/UAC), Start Menu
; shortcut, optional run-at-startup, clean uninstaller. Version is passed in
; from CI: ISCC /DMyAppVersion=x.y.z
;
; Build locally with:  ISCC.exe /DMyAppVersion=0.1.0-alpha installer\lowresourcecapture.iss
; Output: installer\Output\LowResourceCapture-Setup-<version>.exe

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif

#define MyAppName "LowResourceCapture"
#define MyAppExe  "lowresourcecapture.exe"
#define MyAppPublisher "LowResourceCapture"

[Setup]
; Stable AppId — keep constant across versions so upgrades replace cleanly.
AppId={{8F3A1C2E-9B4D-4E6A-A1F7-2C5D8E0B4A91}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
; Machine-wide install into C:\Program Files (x86)\LowResourceCapture. This is
; a protected system location, so setup requests admin elevation (UAC). Inno
; creates the folder if it doesn't exist; on upgrade it detects the prior
; install (same AppId) and reuses its location.
PrivilegesRequired=admin
DefaultDirName={commonpf32}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#MyAppExe}
; App icon for the setup wizard (the exe carries its own embedded icon).
SetupIconFile=..\assets\icon.ico
OutputDir=Output
OutputBaseFilename=LowResourceCapture-Setup-{#MyAppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Try to close a running instance gracefully during install/uninstall.
CloseApplications=yes
RestartApplications=no
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Dirs]
; The app runs non-elevated (tray app / run-at-startup), but Program Files is
; read-only to non-elevated processes. Grant the Users group modify rights so
; the app can write its config.toml and logs here. `logs` is created up front so
; logging always has a writable target.
Name: "{app}"; Permissions: users-modify
Name: "{app}\logs"; Permissions: users-modify

[Tasks]
Name: "startup"; Description: "Start {#MyAppName} automatically when Windows starts"; GroupDescription: "Startup:"; Flags: checkedonce
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
; The release build produced by CI (cargo build --release).
Source: "..\target\release\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; Tasks: desktopicon

[Registry]
; Run-at-startup for the current user (per-user, no admin). Gated on the task.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; \
  ValueType: string; ValueName: "{#MyAppName}"; ValueData: """{app}\{#MyAppExe}"""; \
  Flags: uninsdeletevalue; Tasks: startup

[Run]
; Offer to launch right after install.
Filename: "{app}\{#MyAppExe}"; Description: "Launch {#MyAppName} now"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Make sure the tray app isn't running, so its exe/log aren't locked when the
; uninstaller removes files. `& exit 0` keeps uninstall from erroring if the
; app wasn't running.
Filename: "{cmd}"; Parameters: "/C taskkill /IM {#MyAppExe} /F & exit 0"; Flags: runhidden; RunOnceId: "StopApp"

[UninstallDelete]
; Remove runtime-created config + logs (they live in the install dir now) so the
; folder is left clean. SEPARATE from your saved clips, which live in your Videos
; folder and are intentionally left untouched.
Type: filesandordirs; Name: "{app}\logs"
Type: files; Name: "{app}\config.toml"
; WebView2 runtime data folder, created next to the exe when the settings
; window opens. Not needed after uninstall — remove it for a clean uninstall.
Type: filesandordirs; Name: "{app}\{#MyAppExe}.WebView2"
; Also clean up the old per-user data location from previous versions.
Type: filesandordirs; Name: "{userappdata}\{#MyAppName}"
