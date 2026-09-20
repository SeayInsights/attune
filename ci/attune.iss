; Attune installation script.
;
; Kept alongside upstream's goxlr-utility.iss rather than replacing it, so a
; merge from upstream touches their file and not this one.
;
; Two deliberate differences from upstream's script:
;
;   1. The driver check looks in several places and does not abort. Upstream
;      checks exactly one path, W10_x64, and refuses to install if it is not
;      there. On the development machine the driver is installed at x64 instead,
;      so that check fails on a machine where the driver is present and working.
;      A wrong path should not be indistinguishable from a missing driver.
;
;   2. It never bundles the TC-Helicon driver. No redistribution licence has been
;      granted for it, so the installer points at the vendor download instead.

#define AppName "Attune"
#define AppVersion "0.1.0"
#define DriverUrl "https://utility.frostycoolslug.com/update-site/drivers/"

[Setup]
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Dannis Seay
AppPublisherURL=https://github.com/SeayInsights/attune
WizardStyle=modern
DefaultDirName={autopf}\Attune
DefaultGroupName=Attune
; Install per-user by default, with the option to elevate for all users.
;
; The daemon is a per-user thing: it keeps its settings in the user's AppData
; and its autostart entry in the user's Startup folder. Defaulting to an
; administrative install would put that shortcut in the administrator's Startup
; folder instead of the person's, so "start on login" would quietly never fire
; for whoever actually uses the machine.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
UninstallDisplayIcon={app}\goxlr-daemon.exe
Compression=lzma2
SolidCompression=yes
LicenseFile=..\LICENSE
OutputDir=..\target\installer
OutputBaseFilename=attune-setup
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=..\daemon\resources\goxlr-utility.ico
; Stop a running daemon rather than failing on a locked binary.
CloseApplications=force
RestartApplications=no

[Files]
Source: "..\target\release\goxlr-daemon.exe";   DestDir: "{app}"
Source: "..\target\release\goxlr-client.exe";   DestDir: "{app}"
Source: "..\target\release\goxlr-defaults.exe"; DestDir: "{app}"
Source: "..\target\release\goxlr-launcher.exe"; DestDir: "{app}"
; The MCP server. Optional at runtime -- Attune works with no AI configured --
; but shipped so that pointing a model at it needs no separate download.
Source: "..\target\release\attune-mcp.exe";     DestDir: "{app}"

Source: "..\LICENSE";                           DestDir: "{app}"
Source: "..\LICENSE-3RD-PARTY";                 DestDir: "{app}"; Flags: isreadme
Source: "..\NOTICE";                            DestDir: "{app}"

[Tasks]
Name: StartOnLogin; Description: "Start Attune automatically when I log in"

[Icons]
Name: "{group}\Attune";           Filename: "{app}\goxlr-launcher.exe"
Name: "{group}\Uninstall Attune"; Filename: "{uninstallexe}"
; {autostartup} follows the install mode, so it lands in the right place whether
; this was installed for one user or for everybody.
Name: "{autostartup}\Attune";     Filename: "{app}\goxlr-daemon.exe"; Tasks: StartOnLogin

[Run]
Filename: "{app}\goxlr-launcher.exe"; Description: "Start Attune"; \
  Flags: shellexec skipifsilent nowait postinstall

[Code]
var
  DriverMissing: Boolean;

// The driver has moved between versions, so check the places it is known to
// live rather than asserting one.
function DriverPresent(): Boolean;
var
  Candidates: array[0..3] of String;
  I: Integer;
begin
  Candidates[0] := ExpandConstant('{commonpf}\TC-Helicon\GoXLR_Audio_Driver\x64\goxlr_audioapi_x64.dll');
  Candidates[1] := ExpandConstant('{commonpf}\TC-Helicon\GoXLR_Audio_Driver\W10_x64\goxlr_audioapi_x64.dll');
  Candidates[2] := ExpandConstant('{commonpf}\TC-HELICON\GoXLR_Audio_Driver\x64\goxlr_audioapi_x64.dll');
  Candidates[3] := ExpandConstant('{commonpf}\TC-HELICON\GoXLR_Audio_Driver\W10_x64\goxlr_audioapi_x64.dll');

  Result := False;
  for I := 0 to 3 do
  begin
    if FileExists(Candidates[I]) then
    begin
      Result := True;
      Exit;
    end;
  end;
end;

function InitializeSetup(): Boolean;
begin
  DriverMissing := not DriverPresent();

  if DriverMissing then
  begin
    // A warning, not a refusal. The check is a heuristic over paths that have
    // changed before, and being wrong about it should not stop someone
    // installing software they asked for.
    if MsgBox(
      'The TC-Helicon GoXLR driver was not found in any of its usual locations.' + #13#10#13#10 +
      'Attune needs it to talk to the device, and cannot bundle it -- no ' +
      'redistribution licence has been granted, so it has to come from the vendor.' + #13#10#13#10 +
      'If you already have the official GoXLR app installed, you have the driver ' +
      'and this check is simply looking in the wrong place.' + #13#10#13#10 +
      'Continue with the installation?',
      mbConfirmation, MB_YESNO) = IDNO then
    begin
      Result := False;
      Exit;
    end;
  end;

  Result := True;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ErrorCode: Integer;
begin
  if (CurStep = ssPostInstall) and DriverMissing then
  begin
    if MsgBox(
      'Open the driver download page now?' + #13#10#13#10 +
      'Attune will not detect your GoXLR until the driver is installed.',
      mbConfirmation, MB_YESNO) = IDYES then
    begin
      ShellExecAsOriginalUser('open', '{#DriverUrl}', '', '', SW_SHOW, ewNoWait, ErrorCode);
    end;
  end;
end;
