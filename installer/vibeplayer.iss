; Build after `cargo build --release`, with FFMPEG_DIR set: iscc installer/vibeplayer.iss

#define AppName "VibePlayer"
#define AppExeName "vibeplayer.exe"
#define AppExe AddBackslash(SourcePath) + "..\target\release\" + AppExeName
#define AppVersion GetStringFileInfo(AppExe, "ProductVersion")
#define FFmpegDir AddBackslash(GetEnv("FFMPEG_DIR"))
#define FFmpegBin FFmpegDir + "bin"
#define ProgId "VibePlayer.Video"

#if AppVersion == ""
  #error "Build target\release\vibeplayer.exe first"
#endif
#if GetEnv("FFMPEG_DIR") == ""
  #error "Set FFMPEG_DIR to the shared FFmpeg build the exe was built against"
#endif

[Setup]
; AppId identifies the app to Windows across versions; never change it.
AppId={{1E5868AD-FEAC-497D-9A52-7B045AF8AD5E}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
; Per-user by default so the in-app updater can run the installer without an admin prompt.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=yes
CloseApplications=yes
RestartApplications=yes
UninstallDisplayIcon={app}\{#AppExeName}
SetupIconFile=..\crates\app\resources\icon.ico
LicenseFile=..\LICENSE
OutputDir=..\target\installer
OutputBaseFilename=VibePlayer-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}\licenses"; DestName: "VibePlayer-LICENSE.txt"; Flags: ignoreversion
Source: "{#FFmpegBin}\*.dll"; DestDir: "{app}"; Flags: ignoreversion
; FFmpeg's license ships with it.
Source: "{#FFmpegDir}LICENSE.txt"; DestDir: "{app}\licenses"; DestName: "FFmpeg-LICENSE.txt"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Registry]
; ProgID that opens a video in VibePlayer.
Root: HKA; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueName: ""; ValueData: "Video file"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExeName},0"
Root: HKA; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""

; Lists VibePlayer in Settings > Default apps so the user can make it the default.
Root: HKA; Subkey: "Software\{#AppName}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\{#AppName}\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "{#AppName}"
Root: HKA; Subkey: "Software\{#AppName}\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Video player with lossless clip trimming"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "{#AppName}"; ValueData: "Software\{#AppName}\Capabilities"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#AppName}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExeName}"" ""%1"""

; Per extension: offer VibePlayer in "Open with" and in Default apps. Never takes over an
; existing default; Windows only lets the user choose that.
#dim Extensions[16] {".mp4", ".m4v", ".mkv", ".webm", ".mov", ".avi", ".wmv", ".ts", ".m2ts", ".mts", ".flv", ".mpg", ".mpeg", ".3gp", ".ogv", ".vob"}
#define i
#sub RegisterExtension
Root: HKA; Subkey: "Software\Classes\{#Extensions[i]}\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\{#AppName}\Capabilities\FileAssociations"; ValueType: string; ValueName: "{#Extensions[i]}"; ValueData: "{#ProgId}"
Root: HKA; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: "{#Extensions[i]}"; ValueData: ""
#endsub
#for {i = 0; i < DimOf(Extensions); i++} RegisterExtension

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
; In-app updates run silently after VibePlayer has closed itself; start it again afterwards.
Filename: "{app}\{#AppExeName}"; Flags: nowait; Check: WizardSilent
