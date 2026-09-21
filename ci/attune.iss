; Attune installation script.
;
; Kept alongside upstream's goxlr-utility.iss rather than replacing it, so a
; merge from upstream touches their file and not this one.
;
; Three deliberate differences from upstream's script:
;
;   1. The driver check looks in several places and does not abort. Upstream
;      checks exactly one path, W10_x64, and refuses to install if it is not
;      there. On the development machine the driver is installed at x64 instead,
;      so that check fails on a machine where the driver is present and working.
;      A wrong path should not be indistinguishable from a missing driver.
;
;   2. It never bundles the TC-Helicon driver. No redistribution licence has been
;      granted for it, so the installer points at the vendor download instead.
;
;   3. It looks for Equalizer APO too, and says what will and will not work
;      without it. Attune's headphone side is written entirely through APO, so
;      installing Attune alone gets you the mixer and the microphone and none of
;      the correction -- which would otherwise look like the feature is broken
;      rather than absent.

#define AppName "Attune"
; Attune's own version, and the only place it is written down. The crates
; still carry 1.2.4 -- that is goxlr-utility's number, inherited through the
; workspace, and upstream's packaging reads it back with
; `cargo pkgid -p goxlr-daemon`. Renumbering the crates would change what the
; daemon reports to clients that were written against upstream, so the fork's
; version lives here instead, on the artifact a user actually sees.
#define AppVersion "1.0.0"
#define DriverUrl "https://utility.frostycoolslug.com/update-site/drivers/"
#define ApoUrl "https://sourceforge.net/projects/equalizerapo/"

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
UninstallDisplayIcon={app}\attune-app.exe
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

; The window. Upstream opens its UI by shell-opening the URL, so the
; application arrives as a tab in whatever browser is default. This hosts the
; same page in a window of its own through WebView2 -- the engine Edge already
; uses and which ships with Windows, so there is no second browser bundled
; here. It registers itself with the daemon on first run, which is what makes
; the tray icon open the window too rather than a browser.
Source: "..\target\release\attune-app.exe";     DestDir: "{app}"

; The MCP server. Optional at runtime -- Attune works with no AI configured --
; but shipped so that pointing a model at it needs no separate download.
Source: "..\target\release\attune-mcp.exe";     DestDir: "{app}"

; Diagnostics. Shipped because it has no equivalent in the UI: when the daemon
; will not talk to the device there is by definition no window to look at.
;
; attune-measure and attune-tune are deliberately NOT shipped. Everything they
; do is in the Headphones and Mic Setup tabs now, and a second way in is a
; second thing to keep in step with the first.
Source: "..\target\release\attune-check.exe";   DestDir: "{app}"

Source: "..\LICENSE";                           DestDir: "{app}"
Source: "..\LICENSE-3RD-PARTY";                 DestDir: "{app}"; Flags: isreadme
Source: "..\NOTICE";                            DestDir: "{app}"

[Tasks]
Name: StartOnLogin; Description: "Start Attune automatically when I log in"
Name: DesktopIcon;  Description: "Create a desktop shortcut"; Flags: unchecked

[Icons]
Name: "{group}\Attune";           Filename: "{app}\attune-app.exe"
Name: "{group}\Uninstall Attune"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Attune";     Filename: "{app}\attune-app.exe"; Tasks: DesktopIcon
; {autostartup} follows the install mode, so it lands in the right place whether
; this was installed for one user or for everybody.
Name: "{autostartup}\Attune";     Filename: "{app}\goxlr-daemon.exe"; Tasks: StartOnLogin

[UninstallRun]
; Stop what is running before trying to delete it.
;
; CloseApplications=force is supposed to cover this and does not. Measured:
; uninstalling with the window open removes everything except the two binaries
; that are executing, schedules those for deletion on the next reboot, and
; leaves WebView2's profile behind because its child processes still hold it
; open -- while reporting success throughout.
;
; The process tree, not the process: closing attune-app on its own orphans its
; WebView2 children, and they are what keep the profile locked.
Filename: "{sys}\taskkill.exe"; Parameters: "/IM attune-app.exe /T /F"; \
  Flags: runhidden; RunOnceId: "StopWindow"
Filename: "{sys}\taskkill.exe"; Parameters: "/IM goxlr-daemon.exe /T /F"; \
  Flags: runhidden; RunOnceId: "StopDaemon"

[UninstallDelete]
; WebView2's profile: cache, shader caches, logs. Disposable, and large enough
; to be worth removing -- a test install left 275 files there. Attune's own
; settings under %APPDATA%\Attune are deliberately left alone: corrections
; somebody measured and tuned are not the installer's to throw away, and
; reinstalling should find them again.
Type: filesandordirs; Name: "{localappdata}\Attune\WebView2"

[Run]
Filename: "{app}\attune-app.exe"; Description: "Start Attune"; \
  Flags: shellexec skipifsilent nowait postinstall

[Code]
var
  DriverMissing: Boolean;
  ApoMissing: Boolean;

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

// Equalizer APO's config directory is what matters -- an installation root
// without one is a partial install, and Attune would fail later rather than
// here. This is the same test the application itself makes, deliberately, so
// the installer and the app cannot disagree about whether APO is usable.
function ApoPresent(): Boolean;
begin
  Result := DirExists(ExpandConstant('{commonpf}\EqualizerAPO\config')) or
            DirExists(ExpandConstant('{commonpf32}\EqualizerAPO\config'));
end;

function InitializeSetup(): Boolean;
begin
  DriverMissing := not DriverPresent();
  ApoMissing := not ApoPresent();

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
  if CurStep <> ssPostInstall then
    Exit;

  if DriverMissing then
  begin
    if MsgBox(
      'Open the driver download page now?' + #13#10#13#10 +
      'Attune will not detect your GoXLR until the driver is installed.',
      mbConfirmation, MB_YESNO) = IDYES then
    begin
      ShellExecAsOriginalUser('open', '{#DriverUrl}', '', '', SW_SHOW, ewNoWait, ErrorCode);
    end;
  end;

  // Said plainly, and after the install rather than as a gate: Attune is
  // genuinely useful without APO -- the mixer, the microphone chain, the
  // measurement and the tuner all work -- but every headphone correction is
  // written through APO and simply will not exist without it.
  if ApoMissing then
  begin
    if MsgBox(
      'Equalizer APO was not found.' + #13#10#13#10 +
      'Attune works without it: the mixer, the microphone chain and the ' +
      'measurement tools are all unaffected.' + #13#10#13#10 +
      'What will not work is the Headphones tab. Every headphone correction, ' +
      'the crossfeed and the loudness plugin are applied through Equalizer APO ' +
      'rather than by Attune itself, which is why they add no latency to your ' +
      'game audio. Without it those settings can be chosen and saved, and ' +
      'nothing will reach your headphones.' + #13#10#13#10 +
      'It is free and open source. Installing it needs administrator rights ' +
      'and a reboot, so Attune does not do it for you.' + #13#10#13#10 +
      'Open the Equalizer APO download page now?',
      mbConfirmation, MB_YESNO) = IDYES then
    begin
      ShellExecAsOriginalUser('open', '{#ApoUrl}', '', '', SW_SHOW, ewNoWait, ErrorCode);
    end;
  end;
end;
