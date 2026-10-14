; DeskChalk setup wizard (Inno Setup 6).
; Build the app first (cargo build --release), then:
;   iscc /DAppVersion=0.2.1 installer\DeskChalk.iss
; Output: target\installer\DeskChalk_Setup_v0.2.1.exe
;
; Installs for the current user only (no admin rights), shows up in Settings > Apps and
; Programs and Features, updates in place, and uninstalls cleanly (boxes and settings too).

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define AppGuid "3F9C2B7E-8D41-4A6C-B5E2-7A1D9C0E4F68"

[Setup]
AppId={{{#AppGuid}}
AppName=DeskChalk
AppVersion={#AppVersion}
AppVerName=DeskChalk {#AppVersion}
AppPublisher=WuzNab
AppPublisherURL=https://github.com/WuzNab/DeskChalk
AppSupportURL=https://github.com/WuzNab/DeskChalk/issues
AppUpdatesURL=https://github.com/WuzNab/DeskChalk/releases
AppComments=Draw labeled boxes on your desktop wallpaper, behind your icons.
DefaultDirName={autopf}\DeskChalk
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\target\installer
OutputBaseFilename=DeskChalk_Setup_v{#AppVersion}
SetupIconFile=..\assets\deskchalk.ico
WizardSmallImageFile=wizard-small-55.bmp,wizard-small-110.bmp
UninstallDisplayIcon={app}\DeskChalk.exe
UninstallDisplayName=DeskChalk
LicenseFile=..\LICENSE.txt
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=no
VersionInfoVersion={#AppVersion}
VersionInfoProductName=DeskChalk
VersionInfoDescription=DeskChalk Setup

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startup"; Description: "Start DeskChalk when Windows starts"; GroupDescription: "Options:"
Name: "startmenu"; Description: "Add DeskChalk to the Start menu"; GroupDescription: "Options:"; Flags: unchecked
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Options:"; Flags: unchecked

[Files]
Source: "..\target\release\DeskChalk.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\DeskChalk"; Filename: "{app}\DeskChalk.exe"; Tasks: startmenu
Name: "{autodesktop}\DeskChalk"; Filename: "{app}\DeskChalk.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "DeskChalk"; ValueData: """{app}\DeskChalk.exe"" --autostart"; Tasks: startup; Flags: uninsdeletevalue

[InstallDelete]
; Start menu shortcut left by DeskChalk 0.1's built-in installer.
Type: files; Name: "{userprograms}\DeskChalk.lnk"

[UninstallDelete]
; Boxes and settings.
Type: filesandordirs; Name: "{userappdata}\DeskChalk"
; The program folder, even if an older DeskChalk created it before this setup did.
Type: dirifempty; Name: "{app}"

[Run]
Filename: "{app}\DeskChalk.exe"; Description: "Launch DeskChalk"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\DeskChalk.exe"; Parameters: "--quit"; Flags: runhidden waituntilterminated; RunOnceId: "QuitDeskChalk"

[Code]
const
  UninstallRoot = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\';
  OldUninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\DeskChalk';
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  ApprovedKey = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run';

var
  ModePage: TInputOptionWizardPage;

function HasPreviousSetup(): Boolean;
begin
  Result := RegKeyExists(HKCU, UninstallRoot + '{{#AppGuid}}_is1')
    or RegKeyExists(HKCU, OldUninstallKey)
    or DirExists(ExpandConstant('{userappdata}\DeskChalk'));
end;

procedure InitializeWizard();
begin
  if HasPreviousSetup() then
  begin
    ModePage := CreateInputOptionPage(wpSelectTasks,
      'DeskChalk is already set up',
      'Keep your boxes, or start fresh?',
      'Choose how to install this version, then click Next.',
      True, False);
    ModePage.Add('Update: keep my boxes and settings (recommended)');
    ModePage.Add('Clean reinstall: delete all my boxes and settings first');
    ModePage.SelectedValueIndex := 0;
  end;
end;

function CleanReinstall(): Boolean;
begin
  Result := (ModePage <> nil) and (ModePage.SelectedValueIndex = 1);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Result := '';
  // Close any running DeskChalk (installed or portable) so files can be replaced.
  ExtractTemporaryFile('DeskChalk.exe');
  Exec(ExpandConstant('{tmp}\DeskChalk.exe'), '--quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
  begin
    // DeskChalk 0.1 registered itself under its own key; keep a single Apps entry.
    RegDeleteKeyIncludingSubkeys(HKCU, OldUninstallKey);
    if CleanReinstall() then
      DelTree(ExpandConstant('{userappdata}\DeskChalk'), True, True, True);
  end;
  if CurStep = ssPostInstall then
  begin
    if WizardIsTaskSelected('startup') then
      // Re-enable it if Task Manager had disabled it.
      RegDeleteValue(HKCU, ApprovedKey, 'DeskChalk')
    else
      RegDeleteValue(HKCU, RunKey, 'DeskChalk');
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    // DeskChalk's own "Start with Windows" toggle may have written these.
    RegDeleteValue(HKCU, RunKey, 'DeskChalk');
    RegDeleteValue(HKCU, ApprovedKey, 'DeskChalk');
    RegDeleteKeyIncludingSubkeys(HKCU, OldUninstallKey);
  end;
end;
