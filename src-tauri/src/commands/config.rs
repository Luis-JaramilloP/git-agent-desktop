//! Comandos de configuracion, catalogo de proveedores, clave cifrada y
//! apertura de enlaces.

use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::ai::client::{list_models, test_connection, AiTest};
use crate::ai::providers::{self, ProviderPreset};
use crate::config::AppConfig;
use crate::secrets::{self, SecretStatus};
use crate::state::AppState;

#[tauri::command]
pub fn config_get(state: State<'_, AppState>) -> AppConfig {
    state.config_snapshot()
}

#[tauri::command]
pub fn config_set(state: State<'_, AppState>, config: AppConfig) -> Result<AppConfig, String> {
    let mut sanitized = config.sanitize();

    // Al cambiar de proveedor sin URL explicita se adopta la del preset, para
    // que el usuario no tenga que copiarla a mano.
    if sanitized.base_url.trim().is_empty() {
        if let Some(preset) = providers::find(&sanitized.provider) {
            sanitized.base_url = preset.base_url;
        }
    }

    sanitized.save()?;
    state.set_config(sanitized.clone());
    Ok(sanitized)
}

/// Catalogo de proveedores compatibles con OpenAI.
#[tauri::command]
pub fn providers_list() -> Vec<ProviderPreset> {
    providers::presets()
}

/// Lista los modelos que ofrece el proveedor configurado (`GET /models`).
#[tauri::command]
pub async fn ai_models(app: AppHandle) -> Result<Vec<String>, String> {
    let state = app.state::<AppState>();
    list_models(state.inner()).await
}

/// Envia un prompt minimo al proveedor configurado y devuelve latencia y respuesta.
#[tauri::command]
pub async fn ai_test(app: AppHandle) -> Result<AiTest, String> {
    let state = app.state::<AppState>();
    test_connection(state.inner()).await
}

#[tauri::command]
pub fn secret_status() -> SecretStatus {
    secrets::status()
}

/// Guarda la clave cifrada con DPAPI (cadena vacia = borrarla).
#[tauri::command]
pub fn secret_set(key: String) -> Result<SecretStatus, String> {
    secrets::save(&key)
}

#[tauri::command]
pub fn secret_clear() -> Result<SecretStatus, String> {
    secrets::clear()
}

/// Abre una URL en el navegador predeterminado (solo http/https).
#[tauri::command]
pub fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err("Solo se permiten enlaces http(s).".to_string());
    }

    app.opener()
        .open_url(trimmed.to_string(), None::<&str>)
        .map_err(|e| format!("No se pudo abrir el enlace: {}", e))
}
