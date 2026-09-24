; aura.iss - Inno Setup script for the Aura toolchain.
; Produces AuraSetup-<version>.exe: a wizard that installs into
; %LOCALAPPDATA%\Programs\Aura with no admin rights, adds aura to the
; user PATH, optionally associates .aura files and installs the VS
; Code extension.
;
; Build:  iscc /DVersion=0.1.0-alpha /DDistDir=dist\aura-0.1.0-alpha-windows-x86_64 installer\aura.iss
; (defaults match the values below so plain `iscc aura.iss` works too)

#ifndef Version
  #define Version "0.1.0-alpha"
#endif
#ifndef DistDir
  #define DistDir "..\dist\aura-" + Version + "-x86_64-pc-windows-msvc"
#endif

#define AppName "Aura"
#define AppPublisher "Aura contributors"
#define AppURL "https://tchoungageslin-blip.github.io/aura"
#define AppExe "aura.exe"

[Setup]
AppId={{7A0AA11A-4E4C-4A0D-9E5A-00A11CE00A11}
AppName={#AppName}
AppVersion={#Version}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}
AppUpdatesURL={#AppURL}
; per-user install - no admin
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\Aura
DefaultGroupName=Aura
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE-MIT
OutputDir=.
OutputBaseFilename=AuraSetup-{#Version}
SetupIconFile=..\assets\icon\aura.ico
WizardSmallImageFile=..\assets\icon\wizard.bmp
Compression=lzma2/ultra64
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\aura.exe
ChangesEnvironment=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french";  MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "modifypath"; Description: "Add Aura to your PATH (recommended)"; GroupDescription: "Integration:"
Name: "assocaura";  Description: "Associate .aura files with the Aura toolchain"; GroupDescription: "Integration:"; Flags: unchecked
Name: "vscodeext";  Description: "Install the VS Code extension (requires 'code' on PATH)"; GroupDescription: "Integration:"; Flags: unchecked

[Files]
Source: "{#DistDir}\aura.exe";          DestDir: "{app}"; Flags: ignoreversion
Source: "{#DistDir}\aura_runtime.lib";  DestDir: "{app}"; Flags: ignoreversion
Source: "{#DistDir}\lld-link.exe";      DestDir: "{app}"; Flags: ignoreversion
Source: "{#DistDir}\*.vsix";            DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#DistDir}\README.txt";        DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "{#DistDir}\LICENSE-*";         DestDir: "{app}"; Flags: ignoreversion
Source: "{#DistDir}\bench\*";           DestDir: "{app}\bench"; Flags: ignoreversion
Source: "{#DistDir}\docs\*";            DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs
Source: "{#DistDir}\licenses\*";        DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "..\assets\icon\aura.ico";      DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Aura documentation"; Filename: "{app}\docs\intro.md"
Name: "{group}\Uninstall Aura";     Filename: "{uninstallexe}"

[Registry]
; .aura file association (task-gated)
Root: HKCU; Subkey: "Software\Classes\.aura"; ValueType: string; ValueData: "Aura.SourceFile"; Flags: uninsdeletekey; Tasks: assocaura
Root: HKCU; Subkey: "Software\Classes\Aura.SourceFile"; ValueType: string; ValueData: "Aura source file"; Flags: uninsdeletekey; Tasks: assocaura
Root: HKCU; Subkey: "Software\Classes\Aura.SourceFile\DefaultIcon"; ValueType: string; ValueData: "{app}\aura.ico"; Tasks: assocaura
Root: HKCU; Subkey: "Software\Classes\Aura.SourceFile\shell\open\command"; ValueType: string; ValueData: """{app}\aura.exe"" run ""%1"""; Tasks: assocaura

[Run]
; Offered on the finish page: open README, install VS Code ext.
Filename: "{app}\README.txt"; Description: "Read the README"; Flags: postinstall shellexec skipifsilent unchecked
Filename: "{cmd}"; Parameters: "/c for %f in (""{app}\*.vsix"") do code --install-extension ""%f"""; Description: "Install the VS Code extension now"; Flags: postinstall skipifsilent unchecked; Tasks: vscodeext

[Code]
{ ---- user PATH editing -------------------------------------------- }
{ The user PATH lives in HKCU\Environment. We add/remove the install  }
{ dir and broadcast WM_SETTINGCHANGE via ChangesEnvironment=yes.      }

function PathHasEntry(path, entry: string): Boolean;
var
  hay: string;
begin
  hay := ';' + Uppercase(path) + ';';
  entry := ';' + Uppercase(entry) + ';';
  { normalize trailing backslash variants: 'C:\Dir\;' -> 'C:\Dir;' }
  StringChangeEx(hay, '\;', ';', True);
  StringChangeEx(entry, '\;', ';', True);
  Result := Pos(entry, hay) > 0;
end;

procedure AddToPath();
var
  path: string;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', path) then
    path := '';
  if PathHasEntry(path, ExpandConstant('{app}')) then
    exit;
  if path <> '' then path := path + ';';
  path := path + ExpandConstant('{app}');
  RegWriteExpandStringValue(HKCU, 'Environment', 'Path', path);
end;

procedure RemoveFromPath();
var
  path, entry, remain: string;
  i, n: Integer;
  parts: TArrayOfString;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', path) then
    exit;
  entry := Uppercase(ExpandConstant('{app}'));
  remain := '';
  { split on ';' and drop our directory (either slash flavor) }
  n := 0;
  SetLength(parts, 0);
  while True do begin
    i := Pos(';', path);
    if i = 0 then begin
      SetLength(parts, n + 1); parts[n] := path; Inc(n); break;
    end;
    SetLength(parts, n + 1); parts[n] := Copy(path, 1, i - 1); Inc(n);
    path := Copy(path, i + 1, Length(path));
  end;
  for i := 0 to n - 1 do begin
    if (Uppercase(parts[i]) <> entry) and (Uppercase(parts[i]) <> entry + '\') then begin
      if remain <> '' then remain := remain + ';';
      remain := remain + parts[i];
    end;
  end;
  RegWriteExpandStringValue(HKCU, 'Environment', 'Path', remain);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('modifypath') then
    AddToPath();
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath();
end;
