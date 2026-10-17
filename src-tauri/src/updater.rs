//! Actualizacion automatica sin volver al instalador.
//!
//! Flujo:
//! 1. Se lee un manifiesto `latest.json` (URL http/https o carpeta local; por
//!    defecto `%APPDATA%\GitAgentDesktop\updates\latest.json`).
//! 2. Si la version del manifiesto es mayor que la actual, la UI ofrece
//!    actualizar y muestra el progreso (`update://progress`).
//! 3. Se descarga el instalador, se verifica su SHA-256 y se lanza en modo
//!    silencioso (`/SILENT`), que muestra la ventana pequeña de progreso de
//!    Inno Setup. La app se cierra para liberar el ejecutable y el propio
//!    instalador la vuelve a abrir al terminar.
//! 4. Si el directorio de instalacion no es escribible (p. ej. Program Files),
//!    se lanza con `runas` para que Windows pida elevacion una vez.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

use crate::config::AppConfig;
use crate::memory::now;
use crate::ui::emit_log;
use crate::state::AppState;

/// Version compilada del binario en ejecucion.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Evento de progreso hacia la interfaz.
pub const EVENT_PROGRESS: &str = "update://progress";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateManifest {
    pub version: String,
    pub notes: String,
    /// URL o ruta (relativa al manifiesto) del instalador.
    pub url: String,
    /// SHA-256 en hexadecimal del instalador.
    pub sha256: String,
    pub size: u64,
    pub mandatory: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub mandatory: bool,
    pub size: u64,
    pub source: String,
    /// false cuando no hay canal de actualizaciones configurado.
    pub configured: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub phase: String,
    pub percent: u32,
    pub message: String,
}

fn emit(app: &AppHandle, phase: &str, percent: u32, message: impl Into<String>) {
    let _ = app.emit(
        EVENT_PROGRESS,
        UpdateProgress {
            phase: phase.to_string(),
            percent,
            message: message.into(),
        },
    );
}

/// Carpeta del canal local por defecto.
pub fn default_channel_file() -> PathBuf {
    AppConfig::config_dir().join("updates").join("latest.json")
}

/// Origen efectivo del manifiesto (configurado o canal local).
pub fn manifest_source(config: &AppConfig) -> String {
    let custom = config.update_url.trim();
    if !custom.is_empty() {
        return custom.to_string();
    }
    default_channel_file().to_string_lossy().to_string()
}

fn split_base(source: &str) -> String {
    if source.starts_with("http://") || source.starts_with("https://") {
        source
            .rsplit_once('/')
            .map(|(base, _)| base.to_string())
            .unwrap_or_default()
    } else {
        Path::new(source)
            .parent()
            .map(|parent| parent.to_string_lossy().to_string())
            .unwrap_or_default()
    }
}

/// Resuelve la URL del instalador respecto al origen del manifiesto.
pub fn resolve_artifact(base: &str, url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.contains("://") {
        return trimmed.to_string();
    }
    if Path::new(trimmed).is_absolute() {
        return trimmed.to_string();
    }
    if base.is_empty() {
        return trimmed.to_string();
    }
    if base.starts_with("http://") || base.starts_with("https://") {
        format!(
            "{}/{}",
            base.trim_end_matches('/'),
            trimmed.trim_start_matches('/')
        )
    } else {
        Path::new(base)
            .join(trimmed)
            .to_string_lossy()
            .to_string()
    }
}

/// Compara versiones con formato `x.y.z` (los sufijos no numericos se ignoran).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parts(value: &str) -> Vec<u64> {
        value
            .trim()
            .trim_start_matches('v')
            .split('.')
            .map(|piece| {
                piece
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .filter(|digits| !digits.is_empty())
                    .and_then(|digits| digits.parse::<u64>().ok())
                    .unwrap_or(0)
            })
            .collect()
    }

    let left = parts(candidate);
    let right = parts(current);
    let length = left.len().max(right.len());

    for index in 0..length {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

async fn fetch_manifest(
    http: &reqwest::Client,
    config: &AppConfig,
) -> Result<(UpdateManifest, String), String> {
    let source = manifest_source(config);
    let base = split_base(&source);

    let text = if source.starts_with("http://") || source.starts_with("https://") {
        let response = http
            .get(&source)
            .timeout(Duration::from_secs(25))
            .send()
            .await
            .map_err(|e| format!("No se pudo consultar el canal de actualizaciones: {}", e))?;

        if !response.status().is_success() {
            return Err(format!(
                "El canal de actualizaciones respondio HTTP {}.",
                response.status().as_u16()
            ));
        }
        response
            .text()
            .await
            .map_err(|e| format!("Respuesta ilegible del canal: {}", e))?
    } else {
        std::fs::read_to_string(&source)
            .map_err(|e| format!("No se pudo leer el manifiesto {}: {}", source, e))?
    };

    let manifest: UpdateManifest = serde_json::from_str(&text)
        .map_err(|e| format!("El manifiesto de actualizacion no es JSON valido: {}", e))?;

    Ok((manifest, base))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect()
}

/// Consulta el canal y decide si hay una version nueva.
pub async fn check(state: &AppState) -> Result<UpdateInfo, String> {
    let config = state.config_snapshot();
    let source = manifest_source(&config);
    let configured = !config.update_url.trim().is_empty() || default_channel_file().exists();

    if !configured {
        return Ok(UpdateInfo {
            current_version: CURRENT_VERSION.to_string(),
            available: false,
            version: None,
            notes: None,
            mandatory: false,
            size: 0,
            source,
            configured: false,
        });
    }

    let (manifest, _base) = fetch_manifest(state.http(), &config).await?;

    if manifest.version.trim().is_empty() {
        return Err("El manifiesto no indica la version.".to_string());
    }

    let available = is_newer(&manifest.version, CURRENT_VERSION);

    Ok(UpdateInfo {
        current_version: CURRENT_VERSION.to_string(),
        available,
        version: Some(manifest.version.clone()),
        notes: Some(manifest.notes.clone()),
        mandatory: manifest.mandatory,
        size: manifest.size,
        source,
        configured: true,
    })
}

fn install_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

/// Comprueba si el usuario puede escribir en la carpeta de instalacion.
fn install_dir_writable() -> bool {
    let dir = install_dir();
    if dir.as_os_str().is_empty() {
        return false;
    }
    let probe = dir.join(".gad-write-probe.tmp");
    match std::fs::write(&probe, b"x") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Descarga el instalador, verifica el SHA-256 y lanza la actualizacion.
pub async fn download_and_install(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let config = state.config_snapshot();
    emit(app, "check", 2, "Consultando el canal de actualizaciones...");

    let (manifest, base) = fetch_manifest(state.http(), &config).await?;

    if !is_newer(&manifest.version, CURRENT_VERSION) {
        return Err(format!(
            "Ya tienes la ultima version ({}).",
            CURRENT_VERSION
        ));
    }

    let artifact = resolve_artifact(&base, &manifest.url);
    if artifact.trim().is_empty() {
        return Err("El manifiesto no incluye la URL del instalador.".to_string());
    }

    emit(app, "download", 10, format!("Descargando la version {}...", manifest.version));
    emit_log(
        app,
        "agent",
        format!("Actualizacion {}: descargando {}", manifest.version, artifact),
    );

    let bytes = if artifact.starts_with("http://") || artifact.starts_with("https://") {
        let response = state
            .http()
            .get(&artifact)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|e| format!("No se pudo descargar la actualizacion: {}", e))?;

        if !response.status().is_success() {
            return Err(format!(
                "La descarga de la actualizacion devolvio HTTP {}.",
                response.status().as_u16()
            ));
        }
        response
            .bytes()
            .await
            .map_err(|e| format!("Error leyendo la descarga: {}", e))?
            .to_vec()
    } else {
        std::fs::read(&artifact)
            .map_err(|e| format!("No se pudo leer el instalador {}: {}", artifact, e))?
    };

    if bytes.len() < 100_000 {
        return Err(format!(
            "El instalador descargado es sospechosamente pequeno ({} bytes).",
            bytes.len()
        ));
    }

    emit(app, "verify", 60, "Verificando la firma del instalador...");

    if !manifest.sha256.trim().is_empty() {
        let actual = sha256_hex(&bytes);
        if !actual.eq_ignore_ascii_case(manifest.sha256.trim()) {
            return Err(format!(
                "La firma no coincide: se esperaba {} y se obtuvo {}. Actualizacion cancelada.",
                manifest.sha256.trim(),
                actual
            ));
        }
        emit_log(app, "ok", "SHA-256 del instalador verificado.");
    } else {
        emit_log(
            app,
            "warn",
            "El manifiesto no incluye SHA-256: se omite la verificacion de integridad.",
        );
    }

    let target = std::env::temp_dir().join(format!(
        "GitAgentSetup-{}.exe",
        manifest.version.replace(|c: char| !c.is_ascii_alphanumeric() && c != '.', "_")
    ));

    std::fs::write(&target, &bytes)
        .map_err(|e| format!("No se pudo guardar el instalador en {:?}: {}", target, e))?;

    emit(app, "launch", 85, "Aplicando los cambios...");
    emit_log(
        app,
        "agent",
        format!(
            "Lanzando el instalador {} en modo silencioso.",
            target.to_string_lossy()
        ),
    );

    launch_installer(&target)?;

    // Se cede un instante para que el instalador arranque y luego la app se
    // cierra: el instalador reemplaza el .exe y la vuelve a abrir al final.
    tokio::time::sleep(Duration::from_millis(400)).await;
    emit(app, "exit", 100, "Reiniciando la aplicacion...");
    tokio::time::sleep(Duration::from_millis(200)).await;

    std::process::exit(0);
}

/// Parametros de Inno Setup para una actualizacion desatendida pero visible:
/// `/SILENT` muestra la ventana de progreso (la "ventanita" de los cambios).
fn installer_args() -> &'static str {
    "/SILENT /CLOSEAPPLICATIONS /NORESTART /SUPPRESSMSGBOXES"
}

fn launch_installer(exe: &Path) -> Result<(), String> {
    let elevate = !install_dir_writable();

    if elevate {
        // Program Files u otra ruta protegida: Windows pedira elevacion.
        shell_execute_runas(exe, installer_args())
    } else {
        spawn_detached(exe, installer_args())
    }
}

#[cfg(windows)]
fn spawn_detached(exe: &Path, args: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    let mut command = std::process::Command::new(exe);
    for argument in args.split_whitespace() {
        command.arg(argument);
    }
    command
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("No se pudo lanzar el instalador: {}", e))
}

#[cfg(not(windows))]
fn spawn_detached(exe: &Path, args: &str) -> Result<(), String> {
    let mut command = std::process::Command::new(exe);
    for argument in args.split_whitespace() {
        command.arg(argument);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("No se pudo lanzar el instalador: {}", e))
}

#[cfg(windows)]
fn shell_execute_runas(exe: &Path, args: &str) -> Result<(), String> {
    use std::ffi::c_void;

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let operation = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let parameters = wide(args);

    // SAFETY: los punteros apuntan a buffers vivos durante la llamada.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    } as usize;

    if result <= 32 {
        return Err(format!(
            "No se pudo iniciar el instalador con permisos de administrador (codigo {}). \
             Puede que hayas cancelado el aviso de Windows: vuelve a intentarlo y acepta la elevacion.",
            result
        ));
    }

    Ok(())
}

#[cfg(not(windows))]
fn shell_execute_runas(exe: &Path, args: &str) -> Result<(), String> {
    spawn_detached(exe, args)
}

/// Marca de tiempo reutilizable en la UI de actualizaciones.
pub fn timestamp() -> String {
    now()
}

// ---------------------------------------------------------------------------
// Comandos IPC
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub current_version: String,
    pub channel: String,
    pub configured: bool,
    pub install_dir: String,
    pub requires_elevation: bool,
}

#[tauri::command]
pub fn app_version(state: tauri::State<'_, AppState>) -> VersionInfo {
    let config = state.config_snapshot();
    VersionInfo {
        current_version: CURRENT_VERSION.to_string(),
        channel: manifest_source(&config),
        configured: !config.update_url.trim().is_empty() || default_channel_file().exists(),
        install_dir: install_dir().to_string_lossy().to_string(),
        requires_elevation: !install_dir_writable(),
    }
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<UpdateInfo, String> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    check(state.inner()).await
}

#[tauri::command]
pub async fn update_apply(app: AppHandle) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    download_and_install(&app, state.inner()).await
}
