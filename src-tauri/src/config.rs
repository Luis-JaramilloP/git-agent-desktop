//! Configuracion persistente.
//!
//! Nota de seguridad: este archivo NUNCA contiene credenciales. La clave del
//! proveedor de IA se guarda cifrada con DPAPI en `secrets.rs` y el token de
//! GitHub vive en el Administrador de Credenciales de Windows via `gh`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ai::providers;

/// Endpoint por defecto cuando el usuario no define ninguno: Ollama local
/// (gratuito y sin clave). GitHub Models fue retirado el 30 de julio de 2026.
pub const DEFAULT_PROVIDER: &str = "ollama";
pub const DEFAULT_MODEL: &str = "qwen2.5-coder:7b";

/// Canal de actualizaciones por defecto: `latest.json` del ultimo release de
/// GitHub. El workflow de CI lo publica en cada tag. Un valor vacio en la
/// configuracion significa "usar el canal local de %APPDATA%".
pub const DEFAULT_UPDATE_URL: &str =
    "https://github.com/Luis-JaramilloP/git-agent-desktop/releases/latest/download/latest.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    /// Id del proveedor de inferencia (`ollama`, `gemini`, `groq`, ...).
    pub provider: String,
    pub model: String,
    /// URL base propia. Vacio = la del preset del proveedor.
    pub base_url: String,
    pub temperature: f32,
    pub max_tokens: u32,
    /// Tamano maximo de un archivo enviado al modelo.
    pub max_file_bytes: usize,
    /// Presupuesto total de caracteres del contexto de la etapa 2.
    pub max_context_chars: usize,
    /// Numero maximo de rutas en el arbol enviado en la etapa 1.
    pub max_files_in_tree: usize,
    /// Guardrail de la Seccion 11: ciclos correctivos permitidos.
    pub max_attempts: u32,
    /// Canal de actualizaciones: URL de `latest.json` o carpeta que lo contenga.
    /// Vacio = canal local `%APPDATA%\GitAgentDesktop\updates`.
    pub update_url: String,
    /// Modo de pruebas sin red ni proveedor (ver README).
    pub mock_ai: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            provider: DEFAULT_PROVIDER.to_string(),
            model: DEFAULT_MODEL.to_string(),
            base_url: String::new(),
            temperature: 0.1,
            max_tokens: 8192,
            max_file_bytes: 300_000,
            max_context_chars: 120_000,
            max_files_in_tree: 400,
            max_attempts: 3,
            update_url: DEFAULT_UPDATE_URL.to_string(),
            mock_ai: false,
        }
    }
}

impl AppConfig {
    /// `%APPDATA%\GitAgentDesktop` (o equivalente en el SO).
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("GitAgentDesktop")
    }

    pub fn config_file() -> PathBuf {
        Self::config_dir().join("config.json")
    }

    pub fn memory_dir() -> PathBuf {
        Self::config_dir().join("memory")
    }

    /// Carga la configuracion de disco y aplica los overrides de entorno.
    ///
    /// Migracion: los archivos anteriores a la retirada de GitHub Models no
    /// tienen la clave `provider`. En ese caso se restablece el proveedor por
    /// defecto (Ollama local) y se descartan la URL y el modelo antiguos.
    pub fn load() -> Self {
        let mut cfg = match std::fs::read_to_string(Self::config_file()) {
            Ok(text) => {
                let legacy = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .map(|value| value.get("provider").is_none())
                    .unwrap_or(true);

                let mut parsed = serde_json::from_str::<AppConfig>(&text).unwrap_or_default();
                if legacy {
                    parsed.provider = DEFAULT_PROVIDER.to_string();
                    parsed.base_url = String::new();
                    parsed.model = DEFAULT_MODEL.to_string();
                    parsed.mock_ai = false;
                    let _ = parsed.save();
                }
                parsed
            }
            Err(_) => Self::default(),
        };

        cfg.apply_env();
        cfg
    }

    pub fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("GAD_PROVIDER") {
            if !v.trim().is_empty() {
                self.provider = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GAD_MODEL") {
            if !v.trim().is_empty() {
                self.model = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GAD_BASE_URL") {
            if !v.trim().is_empty() {
                self.base_url = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GAD_UPDATE_URL") {
            if !v.trim().is_empty() {
                self.update_url = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("GAD_MOCK_AI") {
            self.mock_ai = matches!(v.trim(), "1" | "true" | "TRUE" | "yes");
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = Self::config_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("No se pudo crear {:?}: {}", dir, e))?;
        let raw = serde_json::to_string_pretty(self)
            .map_err(|e| format!("No se pudo serializar la configuracion: {}", e))?;
        std::fs::write(Self::config_file(), raw)
            .map_err(|e| format!("No se pudo escribir la configuracion: {}", e))
    }

    /// URL base efectiva: la personalizada si existe; si no, la del preset.
    pub fn effective_base_url(&self) -> String {
        let custom = self.base_url.trim().trim_end_matches('/');
        if !custom.is_empty() {
            return custom.to_string();
        }
        providers::find(&self.provider)
            .map(|preset| preset.base_url)
            .unwrap_or_default()
    }

    /// Modelo efectivo: el configurado o el primero sugerido por el preset.
    pub fn effective_model(&self) -> String {
        let model = self.model.trim();
        if !model.is_empty() {
            return model.to_string();
        }
        providers::find(&self.provider)
            .and_then(|preset| preset.suggested_models.first().cloned())
            .unwrap_or_default()
    }

    pub fn sanitize(mut self) -> Self {
        self.temperature = self.temperature.clamp(0.0, 1.0);
        self.max_tokens = self.max_tokens.clamp(256, 32_000);
        self.max_file_bytes = self.max_file_bytes.clamp(1_000, 5_000_000);
        self.max_context_chars = self.max_context_chars.clamp(2_000, 1_500_000);
        self.max_files_in_tree = self.max_files_in_tree.clamp(20, 5_000);
        self.max_attempts = self.max_attempts.clamp(1, 10);

        if self.provider.trim().is_empty() || providers::find(&self.provider).is_none() {
            self.provider = DEFAULT_PROVIDER.to_string();
        }
        if self.model.trim().is_empty() {
            self.model = self
                .effective_model()
                .chars()
                .take(200)
                .collect::<String>();
        }
        self.base_url = self.base_url.trim().to_string();
        self
    }
}
