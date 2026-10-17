//! Estado compartido de la aplicacion (gestionado por Tauri con `.manage()`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::commands::patch::FileSnapshot;
use crate::config::AppConfig;

/// Consumo de tokens reportado por GitHub Models.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl Usage {
    pub fn accumulate(&mut self, other: &Usage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
    }
}

/// Evento de bitacora enviado a la interfaz.
pub const EVENT_LOG: &str = "agent://log";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub level: String,
    pub message: String,
    pub ts: String,
}

/// Emisor de una linea de bitacora (lo implementa la capa de interfaz).
pub type LogSink = Arc<dyn Fn(LogLine) + Send + Sync + 'static>;

/// Publicador de eventos agnosticо del runtime y de la interfaz.
///
/// El ciclo agentico depende solo de esta abstraccion, de modo que puede
/// ejercitarse en pruebas de integracion sin ventana ni runtime de escritorio
/// (`Reporter::silent()`), o conectado a Tauri mediante `crate::ui`.
#[derive(Clone, Default)]
pub struct Reporter {
    sink: Option<LogSink>,
}

impl Reporter {
    /// Reporter sin UI: descarta los eventos (pruebas y ejecucion headless).
    pub fn silent() -> Self {
        Self { sink: None }
    }

    pub fn with_sink(sink: LogSink) -> Self {
        Self { sink: Some(sink) }
    }

    pub fn log(&self, level: &str, message: impl Into<String>) {
        let line = LogLine {
            level: level.to_string(),
            message: message.into(),
            ts: crate::memory::now(),
        };
        if let Some(sink) = &self.sink {
            sink(line);
        }
    }
}

/// Estado de la tarea agentica en curso (se muestra en la UI y alimenta la
/// memoria reflexiva de la Seccion 11 del plan).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub active: bool,
    pub repo_path: Option<String>,
    pub task: Option<String>,
    pub model: Option<String>,
    pub issue: Option<u64>,
    /// Rama aislada `agent/...` sobre la que se aplican los cambios.
    pub branch: Option<String>,
    /// Rama base del repositorio (main/master) usada por `gh pr create --base`.
    pub base_branch: Option<String>,
    /// Numero de ciclo correctivo en curso (guardrail Max Retries = 3).
    pub attempts: u32,
    pub max_attempts: u32,
    /// fase: idle | planning | applying | review | published | aborted
    pub phase: String,
    pub summary: Option<String>,
    pub selected_files: Vec<String>,
    pub touched_files: Vec<String>,
    pub commit_message: Option<String>,
    pub pr_title: Option<String>,
    pub pr_body: Option<String>,
    pub pr_url: Option<String>,
    pub pr_number: Option<u64>,
    pub last_diff: String,
    pub usage: Option<Usage>,
    /// Copia exacta del contenido previo de cada archivo tocado. Permite
    /// deshacer el trabajo del agente sin destruir cambios del usuario.
    #[serde(skip)]
    pub snapshot: Vec<FileSnapshot>,
}

/// Estado global gestionado por Tauri.
pub struct AppState {
    repo: Mutex<Option<PathBuf>>,
    session: Mutex<SessionState>,
    config: Mutex<AppConfig>,
    abort_requested: AtomicBool,
    http: reqwest::Client,
}

impl AppState {
    pub fn new(config: AppConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .user_agent(concat!("GitAgentDesktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            repo: Mutex::new(None),
            session: Mutex::new(SessionState::default()),
            config: Mutex::new(config),
            abort_requested: AtomicBool::new(false),
            http,
        }
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    // ------------------------- Repositorio -------------------------

    pub fn repo_path(&self) -> Option<PathBuf> {
        self.repo.lock().ok().and_then(|g| g.clone())
    }

    pub fn set_repo(&self, path: Option<PathBuf>) {
        if let Ok(mut guard) = self.repo.lock() {
            *guard = path;
        }
    }

    // ------------------------- Configuracion ------------------------

    pub fn config_snapshot(&self) -> AppConfig {
        self.config
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| AppConfig::default())
    }

    pub fn set_config(&self, cfg: AppConfig) {
        if let Ok(mut guard) = self.config.lock() {
            *guard = cfg;
        }
    }

    // ------------------------- Sesion -------------------------------

    pub fn session_snapshot(&self) -> SessionState {
        self.session.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn update_session<F>(&self, f: F)
    where
        F: FnOnce(&mut SessionState),
    {
        if let Ok(mut guard) = self.session.lock() {
            f(&mut guard);
        }
    }

    pub fn clear_session(&self) {
        if let Ok(mut guard) = self.session.lock() {
            *guard = SessionState::default();
        }
    }

    // ------------------------- Cancelacion --------------------------

    pub fn request_abort(&self) {
        self.abort_requested.store(true, Ordering::SeqCst);
    }

    pub fn clear_abort(&self) {
        self.abort_requested.store(false, Ordering::SeqCst);
    }

    pub fn is_aborted(&self) -> bool {
        self.abort_requested.load(Ordering::SeqCst)
    }

    pub fn ensure_not_aborted(&self) -> Result<(), String> {
        if self.is_aborted() {
            Err("Ejecucion cancelada por el usuario.".to_string())
        } else {
            Ok(())
        }
    }
}
