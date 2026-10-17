//! Autenticacion transparente e integracion con GitHub Models (Seccion 3).
//!
//! Principio "Zero Plaintext Secrets": no se guardan tokens en archivos. El
//! token se obtiene en memoria con `gh auth token` y solo se usa en el
//! encabezado HTTP hacia GitHub Models. Nunca se expone a la interfaz.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;
use tauri::AppHandle;

use crate::state::AppState;
use crate::ui::emit_log;
use crate::utils::process;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub gh_installed: bool,
    pub git_installed: bool,
    pub gh_version: Option<String>,
    pub git_version: Option<String>,
    pub authenticated: bool,
    pub user: Option<String>,
    pub host: Option<String>,
    pub scopes: Option<String>,
    pub token_kind: Option<String>,
    pub message: String,
}

fn extract_after(text: &str, marker: &str) -> Option<String> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        if let Some(position) = lower.find(&marker.to_lowercase()) {
            let rest = line[position + marker.len()..].trim();
            let cleaned = rest.trim_start_matches(':').trim().trim_matches('\'');
            if !cleaned.is_empty() {
                return Some(cleaned.to_string());
            }
        }
    }
    None
}

/// Usuario activo segun la API de GitHub (requiere sesion).
pub fn current_user() -> Option<String> {
    let out = process::run_cli_any("gh", &["api", "user", "--jq", ".login"], None).ok()?;
    if !out.ok() {
        return None;
    }
    let user = out.stdout.trim().to_string();
    if user.is_empty() {
        None
    } else {
        Some(user)
    }
}

/// Estado de la sesion de GitHub CLI (`gh auth status`).
pub fn status() -> AuthStatus {
    let gh_version = process::version_of("gh");
    let git_version = process::version_of("git");

    let mut result = AuthStatus {
        gh_installed: gh_version.is_some(),
        git_installed: git_version.is_some(),
        gh_version,
        git_version,
        host: Some("github.com".to_string()),
        ..Default::default()
    };

    if !result.gh_installed {
        result.message =
            "GitHub CLI (gh) no esta instalado. El instalador bootstrapper lo agrega automaticamente."
                .to_string();
        return result;
    }

    let output = match process::run_cli_any("gh", &["auth", "status"], None) {
        Ok(output) => output,
        Err(err) => {
            result.message = err;
            return result;
        }
    };

    let combined = output.combined();
    result.scopes = extract_after(&combined, "Token scopes");
    result.token_kind = extract_after(&combined, "Token:");

    if output.ok() {
        result.authenticated = true;
        result.user = current_user();
        result.message = match &result.user {
            Some(user) => format!("Sesion activa como {}", user),
            None => "Sesion activa de GitHub CLI".to_string(),
        };
    } else {
        result.authenticated = false;
        result.message =
            "No hay una sesion de GitHub activa. Pulsa 'Conectar cuenta de GitHub'.".to_string();
    }

    result
}

/// Obtiene el token en memoria (nunca se persiste ni se envia a la UI).
pub fn get_token() -> Result<String, String> {
    let out = process::run_cli_any("gh", &["auth", "token"], None)?;
    if !out.ok() {
        return Err(format!(
            "No se pudo obtener el token de GitHub CLI ({}). Ejecuta 'gh auth login'.",
            out.combined()
        ));
    }
    let token = out.stdout.trim().to_string();
    if token.is_empty() {
        return Err("GitHub CLI devolvio un token vacio. Vuelve a autenticarte.".to_string());
    }
    Ok(token)
}

fn pump<R: Read + Send + 'static>(stream: R, app: AppHandle) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        match line {
            Ok(raw) => {
                let text = raw.replace('\r', "");
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    emit_log(&app, "info", format!("[gh] {}", trimmed));
                }
            }
            Err(_) => break,
        }
    }
}

fn run_login_stream(app: AppHandle) -> Result<(), String> {
    let executable = process::resolve_binary("gh");

    let mut command = Command::new(&executable);
    command.args([
        "auth",
        "login",
        "--web",
        "--git-protocol",
        "https",
        "--hostname",
        "github.com",
    ]);
    // stdin cerrado: `gh` no puede pedir confirmacion interactiva y continua
    // directo al flujo de dispositivo por navegador.
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    process::hide_console(&mut command);

    let mut child = command
        .spawn()
        .map_err(|e| format!("No se pudo iniciar 'gh auth login' ({:?}): {}", executable, e))?;

    let mut readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let handle = app.clone();
        readers.push(std::thread::spawn(move || pump(stdout, handle)));
    }
    if let Some(stderr) = child.stderr.take() {
        let handle = app.clone();
        readers.push(std::thread::spawn(move || pump(stderr, handle)));
    }

    let exit = child
        .wait()
        .map_err(|e| format!("Error esperando a 'gh auth login': {}", e))?;

    for reader in readers {
        let _ = reader.join();
    }

    emit_log(
        &app,
        "info",
        format!(
            "'gh auth login' finalizo con codigo {}.",
            exit.code().unwrap_or(-1)
        ),
    );

    Ok(())
}

/// Dispara el flujo OAuth web de GitHub CLI y espera a que la sesion quede activa.
pub async fn login(app: AppHandle) -> Result<AuthStatus, String> {
    if !process::is_installed("gh") {
        return Err(
            "GitHub CLI (gh) no esta instalado. Reinstala con el bootstrapper o con 'winget install --id GitHub.cli -e'."
                .to_string(),
        );
    }

    emit_log(&app, "info", "Abriendo el flujo OAuth de GitHub en el navegador...");

    let task_app = app.clone();
    let join = tauri::async_runtime::spawn_blocking(move || run_login_stream(task_app));
    join.await
        .map_err(|e| format!("La tarea de autenticacion fallo: {}", e))??;

    // La sesion puede tardar unos segundos en reflejarse.
    for _ in 0..20 {
        let current = status();
        if current.authenticated {
            return Ok(current);
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
    }

    Err(
        "El flujo OAuth termino pero no se detecto una sesion activa. Verifica el navegador y reintenta."
            .to_string(),
    )
}

pub fn logout() -> Result<AuthStatus, String> {
    let out = process::run_cli_any("gh", &["auth", "logout", "--hostname", "github.com"], None)?;
    if !out.ok() {
        let combined = out.combined();
        if !combined.to_lowercase().contains("not logged in") {
            return Err(format!("No se pudo cerrar la sesion: {}", combined));
        }
    }
    Ok(status())
}

/// Diagnostico del entorno (version de binarios, rutas resueltas, WebView2...).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvDoctor {
    pub os: String,
    pub arch: String,
    pub git_version: Option<String>,
    pub git_path: String,
    pub gh_version: Option<String>,
    pub gh_path: String,
    pub gh_authenticated: bool,
    pub config_dir: String,
    pub mock_ai: bool,
    pub webview2: Option<String>,
}

fn webview2_version() -> Option<String> {
    #[cfg(windows)]
    {
        let keys = [
            r"SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
            r"SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
        ];
        for key in keys {
            let out = Command::new("reg")
                .args(["query", &format!("HKLM\\{}", key), "/v", "pv"])
                .stdin(Stdio::null())
                .output();
            if let Ok(output) = out {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = text.lines().find(|line| line.contains("pv")) {
                    if let Some(value) = line.split("REG_SZ").nth(1) {
                        return Some(value.trim().to_string());
                    }
                }
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub fn doctor(state: &AppState) -> EnvDoctor {
    let auth = status();
    EnvDoctor {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        git_version: auth.git_version.clone(),
        gh_version: auth.gh_version.clone(),
        git_path: process::resolve_binary("git").to_string_lossy().to_string(),
        gh_path: process::resolve_binary("gh").to_string_lossy().to_string(),
        gh_authenticated: auth.authenticated,
        config_dir: crate::config::AppConfig::config_dir()
            .to_string_lossy()
            .to_string(),
        mock_ai: state.config_snapshot().mock_ai,
        webview2: webview2_version(),
    }
}

// ---------------------------------------------------------------------------
// Comandos IPC
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn auth_status() -> AuthStatus {
    status()
}

#[tauri::command]
pub async fn auth_login(app: AppHandle) -> Result<AuthStatus, String> {
    login(app).await
}

#[tauri::command]
pub fn auth_logout() -> Result<AuthStatus, String> {
    logout()
}

#[tauri::command]
pub fn env_doctor(state: tauri::State<'_, AppState>) -> EnvDoctor {
    doctor(&state)
}
