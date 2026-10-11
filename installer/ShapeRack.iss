; Compile with Inno Setup 6.3 or newer. The PowerShell build script supplies
; AppVersion/FileVersion from Cargo.toml and an absolute RepoRoot.
#ifndef AppVersion
  #define AppVersion "0.1.1"
#endif
#ifndef FileVersion
  #define FileVersion "0.1.1.0"
#endif
#ifndef RepoRoot
  #define RepoRoot AddBackslash(SourcePath) + ".."
#endif

; TestRoot creates a separate installer that requires no elevation and cannot
; write plugin files, documentation, or its uninstaller outside this directory.
#ifdef TestRoot
  #define ProductName "ShapeRack (staging test)"
  #define ProductId "PRISMM.ShapeRack.StagingTest"
  #define InstallRoot AddBackslash(TestRoot) + "PRISMM\ShapeRack"
  #define Vst3Root AddBackslash(TestRoot) + "VST3\ShapeRack.vst3"
  #define ClapRoot AddBackslash(TestRoot) + "CLAP"
  #define InstallerOutput "dist\test-installer"
  #define InstallerFilename "ShapeRack-" + AppVersion + "-Setup-TEST"
#else
  #define ProductName "ShapeRack"
  #define ProductId "PRISMM.ShapeRack"
  #define InstallRoot "{autopf}\PRISMM\ShapeRack"
  #define Vst3Root "{commoncf64}\VST3\ShapeRack.vst3"
  #define ClapRoot "{commoncf64}\CLAP"
  #define InstallerOutput "dist"
  #define InstallerFilename "ShapeRack-" + AppVersion + "-Setup"
#endif

[Setup]
AppId={#ProductId}
AppName={#ProductName}
AppVersion={#AppVersion}
AppVerName={#ProductName} {#AppVersion}
AppPublisher=PRISMM
AppPublisherURL=https://github.com/0xPRISMM/shaperack
AppSupportURL=https://github.com/0xPRISMM/shaperack/issues
AppUpdatesURL=https://github.com/0xPRISMM/shaperack/releases
DefaultDirName={#InstallRoot}
DisableDirPage=yes
DisableProgramGroupPage=yes
UsePreviousAppDir=no
UsePreviousTasks=no
SourceDir={#RepoRoot}
OutputDir={#RepoRoot}\{#InstallerOutput}
OutputBaseFilename={#InstallerFilename}
VersionInfoVersion={#FileVersion}
VersionInfoDescription=ShapeRack audio effect installer
LicenseFile={#RepoRoot}\LICENSE
InfoAfterFile={#RepoRoot}\installer\AfterInstall.txt
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
MinVersion=10.0
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
UninstallDisplayName={#ProductName} {#AppVersion}
UninstallFilesDir={app}\Uninstall
CloseApplications=no
RestartApplications=no
SetupLogging=yes
#ifdef TestRoot
PrivilegesRequired=lowest
CreateUninstallRegKey=no
#else
PrivilegesRequired=admin
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Types]
Name: "standard"; Description: "FL Studio / VST3"
Name: "full"; Description: "VST3 and CLAP"
Name: "custom"; Description: "Custom installation"; Flags: iscustom

[Components]
Name: "vst3"; Description: "VST3 effect plugin (FL Studio)"; Types: standard full custom; Flags: fixed
Name: "clap"; Description: "CLAP effect plugin (for compatible hosts)"; Types: full

[Files]
Source: "target\bundled\ShapeRack.vst3\*"; DestDir: "{#Vst3Root}"; Components: vst3; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "target\bundled\ShapeRack.clap"; DestDir: "{#ClapRoot}"; Components: clap; Flags: ignoreversion
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "INSTALL.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "VALIDATION.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "THIRD_PARTY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "installer\AfterInstall.txt"; DestDir: "{app}"; Flags: ignoreversion

[Messages]
WelcomeLabel2=This installs ShapeRack, a drawable modulation effect, for 64-bit audio hosts.%n%nClose FL Studio and other audio hosts before continuing. ShapeRack will be available after you scan for new plugins.
SelectComponentsLabel2=VST3 is the format to use in FL Studio. Select CLAP only if another host supports that format.

[Code]
function InitializeSetup: Boolean;
begin
  Result := True;
#ifdef TestRoot
  Log('STAGING TEST: plugin files and documentation are redirected to {#TestRoot}');
#endif
end;

#ifdef TestRoot
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  { /DIR can override DefaultDirName even when the destination page is hidden. }
  if CompareText(AddBackslash(ExpandConstant('{app}')),
    AddBackslash('{#InstallRoot}')) <> 0 then
    Result := 'This staging installer can only install inside its compiled TestRoot. ' +
      'Remove the /DIR override or use the exact staging application directory.';
end;
#endif
