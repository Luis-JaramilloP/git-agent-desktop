; ============================================================================
;  Git Agent Desktop - Instalador Bootstrapper (Inno Setup 6)
;  Seccion 5 del plan de arquitectura v2.0
;
;  Flujo:
;   1. Diagnostico previo (git, gh, WebView2)
;   2. Instalacion silenciosa de dependencias faltantes via winget
;   3. Despliegue de binarios
;   4. Accesos directos + registro de desinstalacion
;
;  Compilar:  iscc installer\installer.iss
;  Salida:    dist-installer\GitAgentSetup.exe
; ============================================================================

#define MyAppName "Git Agent Desktop"
#define MyAppVersion "2.2.2"
#define MyAppPublisher "Git Agent Desktop"
#define MyAppExeName "git-agent-desktop.exe"
#define MyAppId "{{8F3A2B94-5C1E-4C0B-9A4E-1B2D3C4E5F60}"

[Setup]
AppId={#MyAppId}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppComments=Automatizacion agentica de Pull Requests con aprobacion humana
DefaultDirName={autopf}\GitAgentDesktop
DefaultGroupName={#MyAppName}
UninstallDisplayIcon={app}\app_icon.ico
UninstallDisplayName={#MyAppName} {#MyAppVersion}
OutputDir=..\dist-installer
OutputBaseFilename=GitAgentSetup
SetupIconFile=assets\app_icon.ico
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
CloseApplications=yes
RestartApplications=no
AllowNoIcons=yes
InfoAfterFile=..\LEEME.txt
DisableWelcomePage=no

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Crear un acceso directo en el Escritorio"; GroupDescription: "Accesos directos:"; Flags: checkedonce

[Files]
; Binario nativo compilado por `cargo build --release` (o `cargo tauri build`)
Source: "..\src-tauri\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
; Icono de la aplicacion (usado por el desinstalador y los accesos directos)
Source: "..\src-tauri\icons\icon.ico"; DestDir: "{app}"; DestName: "app_icon.ico"; Flags: ignoreversion
; Documentacion
Source: "..\README.md"; DestDir: "{app}"; DestName: "README.md"; Flags: ignoreversion skipifsourcedoesntexist
; Guia rapida para el usuario final (tambien se muestra al final del asistente)
Source: "..\LEEME.txt"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\app_icon.ico"
Name: "{group}\Desinstalar {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\app_icon.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Iniciar {#MyAppName}"; Flags: nowait postinstall

[UninstallDelete]
Type: filesandordirs; Name: "{app}"

[Code]
const
  GIT_WINGET_ID = 'Git.Git';
  GH_WINGET_ID = 'GitHub.cli';
  WEBVIEW2_WINGET_ID = 'Microsoft.EdgeWebView2Runtime';
  WEBVIEW2_KEY_WOW = 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  WEBVIEW2_KEY = 'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

var
  DiagnosticsLog: TStringList;

{ --------------------------------------------------------------------------
  Utilidades de diagnostico
  -------------------------------------------------------------------------- }

function UpdateStatus(const Text: String): Boolean;
begin
  Result := True;
  if WizardForm <> nil then
  begin
    WizardForm.StatusLabel.Caption := Text;
    { Fuerza el repintado para que el usuario vea el avance real. }
    WizardForm.StatusLabel.Update;
  end;
end;

procedure LogLine(const Text: String);
begin
  if DiagnosticsLog <> nil then
    DiagnosticsLog.Add(Text);
end;

function RunHidden(const FileName, Params: String; var ResultCode: Integer): Boolean;
begin
  LogLine('Ejecutando: ' + FileName + ' ' + Params);
  Result := Exec(FileName, Params, '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  LogLine('  -> ok=' + IntToStr(Ord(Result)) + ' code=' + IntToStr(ResultCode));
end;

{ Ejecuta un comando via cmd y comprueba su codigo de salida. }
function CommandWorks(const CommandLine: String): Boolean;
var
  ResultCode: Integer;
begin
  Result := RunHidden(ExpandConstant('{cmd}'), '/C ' + CommandLine + ' >NUL 2>&1', ResultCode)
            and (ResultCode = 0);
end;

function WingetAvailable(): Boolean;
begin
  Result := CommandWorks('winget --version');
end;

{ En modo silencioso (actualizacion automatica) nunca se muestran dialogos:
  se registra el aviso y se continua. }
function ContinueAnyway(const Message: String): Boolean;
begin
  if WizardSilent then
  begin
    LogLine('Modo silencioso: se continua sin preguntar.');
    Result := True;
  end
  else
    Result := MsgBox(Message, mbConfirmation, MB_YESNO) = IDYES;
end;

{ --------------------------------------------------------------------------
  Deteccion de dependencias (PATH + rutas absolutas de respaldo, igual que
  el modulo utils/process.rs de la aplicacion)
  -------------------------------------------------------------------------- }

function GitAvailable(): Boolean;
begin
  Result := CommandWorks('git --version')
            or FileExists(ExpandConstant('{pf}\Git\cmd\git.exe'))
            or FileExists(ExpandConstant('{pf32}\Git\cmd\git.exe'))
            or FileExists(ExpandConstant('{localappdata}\Programs\Git\cmd\git.exe'));
end;

function GhAvailable(): Boolean;
begin
  Result := CommandWorks('gh --version')
            or FileExists(ExpandConstant('{pf}\GitHub CLI\gh.exe'))
            or FileExists(ExpandConstant('{localappdata}\Programs\GitHub CLI\gh.exe'));
end;

function WebView2Available(): Boolean;
var
  Version: String;
begin
  Result := RegQueryStringValue(HKLM, WEBVIEW2_KEY_WOW, 'pv', Version)
            or RegQueryStringValue(HKLM, WEBVIEW2_KEY, 'pv', Version)
            or RegQueryStringValue(HKCU, WEBVIEW2_KEY, 'pv', Version);
end;

{ --------------------------------------------------------------------------
  Instalacion silenciosa via winget
  -------------------------------------------------------------------------- }

function InstallPackage(const PackageId, FriendlyName: String): Boolean;
var
  ResultCode: Integer;
  Params: String;
begin
  UpdateStatus('Instalando ' + FriendlyName + ' automaticamente...');

  Params := '-NoProfile -ExecutionPolicy Bypass -Command "winget install --id ' + PackageId +
            ' -e --silent --accept-source-agreements --accept-package-agreements"';

  Result := RunHidden(
    ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    Params,
    ResultCode
  );

  { winget devuelve 0 si instalo; -1978335189 (0x8A15002B) si ya estaba. }
  if (not Result) or ((ResultCode <> 0) and (ResultCode <> -1978335189)) then
    LogLine('  aviso: winget devolvio codigo ' + IntToStr(ResultCode) + ' para ' + PackageId);
end;

{ --------------------------------------------------------------------------
  Diagnostico y bootstrapping de dependencias
  -------------------------------------------------------------------------- }

function CheckDependencies(): Boolean;
begin
  Result := True;

  { Instalacion silenciosa y sin privilegios (actualizacion automatica o
    instalacion por usuario): no se intenta modificar el sistema, porque
    `winget` necesitaria elevacion y bloquearia el proceso sin interfaz. }
  if WizardSilent and (not IsAdminInstallMode()) then
  begin
    LogLine('Modo silencioso sin privilegios: se omiten las instalaciones de sistema.');
    if not GitAvailable() then
      LogLine('aviso: Git no detectado.');
    if not GhAvailable() then
      LogLine('aviso: GitHub CLI no detectado.');
    Exit;
  end;

  { 1. Git for Windows }
  if GitAvailable() then
  begin
    LogLine('Git ya esta instalado.');
    UpdateStatus('Git detectado.');
  end
  else
  begin
    LogLine('Git no detectado: se intentara instalar con winget.');
    if not WingetAvailable() then
    begin
      if not ContinueAnyway(
        'Git no esta instalado y winget no esta disponible en este equipo.' + #13#10 + #13#10 +
        'Descarga Git for Windows desde https://git-scm.com/download/win y vuelve a ejecutar este instalador.' + #13#10 + #13#10 +
        '¿Deseas continuar la instalacion de Git Agent Desktop de todas formas?') then
      begin
        Result := False;
        Exit;
      end;
    end
    else
    begin
      InstallPackage(GIT_WINGET_ID, 'Git for Windows');
      if not GitAvailable() then
      begin
        if not ContinueAnyway(
          'No se pudo instalar Git automaticamente (puede requerir permisos de administrador).' + #13#10 + #13#10 +
          'Instalalo manualmente desde https://git-scm.com/download/win o ejecuta de nuevo el instalador ' +
          'eligiendo "Instalar para todos los usuarios".' + #13#10 + #13#10 +
          '¿Deseas continuar de todas formas?') then
        begin
          Result := False;
          Exit;
        end;
      end
      else
        LogLine('Git instalado correctamente.');
    end;
  end;

  { 2. GitHub CLI }
  if GhAvailable() then
  begin
    LogLine('GitHub CLI ya esta instalado.');
    UpdateStatus('GitHub CLI detectado.');
  end
  else
  begin
    LogLine('GitHub CLI no detectado: se intentara instalar con winget.');
    if WingetAvailable() then
    begin
      InstallPackage(GH_WINGET_ID, 'GitHub CLI');
      if not GhAvailable() then
      begin
        if not ContinueAnyway(
          'No se pudo instalar GitHub CLI automaticamente.' + #13#10 + #13#10 +
          'Instalalo manualmente desde https://cli.github.com/ y vuelve a abrir la aplicacion.' + #13#10 + #13#10 +
          '¿Deseas continuar de todas formas?') then
        begin
          Result := False;
          Exit;
        end;
      end
      else
        LogLine('GitHub CLI instalado correctamente.');
    end;
  end;

  { 3. WebView2 Runtime (lo usa el motor de Tauri) }
  if WebView2Available() then
  begin
    LogLine('WebView2 Runtime presente.');
    UpdateStatus('WebView2 Runtime detectado.');
  end
  else
  begin
    LogLine('WebView2 Runtime no detectado.');
    if WingetAvailable() then
      InstallPackage(WEBVIEW2_WINGET_ID, 'Microsoft Edge WebView2 Runtime');

    if not WebView2Available() then
    begin
      { En modo silencioso no se interrumpe una actualizacion por este aviso. }
      if WizardSilent then
        LogLine('aviso: WebView2 no detectado en modo silencioso.')
      else
        MsgBox(
          'No se detecto Microsoft Edge WebView2 Runtime, necesario para mostrar la interfaz.' + #13#10 + #13#10 +
          'Descargalo desde https://developer.microsoft.com/microsoft-edge/webview2/ antes de abrir la aplicacion.',
          mbInformation, MB_OK);
    end;
  end;
end;

{ --------------------------------------------------------------------------
  Ciclo de vida del instalador
  -------------------------------------------------------------------------- }

function InitializeSetup(): Boolean;
begin
  DiagnosticsLog := TStringList.Create;
  LogLine('=== Git Agent Desktop Setup ' + '{#MyAppVersion}' + ' ===');
  Result := True;
end;

procedure DeinitializeSetup();
begin
  if DiagnosticsLog <> nil then
  begin
    DiagnosticsLog.Free;
    DiagnosticsLog := nil;
  end;
end;

{ Se ejecuta justo antes de copiar los archivos: es el punto correcto para
  validar e instalar dependencias manteniendo la barra de progreso viva. }
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  NeedsRestart := False;

  { Actualizacion: la app ya esta instalada, asi que las dependencias ya se
    validaron en la primera instalacion. Se omiten para no relentizar ni pedir
    confirmaciones durante una actualizacion automatica. }
  if FileExists(ExpandConstant('{app}\{#MyAppExeName}')) then
  begin
    LogLine('Actualizacion detectada: se omiten las comprobaciones de dependencias.');
    UpdateStatus('Actualizando Git Agent Desktop...');
    Exit;
  end;

  UpdateStatus('Comprobando dependencias del sistema...');
  if not CheckDependencies() then
  begin
    Result := 'La instalacion se cancelo porque faltan dependencias obligatorias (Git / GitHub CLI).';
    Exit;
  end;

  UpdateStatus('Dependencias listas. Copiando archivos de la aplicacion...');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    UpdateStatus('Instalacion completada.');
    LogLine('Instalacion completada en ' + ExpandConstant('{app}'));
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  { Se conservan las credenciales de GitHub CLI a proposito: viven en el
    Administrador de Credenciales de Windows y son del usuario, no del producto. }
  if CurUninstallStep = usPostUninstall then
  begin
    LogLine('Desinstalacion completada. Las credenciales de gh se conservan.');
  end;
end;
