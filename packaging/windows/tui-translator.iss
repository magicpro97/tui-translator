#ifndef AppVersion
#define AppVersion "0.1.0"
#endif

#ifndef ReleaseTag
#define ReleaseTag "v" + AppVersion
#endif

#ifndef OutputDir
#define OutputDir "installer-dist"
#endif

[Setup]
AppId={{3A9C1C5B-2800-4A96-A9C2-5EBD7D30B994}
AppName=TUI Translator
AppVersion={#AppVersion}
AppVerName=TUI Translator {#AppVersion}
AppPublisher=magicpro97
AppPublisherURL=https://github.com/magicpro97/tui-translator
AppSupportURL=https://github.com/magicpro97/tui-translator/issues
AppUpdatesURL=https://github.com/magicpro97/tui-translator/releases
DefaultDirName={localappdata}\Programs\TUI Translator
DefaultGroupName=TUI Translator
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=tui-translator-{#ReleaseTag}-setup
Compression=lzma
SolidCompression=yes
WizardStyle=modern
SetupLogging=yes
UninstallDisplayIcon={app}\tui-translator.exe

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "..\..\target\x86_64-pc-windows-msvc\release\tui-translator.exe"; DestDir: "{app}"; Flags: ignoreversion
; config.example.json is kept as a template reference only.
; The application reads its live configuration from
; %USERPROFILE%\.tui-translator\config.json (created by the first-run setup
; screen), not from a config.json placed beside the .exe.
Source: "..\..\config.example.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\USAGE.md"; DestDir: "{app}"; Flags: ignoreversion
; Top-level project license
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
; Third-party model and runtime licenses (JV-18 / #426)
Source: "..\..\assets\licenses\*"; DestDir: "{app}\LICENSES"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\TUI Translator"; Filename: "{app}\tui-translator.exe"
Name: "{autoprograms}\TUI Translator Setup Guide"; Filename: "{sys}\notepad.exe"; Parameters: """{app}\USAGE.md"""
Name: "{autoprograms}\Open TUI Translator folder"; Filename: "{sys}\explorer.exe"; Parameters: """{app}"""
Name: "{autodesktop}\TUI Translator"; Filename: "{app}\tui-translator.exe"; Tasks: desktopicon

[Run]
Filename: "{sys}\notepad.exe"; Parameters: """{app}\USAGE.md"""; Description: "Open the setup guide"; Flags: postinstall skipifsilent unchecked
