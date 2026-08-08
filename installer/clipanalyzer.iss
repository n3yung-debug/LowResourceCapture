; Inno Setup script for ClipAnalyzer — the offline VOD analyzer.
;
; Deliberately a SEPARATE installer from LowResourceCapture. The recorder is a
; tiny always-resident process tuned for minimal footprint while gaming; this is
; an offline tool that is free to use every core. Different AppId, different
; install dir, independent install/uninstall — you can have either, or both.
;
; Build locally with:  ISCC.exe /DMyAppVersion=0.1.0-alpha installer\clipanalyzer.iss
; Output: installer\Output\ClipAnalyzer-Setup-<version>.exe

#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif

#define MyAppName "ClipAnalyzer"
#define MyAppExe  "clipanalyzer.exe"
#define MyAppPublisher "LowResourceCapture"

[Setup]
; Stable AppId, distinct from the recorder's — keep constant across versions so
; upgrades replace cleanly and neither app can uninstall the other.
AppId={{C4E70B93-6A15-4D82-9F3B-71A0D6E85C24}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
PrivilegesRequired=admin
DefaultDirName={commonpf32}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#MyAppExe}
SetupIconFile=..\assets\icon.ico
OutputDir=Output
OutputBaseFilename=ClipAnalyzer-Setup-{#MyAppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Dirs]
; Same rationale as the recorder: the app runs non-elevated but lives in
; Program Files, so the Users group needs modify rights for config, logs, and
; the detection profiles the calibration UI writes.
Name: "{app}"; Permissions: users-modify
Name: "{app}\logs"; Permissions: users-modify
Name: "{app}\profiles"; Permissions: users-modify

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "..\target\release\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
; Its own copy of ffmpeg. The analyzer is independently installable, so it
; cannot assume the recorder is present to borrow one from.
Source: "..\ffmpeg.exe"; DestDir: "{app}"; Flags: ignoreversion
; Detection profiles — game-specific data, not code, so a new game is a file
; drop rather than a release.
Source: "..\crates\analyzer\profiles\*.toml"; DestDir: "{app}\profiles"; Flags: ignoreversion
; The trainer the Training panel runs. Without this the Train button works in
; a dev checkout and fails on a real install.
Source: "..\crates\analyzer\training\train.py"; DestDir: "{app}\training"; Flags: ignoreversion
Source: "..\crates\analyzer\training\README.md"; DestDir: "{app}\training"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; Tasks: desktopicon

[UninstallRun]
Filename: "{cmd}"; Parameters: "/C taskkill /IM {#MyAppExe} /F & exit 0"; Flags: runhidden; RunOnceId: "StopAnalyzer"

[UninstallDelete]
; Runtime-created state only. Your videos, your saved clips, and anything the
; recorder owns are untouched.
Type: filesandordirs; Name: "{app}\logs"
Type: files; Name: "{app}\config.toml"
Type: filesandordirs; Name: "{app}\{#MyAppExe}.WebView2"
