//! Memoria reflexiva persistente (Seccion 11 del plan).
//!
//! Guarda la bitacora de intentos por repositorio: resumen, diff descartado,
//! evidencia del revisor y resultado. Se usa para construir el contexto de
//! auto-correccion y para la vista "Intentos" de la interfaz.
//!
//! Nota de seguridad: aqui nunca se persisten tokens ni credenciales; solo
//! texto de la tarea y diffs del propio repositorio.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AttemptRecord {
    pub attempt: u32,
    pub timestamp: String,
    pub model: String,
    pub task: String,
    pub branch: String,
    pub summary: String,
    pub diff: String,
    pub files: Vec<String>,
    /// pendiente | aprobado | rechazado | abortado | merged
    pub outcome: String,
    pub reviewer_feedback: Option<String>,
    pub pr_url: Option<String>,
    pub usage_total_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStore {
    pub repo_path: String,
    pub attempts: Vec<AttemptRecord>,
}

fn repo_key(repo: &Path) -> String {
    // FNV-1a de 64 bits sobre la ruta en minusculas: identificador estable y
    // seguro para nombre de archivo.
    let normalized = repo.to_string_lossy().to_lowercase().replace('\\', "/");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in normalized.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", hash)
}

impl MemoryStore {
    pub fn path_for(repo: &Path) -> PathBuf {
        AppConfig::memory_dir().join(format!("{}.json", repo_key(repo)))
    }

    pub fn load(repo: &Path) -> Self {
        let path = Self::path_for(repo);
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<MemoryStore>(&raw).ok())
            .unwrap_or_else(|| MemoryStore {
                repo_path: repo.to_string_lossy().to_string(),
                attempts: Vec::new(),
            })
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = AppConfig::memory_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("No se pudo crear {:?}: {}", dir, e))?;
        let path = dir.join(format!(
            "{}.json",
            repo_key(Path::new(&self.repo_path))
        ));
        let raw = serde_json::to_string_pretty(self)
            .map_err(|e| format!("No se pudo serializar la memoria: {}", e))?;
        std::fs::write(&path, raw).map_err(|e| format!("No se pudo guardar la memoria: {}", e))
    }

    pub fn push(&mut self, record: AttemptRecord) -> Result<(), String> {
        self.attempts.push(record);
        // Limita el crecimiento del archivo a los 40 intentos mas recientes.
        if self.attempts.len() > 40 {
            let excess = self.attempts.len() - 40;
            self.attempts.drain(0..excess);
        }
        self.save()
    }

    pub fn last(&self) -> Option<&AttemptRecord> {
        self.attempts.last()
    }

    /// Texto compacto con los ultimos `n` intentos fallidos, para inyectar como
    /// contexto de auto-correccion.
    pub fn corrective_context(&self, n: usize, max_diff_chars: usize) -> String {
        let mut out = String::new();
        let failed: Vec<&AttemptRecord> = self
            .attempts
            .iter()
            .filter(|a| matches!(a.outcome.as_str(), "rechazado" | "abortado"))
            .collect();

        for record in failed.iter().rev().take(n) {
            out.push_str(&format!(
                "### Intento {} ({})\nResumen: {}\n",
                record.attempt, record.timestamp, record.summary
            ));
            if let Some(feedback) = &record.reviewer_feedback {
                if !feedback.trim().is_empty() {
                    out.push_str(&format!("Evidencia del revisor:\n{}\n", feedback.trim()));
                }
            }
            if !record.diff.trim().is_empty() {
                let diff = record.diff.trim();
                let snippet: String = diff.chars().take(max_diff_chars).collect();
                out.push_str(&format!("Diff descartado:\n```diff\n{}\n```\n", snippet));
            }
            out.push_str("---\n");
        }
        out
    }
}

/// Marca de tiempo local legible.
pub fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}
