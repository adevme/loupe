#ifndef AppVersion
  #error Build with /DAppVersion=x.y.z
#endif

[Setup]
AppId={{6F3A2B71-4C8E-4E57-9C3D-8A1F5B2E7D40}
AppName=Loupe
AppVersion={#AppVersion}
AppVerName=Loupe {#AppVersion}
AppPublisher=adev
AppPublisherURL=https://loupe.adev.me
AppSupportURL=https://github.com/adevme/loupe/issues
AppUpdatesURL=https://loupe.adev.me/#releases
DefaultDirName={localappdata}\Programs\Loupe
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=output
OutputBaseFilename=loupe-setup-{#AppVersion}
SetupIconFile=..\crates\app\assets\icon.ico
UninstallDisplayIcon={app}\Loupe.exe
UninstallDisplayName=Loupe
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
ChangesAssociations=yes
VersionInfoVersion={#AppVersion}

[Tasks]
Name: "desktopicon"; Description: "Put a Loupe shortcut on the desktop"; Flags: unchecked

[Files]
Source: "build\Loupe.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "build\loupe.exe"; DestDir: "{app}\versions\{#AppVersion}"; Flags: ignoreversion
Source: "build\loupe-host.exe"; DestDir: "{app}\versions\{#AppVersion}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Loupe"; Filename: "{app}\Loupe.exe"
Name: "{autodesktop}\Loupe"; Filename: "{app}\Loupe.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\.lp\OpenWithProgids"; ValueType: string; ValueName: "Loupe.Project"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\Loupe.Project"; ValueType: string; ValueName: ""; ValueData: "Loupe project"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Loupe.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\Loupe.exe,0"
Root: HKA; Subkey: "Software\Classes\Loupe.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Loupe.exe"" ""%1"""

[Run]
Filename: "{app}\Loupe.exe"; Description: "Open Loupe"; Flags: nowait postinstall skipifsilent
Filename: "{app}\Loupe.exe"; Parameters: "{code:SongToReopen}"; Flags: nowait; Check: RelaunchAfterUpdate

[UninstallDelete]
Type: filesandordirs; Name: "{app}\versions"
Type: files; Name: "{app}\current"

[Code]
const
  Kept = 3;

function RelaunchAfterUpdate: Boolean;
begin
  Result := ExpandConstant('{param:RELAUNCH|0}') = '1';
end;

function SongToReopen(Unused: String): String;
var
  Song: String;
begin
  Song := ExpandConstant('{param:SONG|}');
  if Song = '' then
    Result := ''
  else
    Result := AddQuotes(Song);
end;

function Part(Version: String; Index: Integer): Integer;
var
  Dot, I: Integer;
begin
  for I := 1 to Index do
  begin
    Dot := Pos('.', Version);
    if Dot = 0 then
    begin
      Result := 0;
      Exit;
    end;
    Delete(Version, 1, Dot);
  end;
  Dot := Pos('.', Version);
  if Dot > 0 then
    Version := Copy(Version, 1, Dot - 1);
  Result := StrToIntDef(Version, -1);
end;

function Newer(A, B: String): Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 0 to 2 do
  begin
    if Part(A, I) <> Part(B, I) then
    begin
      Result := Part(A, I) > Part(B, I);
      Exit;
    end;
  end;
end;

procedure KeepNewest;
var
  Found: TFindRec;
  Names: TArrayOfString;
  Count, I, J, Others: Integer;
  Swap: String;
begin
  Count := 0;
  if FindFirst(ExpandConstant('{app}\versions\*'), Found) then
  begin
    try
      repeat
        if (Found.Attributes and FILE_ATTRIBUTE_DIRECTORY <> 0) and (Found.Name <> '.') and (Found.Name <> '..') and (Found.Name <> '{#AppVersion}') then
        begin
          SetArrayLength(Names, Count + 1);
          Names[Count] := Found.Name;
          Count := Count + 1;
        end;
      until not FindNext(Found);
    finally
      FindClose(Found);
    end;
  end;
  for I := 0 to Count - 1 do
    for J := I + 1 to Count - 1 do
      if Newer(Names[J], Names[I]) then
      begin
        Swap := Names[I];
        Names[I] := Names[J];
        Names[J] := Swap;
      end;
  Others := Kept - 1;
  for I := Others to Count - 1 do
    DelTree(ExpandConstant('{app}\versions\') + Names[I], True, True, True);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    SaveStringToFile(ExpandConstant('{app}\current'), '{#AppVersion}', False);
    KeepNewest;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if UninstallSilent then
      Exit;
    if MsgBox('Also remove Loupe''s settings, themes and plugin list?' + #13#10 + #13#10 + 'Your songs, backups and exports are never removed.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      DelTree(ExpandConstant('{userappdata}\loupe'), True, True, True);
  end;
end;
