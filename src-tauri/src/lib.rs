//! Git Agent Desktop - inicializador modular de Tauri v2.
//!
//! Separacion estricta entre razonamiento y ejecucion:
//! * El modelo (GitHub Models) solo produce bloques SEARCH/REPLACE en JSON.
//! * Este backend (Rust) valida, normaliza y escribe en disco.
//! * Toda publicacion remota exige aprobacion humana previa en la UI.

pub mod agent;
pub mod ai;
pub mod commands;
pub mod config;
pub mod memory;
pub mod secrets;
pub mod state;
pub mod tree;
pub mod updater;
pub mod ui;
pub mod utils;

use tauri::Manager;

use crate::state::AppState;

/// Ajusta tamano y posicion de la ventana al area util del monitor activo.
///
/// Windows aplica escalado por DPI (125 % / 150 % son habituales): una ventana
/// de 1120x700 px logicos mide 1400x875 px fisicos. El centrado automatico de
/// Tauri usa metricas del monitor ya escaladas y puede dejar la ventana fuera
/// de la pantalla, asi que aqui se fija una posicion fisica conocida y se
/// limita el tamano al area disponible.
fn clamp_window_to_monitor(window: &tauri::WebviewWindow) {
    const PREFERRED_WIDTH: f64 = 1120.0;
    const PREFERRED_HEIGHT: f64 = 700.0;
    const OFFSET_X: i32 = 24;
    const OFFSET_Y: i32 = 8;

    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());

    let Some(monitor) = monitor else {
        return;
    };

    let scale = monitor.scale_factor();
    let available = monitor.size().to_logical::<f64>(scale);

    let max_width = (available.width * 0.92).max(900.0);
    let max_height = (available.height * 0.86).max(560.0);

    let width = PREFERRED_WIDTH.min(max_width);
    let height = PREFERRED_HEIGHT.min(max_height);

    let _ = window.set_size(tauri::LogicalSize::new(width, height));
    let _ = window.set_position(tauri::PhysicalPosition::new(OFFSET_X, OFFSET_Y));
}

/// Arranca la aplicacion de escritorio.
pub fn run() {
    let config = config::AppConfig::load();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new(config))
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("Git Agent Desktop");

                // El ajuste se difiere hasta que la ventana esta mapeada: en
                // `setup` el monitor activo y su factor de escala todavia no
                // estan resueltos y el calculo queda desfasado.
                let handle = window.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    clamp_window_to_monitor(&handle);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // --- Autenticacion (GitHub CLI) ---
            commands::auth::auth_status,
            commands::auth::auth_login,
            commands::auth::auth_logout,
            commands::auth::env_doctor,
            // --- Repositorio y Git ---
            commands::git::repo_inspect,
            commands::git::repo_open,
            commands::git::repo_status,
            commands::git::git_create_branch,
            commands::git::git_diff,
            commands::git::git_restore,
            commands::git::git_cleanup_after_merge,
            // --- Motor de parcheo atomico ---
            commands::patch::patch_preview,
            commands::patch::patch_apply,
            // --- Pull Requests (GitHub CLI) ---
            commands::pr::pr_view,
            commands::pr::pr_failed_log,
            commands::pr::pr_comment,
            commands::pr::pr_close,
            // --- Ciclo agentico ---
            commands::agent::agent_run,
            commands::agent::agent_adjust,
            commands::agent::agent_publish,
            commands::agent::agent_abort,
            commands::agent::agent_reset,
            commands::agent::agent_check_pr,
            commands::agent::agent_replan_from_pr,
            commands::agent::agent_history,
            // --- Configuracion y utilidades ---
            commands::config::config_get,
            commands::config::config_set,
            commands::config::providers_list,
            commands::config::ai_test,
            commands::config::ai_models,
            commands::config::secret_status,
            commands::config::secret_set,
            commands::config::secret_clear,
            commands::config::open_external,
            // --- Actualizaciones automaticas ---
            updater::app_version,
            updater::update_check,
            updater::update_apply,
        ])
        .run(tauri::generate_context!())
        .expect("error al iniciar Git Agent Desktop");
}
