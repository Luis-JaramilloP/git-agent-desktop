//! Comandos IPC del ciclo agentico (capa fina sobre `crate::agent`).

use tauri::{AppHandle, Manager, State};

use crate::agent::{self, AbortResult, AgentOutcome, PrCheck, PublishRequest, PublishResult, RunRequest};
use crate::memory::AttemptRecord;
use crate::state::AppState;

#[tauri::command]
pub async fn agent_run(app: AppHandle, request: RunRequest) -> Result<AgentOutcome, String> {
    let state = app.state::<AppState>();
    let reporter = crate::ui::make_reporter(app.clone());
    agent::run(&reporter, state.inner(), request).await
}

#[tauri::command]
pub async fn agent_adjust(app: AppHandle, feedback: String) -> Result<AgentOutcome, String> {
    let state = app.state::<AppState>();
    let reporter = crate::ui::make_reporter(app.clone());
    agent::adjust(&reporter, state.inner(), &feedback).await
}

#[tauri::command]
pub fn agent_publish(
    app: AppHandle,
    state: State<'_, AppState>,
    request: PublishRequest,
) -> Result<PublishResult, String> {
    agent::publish(&crate::ui::make_reporter(app), &state, request)
}

#[tauri::command]
pub fn agent_abort(app: AppHandle, state: State<'_, AppState>) -> Result<AbortResult, String> {
    agent::abort(&crate::ui::make_reporter(app), &state)
}

#[tauri::command]
pub fn agent_reset(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let _ = &app;
    agent::reset(&state);
    Ok("Sesion reiniciada.".to_string())
}

#[tauri::command]
pub fn agent_check_pr(app: AppHandle, state: State<'_, AppState>) -> Result<PrCheck, String> {
    agent::check_pr(&crate::ui::make_reporter(app), &state)
}

#[tauri::command]
pub async fn agent_replan_from_pr(app: AppHandle) -> Result<AgentOutcome, String> {
    let state = app.state::<AppState>();
    let reporter = crate::ui::make_reporter(app.clone());
    agent::replan_from_pr(&reporter, state.inner()).await
}

#[tauri::command]
pub fn agent_history(state: State<'_, AppState>) -> Result<Vec<AttemptRecord>, String> {
    agent::history(&state)
}
