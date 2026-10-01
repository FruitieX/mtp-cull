; Values are passed by scripts/package-windows.ps1, from Cargo metadata.
#ifndef AppVersion
  #error AppVersion must be supplied
#endif
#ifndef PayloadDir
  #error PayloadDir must be supplied
#endif
#ifndef OutputDir
  #error OutputDir must be supplied
#endif
#ifndef RepoDir
  #error RepoDir must be supplied
#endif
#ifndef PackageAppId
  #define PackageAppId "{{C2897D94-4888-4B11-8B7A-35C5F3A1E87B}"
#endif
#ifndef ShortcutName
  #define ShortcutName "mtp-cull"
#endif

[Setup]
; Keep this ID unchanged across versions so upgrades replace the same installation.
AppId={#PackageAppId}
AppName=mtp-cull
AppVersion={#AppVersion}
AppPublisher=FruitieX
AppPublisherURL=https://github.com/FruitieX/mtp-cull
AppSupportURL=https://github.com/FruitieX/mtp-cull/issues
AppUpdatesURL=https://github.com/FruitieX/mtp-cull/releases
DefaultDirName={localappdata}\Programs\mtp-cull
DefaultGroupName=mtp-cull
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=mtp-cull-{#AppVersion}-windows-x64-setup
SetupIconFile={#RepoDir}\assets\icon.ico
UninstallDisplayIcon={app}\mtp-cull.exe
LicenseFile={#RepoDir}\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
AppMutex=Local\FruitieX.mtp-cull.running
CloseApplications=no
RestartApplications=no
ChangesAssociations=no
ChangesEnvironment=no

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\{#ShortcutName}"; Filename: "{app}\mtp-cull.exe"; WorkingDir: "{app}"; AppUserModelID: "FruitieX.mtp-cull"; Comment: "Review camera photos and back up MTP devices"
Name: "{userdesktop}\{#ShortcutName}"; Filename: "{app}\mtp-cull.exe"; WorkingDir: "{app}"; AppUserModelID: "FruitieX.mtp-cull"; Tasks: desktopicon

[Run]
Filename: "{app}\mtp-cull.exe"; Description: "Launch mtp-cull"; Flags: nowait postinstall skipifsilent

; Deliberately no deletion rules for AppData, presets, review databases or staging.
