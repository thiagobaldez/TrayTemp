; Instalador do TrayTemp (Inno Setup 6). Gere com build-installer.ps1, que passa:
;   /DExePath=<TrayTemp.exe compilado>  /DPawnIOSetup=<PawnIO_setup.exe>

#ifndef ExePath
  #error Defina ExePath (use build-installer.ps1)
#endif
#ifndef PawnIOSetup
  #error Defina PawnIOSetup (use build-installer.ps1)
#endif

#define AppVersion GetStringFileInfo(ExePath, "ProductVersion")

[Setup]
AppId={{6B2F3C1E-8A4D-4E7B-9C2A-5D1F0E3B7A94}
AppName=TrayTemp
AppVersion={#AppVersion}
AppVerName=TrayTemp {#AppVersion}
DefaultDirName={autopf}\TrayTemp
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\dist
OutputBaseFilename=TrayTemp-Setup-{#AppVersion}
SetupIconFile=..\assets\TrayTemp.ico
UninstallDisplayIcon={app}\TrayTemp.exe
UninstallDisplayName=TrayTemp
VersionInfoVersion={#AppVersion}
VersionInfoProductName=TrayTemp
VersionInfoDescription=Instalador do TrayTemp
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; O próprio script fecha o app (ver PrepareToInstall).
CloseApplications=no

[Languages]
Name: "pt"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "autostart"; Description: "Iniciar o TrayTemp com o Windows"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#ExePath}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\PawnIO.Modules-COPYING.txt"; DestDir: "{app}"; Flags: ignoreversion
; Driver PawnIO: só é extraído e instalado se ainda não estiver presente.
Source: "{#PawnIOSetup}"; DestDir: "{tmp}"; Flags: deleteafterinstall; Check: NeedsPawnIO

[Icons]
Name: "{autoprograms}\TrayTemp"; Filename: "{app}\TrayTemp.exe"
Name: "{autodesktop}\TrayTemp"; Filename: "{app}\TrayTemp.exe"; Tasks: desktopicon

[Run]
; Cria ou remove a tarefa agendada de inicialização conforme a opção escolhida.
Filename: "{app}\TrayTemp.exe"; Parameters: "--autostart on"; Tasks: autostart; Flags: runhidden waituntilterminated
Filename: "{app}\TrayTemp.exe"; Parameters: "--autostart off"; Tasks: not autostart; Flags: runhidden waituntilterminated
; runascurrentuser: abre já elevado, sem um segundo prompt do UAC.
Filename: "{app}\TrayTemp.exe"; Description: "Abrir o TrayTemp agora"; Flags: postinstall nowait skipifsilent runascurrentuser

[UninstallRun]
Filename: "{app}\TrayTemp.exe"; Parameters: "--exit"; Flags: runhidden waituntilterminated; RunOnceId: "ExitApp"
Filename: "{app}\TrayTemp.exe"; Parameters: "--autostart off"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveAutostart"

[Code]
var
  PawnIONeedsRestart: Boolean;

function NeedsPawnIO: Boolean;
begin
  Result := not RegKeyExists(HKLM64, 'SYSTEM\CurrentControlSet\Services\PawnIO');
end;

{ Fecha o TrayTemp em execução (inclusive cópias em outras pastas) para poder substituir o .exe. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  ExtractTemporaryFile('TrayTemp.exe');
  Exec(ExpandConstant('{tmp}\TrayTemp.exe'), '--exit', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM TrayTemp.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
begin
  if (CurStep = ssPostInstall) and NeedsPawnIO then
  begin
    WizardForm.StatusLabel.Caption := 'Instalando o driver PawnIO...';
    if not Exec(ExpandConstant('{tmp}\PawnIO_setup.exe'), '-install -silent', '', SW_HIDE, ewWaitUntilTerminated, Code) then
      Code := -1;
    if Code = 3010 then
      PawnIONeedsRestart := True
    else if Code <> 0 then
      SuppressibleMsgBox('Não foi possível instalar o driver PawnIO (código ' + IntToStr(Code) + ').' + #13#10 +
        'Sem ele o TrayTemp não lê a temperatura da CPU. Instale manualmente em https://pawnio.eu',
        mbError, MB_OK, IDOK);
  end;
end;

function NeedRestart: Boolean;
begin
  Result := PawnIONeedsRestart;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  { As configurações ficam em %APPDATA%\TrayTemp; pergunta antes de apagar. }
  if (CurUninstallStep = usPostUninstall) and DirExists(ExpandConstant('{userappdata}\TrayTemp')) then
    if SuppressibleMsgBox('Remover também as configurações do TrayTemp?', mbConfirmation, MB_YESNO, IDNO) = IDYES then
      DelTree(ExpandConstant('{userappdata}\TrayTemp'), True, True, True);
end;
