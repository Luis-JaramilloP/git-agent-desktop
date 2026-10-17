//! Puente entre el backend agnistico y la interfaz de Tauri.
//!
//! Concentra aqui las dependencias de UI (emisor de eventos) para que el
//! ciclo agentico y el estado permanezcan testeables sin runtime de escritorio.

use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use crate::state::{LogLine, Reporter, EVENT_LOG};

/// Emite una linea de bitacora a la UI (`agent://log`).
pub fn emit_log(app: &AppHandle, level: &str, message: impl Into<String>) {
    let line = LogLine {
        level: level.to_string(),
        message: message.into(),
        ts: crate::memory::now(),
    };
    let _ = app.emit(EVENT_LOG, line);
}

/// Reporter conectado a la ventana de la aplicacion.
pub fn make_reporter(app: AppHandle) -> Reporter {
    Reporter::with_sink(Arc::new(move |line: LogLine| {
        let _ = app.emit(EVENT_LOG, line);
    }))
}
