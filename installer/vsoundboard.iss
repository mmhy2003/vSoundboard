; vSoundboard installer (Inno Setup 7).
; Build with `make installer` (or: cargo build --release, then iscc installer\vsoundboard.iss).
; The version is read from the built exe, which gets it from Cargo.toml, so it's never typed here.

#define AppName "vSoundboard"
#define AppExe "vsoundboard.exe"
#define BuiltExe AddBackslash(SourcePath) + "..\target\release\" + AppExe
#if !FileExists(BuiltExe)
  #error target\release\vsoundboard.exe not found: run `cargo build --release` first
#endif
#define AppVersion GetStringFileInfo(BuiltExe, "ProductVersion")
#if AppVersion == ""
  #error Couldn't read ProductVersion from vsoundboard.exe
#endif
#define AppId "{16385C1B-9E26-44CD-BD5A-3022320628A0}"

; VB-Cable comes from VB-Audio's site at install time (never bundled). The hash pins the exact
; package; if VB-Audio replaces it, the installer falls back to opening their download page.
#ifndef VbCableUrl
  #define VbCableUrl "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip"
#endif
#define VbCableZip "VBCABLE_Driver_Pack45.zip"
#define VbCableSha256 "b950e39f01af1d04ea623c8f6d8eb9b6ea5c477c637295fabf20631c85116bfb"
#define VbCablePage "https://vb-audio.com/Cable/"
; Test-only switches (iscc /D...): ForceVbCablePage offers VB-Cable even when it's installed;
; TestNoVbLaunch downloads and unpacks it but doesn't run VB-Audio's setup.
#define RepoUrl "https://github.com/mmhy2003/vSoundboard"

[Setup]
; AppId must never change: it's how upgrades find the existing install.
AppId={{#AppId}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Mohamed H. Youssef
AppPublisherURL={#RepoUrl}
AppSupportURL={#RepoUrl}/issues
AppUpdatesURL={#RepoUrl}/releases
; Per-user install: {autopf} is %LOCALAPPDATA%\Programs when not running as admin.
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=vSoundboard-Setup-{#AppVersion}
SetupIconFile=..\assets\vsoundboard.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
; We close a running vSoundboard ourselves: closing its window only hides it to the tray.
CloseApplications=no
ArchiveExtraction=auto
Compression=lzma2
SolidCompression=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
; Only shown when a restart is needed, which here means VB-Audio's setup was run.
FinishedRestartLabel=VB-Cable usually appears only after a restart. If you installed VB-Cable, restart your PC before using vSoundboard in Discord.%n%nRestart now?

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "autostart"; Description: "Start vSoundboard when Windows starts (in the tray)"

[Files]
Source: "{#BuiltExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{tmp}\{#VbCableZip}"; DestDir: "{tmp}\vbcable"; Flags: external extractarchive recursesubdirs ignoreversion; Check: VbDownloaded

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch vSoundboard"; Flags: postinstall nowait skipifsilent; Check: not VbLaunched

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  RunValue = 'vSoundboard';
  AppMutex = 'Local\vSoundboard';
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#AppId}_is1';

var
  TasksPageSeen: Boolean;
  VbPage: TInputOptionWizardPage;
  DownloadPage: TDownloadWizardPage;
  VbMissing, VbIsDownloaded, VbWasLaunched, VbFallback: Boolean;

function VbDownloaded: Boolean;
begin
  Result := VbIsDownloaded;
end;

function VbLaunched: Boolean;
begin
  Result := VbWasLaunched;
end;

{ Offer VB-Cable only when it's missing and the box is ticked. Silent installs never install a
  driver unless asked with /VBCABLE=1. }
function WantVbCable: Boolean;
begin
  if WizardSilent then
    Result := VbMissing and (ExpandConstant('{param:VBCABLE|0}') = '1')
  else
    Result := VbMissing and VbPage.Values[0];
end;

{ Detect VB-Cable with the app's own rule (find_cable), by running the exe we're installing. }
procedure DetectVbCable;
var
  ResultCode: Integer;
begin
  ExtractTemporaryFile('{#AppExe}');
  if Exec(ExpandConstant('{tmp}\{#AppExe}'), '--check-vbcable', '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
    VbMissing := ResultCode <> 0
  else
    VbMissing := True;
#ifdef ForceVbCablePage
  VbMissing := True;
#endif
  Log(Format('VB-Cable missing: %d', [Ord(VbMissing)]));
end;

procedure InitializeWizard;
begin
  DetectVbCable;
  VbPage := CreateInputOptionPage(wpSelectTasks,
    'VB-Cable', 'vSoundboard needs VB-Cable to act as a microphone in Discord',
    'VB-CABLE is made by VB-Audio (https://vb-cable.com). It is donationware: free to use, ' +
    'and you can support VB-Audio at vb-cable.com.',
    False, False);
  VbPage.Add('Download and install VB-Cable now (free, from vb-audio.com; requires administrator permission)');
  VbPage.Values[0] := True;
  DownloadPage := CreateDownloadPage(SetupMessage(msgWizardPreparing), SetupMessage(msgPreparingDesc), nil);
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  Result := (PageID = VbPage.ID) and not VbMissing;
end;

{ Fetch and verify VB-Audio's package. Any failure (offline, wrong hash, cancelled) just means
  the fallback: vSoundboard still installs. }
procedure DownloadVbCable;
begin
  try
    if WizardSilent then
      DownloadTemporaryFile('{#VbCableUrl}', '{#VbCableZip}', '{#VbCableSha256}', nil)
    else begin
      DownloadPage.Clear;
      DownloadPage.Add('{#VbCableUrl}', '{#VbCableZip}', '{#VbCableSha256}');
      DownloadPage.Show;
      try
        DownloadPage.Download;
      finally
        DownloadPage.Hide;
      end;
    end;
    VbIsDownloaded := True;
  except
    Log('VB-Cable download failed: ' + GetExceptionMessage);
    VbFallback := True;
  end;
end;

{ Inno calls this for the Ready page in silent installs too, so it's the one place we download. }
function NextButtonClick(CurPageID: Integer): Boolean;
begin
  if (CurPageID = wpReady) and WantVbCable and not VbIsDownloaded then
    DownloadVbCable;
  Result := True;
end;

{ Run VB-Audio's own setup, elevated and visible, and wait for it. }
procedure RunVbCableSetup;
var
  ResultCode: Integer;
  SetupExe: String;
begin
  SetupExe := ExpandConstant('{tmp}\vbcable\VBCABLE_Setup_x64.exe');
  if not FileExists(SetupExe) then begin
    Log('VB-Cable setup missing after extraction: ' + SetupExe);
    VbFallback := True;
    Exit;
  end;
#ifdef TestNoVbLaunch
  Log('TestNoVbLaunch: would run ' + SetupExe);
#else
  if ShellExec('runas', SetupExe, '', ExtractFileDir(SetupExe), SW_SHOW, ewWaitUntilTerminated, ResultCode) then begin
    Log(Format('VB-Cable setup exited with %d', [ResultCode]));
    VbWasLaunched := True;
  end else begin
    Log(Format('VB-Cable setup not run (declined or failed): %s', [SysErrorMessage(ResultCode)]));
    VbFallback := True;
  end;
#endif
end;

procedure ShowVbFallback;
var
  ErrorCode: Integer;
begin
  if SuppressibleMsgBox('VB-Cable wasn''t installed. You can install it later from {#VbCablePage}. ' +
      'vSoundboard''s Devices screen explains the setup.' + #13#10#13#10 + 'Open the download page now?',
      mbInformation, MB_YESNO, IDNO) = IDYES then
    ShellExecAsOriginalUser('open', '{#VbCablePage}', '', '', SW_SHOWNORMAL, ewNoWait, ErrorCode);
end;

function NeedRestart: Boolean;
begin
  Result := VbWasLaunched;
end;

function IsUpgrade: Boolean;
begin
  Result := RegValueExists(HKCU, UninstallKey, 'UninstallString');
end;

{ The Run value's data; must match command_for() in src/autostart.rs. }
function AutostartCommand: String;
begin
  Result := '"' + ExpandConstant('{app}\{#AppExe}') + '" --minimized';
end;

{ End a running vSoundboard (it may be hidden in the tray) and wait for it to go.
  Returns False if it's still running afterwards. }
function CloseRunningApp(Quiet: Boolean): Boolean;
var
  ResultCode, Tries: Integer;
begin
  Result := True;
  if not CheckForMutexes(AppMutex) then
    Exit;
  if not Quiet then
    MsgBox('vSoundboard is running and will be closed.', mbInformation, MB_OK);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#AppExe}', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Tries := 0;
  while CheckForMutexes(AppMutex) and (Tries < 50) do begin
    Sleep(100);
    Tries := Tries + 1;
  end;
  Result := not CheckForMutexes(AppMutex);
end;

{ On upgrade, the autostart checkbox mirrors whether autostart is on right now, so an
  upgrade never undoes the in-app "Start with Windows" toggle. }
procedure CurPageChanged(CurPageID: Integer);
begin
  if (CurPageID = wpSelectTasks) and not TasksPageSeen then begin
    TasksPageSeen := True;
    if IsUpgrade then begin
      if RegValueExists(HKCU, RunKey, RunValue) then
        WizardSelectTasks('autostart')
      else
        WizardSelectTasks('!autostart');
    end;
  end;
end;

{ Whether autostart should be on after this install. A silent upgrade without /TASKS keeps
  the current state, for the same reason as above. }
function WantAutostart: Boolean;
begin
  if WizardSilent and IsUpgrade and (ExpandConstant('{param:TASKS|<none>}') = '<none>') then
    Result := RegValueExists(HKCU, RunKey, RunValue)
  else
    Result := WizardIsTaskSelected('autostart');
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  if CloseRunningApp(WizardSilent) then
    Result := ''
  else
    Result := 'vSoundboard couldn''t be closed. Quit it from its tray icon (right-click, Quit) and run Setup again.';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then begin
    if VbIsDownloaded then
      RunVbCableSetup;
    if VbFallback then
      ShowVbFallback;
    if WantAutostart then
      RegWriteStringValue(HKCU, RunKey, RunValue, AutostartCommand)
    else
      RegDeleteValue(HKCU, RunKey, RunValue);
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  { usAppMutexCheck comes after the "Are you sure?" question, so saying No leaves the app running. }
  if (CurUninstallStep = usAppMutexCheck) and not CloseRunningApp(UninstallSilent) then
    if not UninstallSilent then
      MsgBox('vSoundboard couldn''t be closed. Quit it from its tray icon (right-click, Quit); ' +
        'uninstall will continue but may leave its program file behind.', mbError, MB_OK);
  { Settings in %APPDATA%\vSoundboard are deliberately left alone. }
  if CurUninstallStep = usUninstall then
    RegDeleteValue(HKCU, RunKey, RunValue);
end;
