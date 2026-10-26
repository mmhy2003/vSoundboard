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
Compression=lzma2
SolidCompression=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "autostart"; Description: "Start vSoundboard when Windows starts (in the tray)"

[Files]
Source: "{#BuiltExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch vSoundboard"; Flags: postinstall nowait skipifsilent

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  RunValue = 'vSoundboard';
  AppMutex = 'Local\vSoundboard';
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#AppId}_is1';

var
  TasksPageSeen: Boolean;

function IsUpgrade: Boolean;
begin
  Result := RegValueExists(HKCU, UninstallKey, 'UninstallString');
end;

{ The Run value's data; must match command_for() in src/autostart.rs. }
function AutostartCommand: String;
begin
  Result := '"' + ExpandConstant('{app}\{#AppExe}') + '" --minimized';
end;

{ End a running vSoundboard (it may be hidden in the tray) and wait for it to go. }
procedure CloseRunningApp(Quiet: Boolean);
var
  ResultCode, Tries: Integer;
begin
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
  CloseRunningApp(WizardSilent);
  Result := '';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then begin
    if WantAutostart then
      RegWriteStringValue(HKCU, RunKey, RunValue, AutostartCommand)
    else
      RegDeleteValue(HKCU, RunKey, RunValue);
  end;
end;

function InitializeUninstall: Boolean;
begin
  CloseRunningApp(UninstallSilent);
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  { Settings in %APPDATA%\vSoundboard are deliberately left alone. }
  if CurUninstallStep = usUninstall then
    RegDeleteValue(HKCU, RunKey, RunValue);
end;
