//! Motor de parcheo atomico SEARCH / REPLACE (Secciones 7 y 12.1 del plan).
//!
//! Garantias:
//! * El modelo nunca escribe en disco: solo propone bloques.
//! * Se normalizan los saltos de linea (CRLF<->LF) antes de comparar y se
//!   restaura la convencion original del archivo antes de guardar, de modo que
//!   el diff no se ensucie con cambios de fin de linea.
//! * La validacion de TODOS los bloques ocurre antes de escribir el primero:
//!   si un bloque no encaja, no se toca ningun archivo.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};

use crate::state::AppState;

/// Un bloque de reemplazo propuesto por el modelo.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchBlock {
    #[serde(alias = "file_path", alias = "path", alias = "file", alias = "filename")]
    pub file_path: String,
    /// Fragmento literal a localizar. Vacio = crear archivo nuevo / anexar.
    #[serde(
        default,
        alias = "search_content",
        alias = "search",
        alias = "old",
        alias = "old_string",
        alias = "find"
    )]
    pub search_content: String,
    #[serde(
        default,
        alias = "replace_content",
        alias = "replace",
        alias = "new",
        alias = "new_string",
        alias = "replace_with"
    )]
    pub replace_content: String,
}

/// Cambio resultante sobre un archivo del repositorio.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub file_path: String,
    pub created: bool,
    pub additions: usize,
    pub deletions: usize,
    pub diff: String,
    pub bytes_written: usize,
}

/// Copia exacta del estado previo de un archivo, para poder deshacer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSnapshot {
    pub file_path: String,
    pub existed: bool,
    pub content: Option<String>,
}

/// Normaliza CRLF -> LF (Seccion 12.1).
pub fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Valida y normaliza la ruta relativa que propone el modelo.
/// Rechaza rutas absolutas, `..`, rutas vacias y cualquier cosa dentro de `.git`.
pub fn sanitize_rel_path(repo_root: &Path, raw: &str) -> Result<(PathBuf, String), String> {
    let cleaned = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .replace('\\', "/");

    if cleaned.is_empty() {
        return Err("Bloque de parche sin ruta de archivo.".to_string());
    }

    let bytes = cleaned.as_bytes();
    if cleaned.starts_with('/') || (bytes.len() > 1 && bytes[1] == b':') {
        return Err(format!("Ruta absoluta rechazada (debe ser relativa al repositorio): {}", raw));
    }

    let mut segments: Vec<&str> = Vec::new();
    for segment in cleaned.split('/') {
        match segment {
            "" | "." => continue,
            ".." => return Err(format!("Ruta con '..' rechazada: {}", raw)),
            other => segments.push(other),
        }
    }

    if segments.is_empty() {
        return Err(format!("Ruta de archivo invalida: {}", raw));
    }
    if segments[0].eq_ignore_ascii_case(".git") {
        return Err(format!("Modificacion de '.git' rechazada: {}", raw));
    }

    let rel = segments.join("/");
    let full = repo_root.join(&rel);

    // Si el archivo ya existe, comprueba que no escape del repositorio por
    // enlaces simbolicos.
    if full.exists() {
        let canon_root = fs::canonicalize(repo_root)
            .map_err(|e| format!("No se pudo resolver el repositorio: {}", e))?;
        let canon_full =
            fs::canonicalize(&full).map_err(|e| format!("No se pudo resolver '{}': {}", rel, e))?;
        if !canon_full.starts_with(&canon_root) {
            return Err(format!("La ruta '{}' escapa del repositorio.", rel));
        }
    }

    Ok((full, rel))
}

fn read_text_file(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("No se pudo leer {:?}: {}", path, e))?;
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err(format!(
            "El archivo {:?} parece binario; el motor de parcheo solo edita texto.",
            path
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

/// Aplica un bloque sobre el contenido en memoria, preservando la convencion de
/// saltos de linea del archivo original.
/// Devuelve el contenido nuevo y, si aplica, un aviso de ambiguedad.
fn replace_exact(current: &str, block: &PatchBlock, rel: &str) -> Result<(String, Option<String>), String> {
    let uses_crlf = current.contains("\r\n");

    let normalized_file = normalize_newlines(current);
    let normalized_search = normalize_newlines(&block.search_content);
    let normalized_replace = normalize_newlines(&block.replace_content);

    let mut warning = None;

    let updated = if normalized_search.trim().is_empty() {
        // Convencion documentada: SEARCH vacio significa "crear archivo nuevo o
        // anexar al final sin destruir el contenido existente".
        let mut base = normalized_file;
        if !base.is_empty() && !base.ends_with('\n') {
            base.push('\n');
        }
        base.push_str(&normalized_replace);
        base
    } else {
        if !normalized_file.contains(normalized_search.as_str()) {
            return Err(format!(
                "El bloque SEARCH no coincide exactamente en '{}'. \
                 Revisa la indentacion y el texto literal (los saltos de linea ya fueron normalizados).",
                rel
            ));
        }

        let occurrences = normalized_file.matches(normalized_search.as_str()).count();
        if occurrences > 1 {
            warning = Some(format!(
                "El bloque SEARCH aparecia {} veces en '{}'; se reemplazo la primera ocurrencia.",
                occurrences, rel
            ));
        }

        normalized_file.replacen(normalized_search.as_str(), normalized_replace.as_str(), 1)
    };

    let restored = if uses_crlf {
        updated.replace('\n', "\r\n")
    } else {
        updated
    };

    Ok((restored, warning))
}

/// Diff unificado (formato git) entre dos versiones de un archivo.
/// Devuelve tambien el conteo de lineas agregadas y eliminadas.
pub fn unified_diff(rel: &str, old: &str, new: &str) -> (String, usize, usize) {
    let old_norm = normalize_newlines(old);
    let new_norm = normalize_newlines(new);

    let diff = TextDiff::from_lines(old_norm.as_str(), new_norm.as_str());

    let mut additions = 0usize;
    let mut deletions = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => additions += 1,
            ChangeTag::Delete => deletions += 1,
            ChangeTag::Equal => {}
        }
    }

    let old_label = format!("a/{}", rel);
    let new_label = format!("b/{}", rel);

    let unified = diff
        .unified_diff()
        .context_radius(3)
        .header(old_label.as_str(), new_label.as_str())
        .to_string();

    (unified, additions, deletions)
}

struct ComputedFile {
    rel: String,
    full: PathBuf,
    original: Option<String>,
    updated: String,
    diff: String,
    additions: usize,
    deletions: usize,
}

/// Valida todos los bloques y calcula el resultado en memoria (sin escribir).
fn compute(repo_root: &Path, blocks: &[PatchBlock]) -> Result<(Vec<ComputedFile>, Vec<String>), String> {
    if blocks.is_empty() {
        return Err("El modelo no propuso ningun bloque de edicion.".to_string());
    }

    let mut order: Vec<String> = Vec::new();
    let mut fulls: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut originals: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut working: BTreeMap<String, String> = BTreeMap::new();
    let mut warnings: Vec<String> = Vec::new();

    for block in blocks {
        let (full, rel) = sanitize_rel_path(repo_root, &block.file_path)?;

        if !working.contains_key(&rel) {
            let original = if full.exists() {
                Some(read_text_file(&full)?)
            } else {
                None
            };
            working.insert(rel.clone(), original.clone().unwrap_or_default());
            originals.insert(rel.clone(), original);
            fulls.insert(rel.clone(), full);
            order.push(rel.clone());
        }

        let current = working.get(&rel).cloned().unwrap_or_default();

        // Heuristica anti-duplicado: si el REPLACE ya existe literalmente en el
        // archivo, el modelo probablemente esta repitiendo contenido en lugar
        // de corregirlo (habitual en modelos pequenos durante el ciclo
        // correctivo). Se avisa al usuario en el visor HITL.
        let normalized_current = normalize_newlines(&current);
        let normalized_replace = normalize_newlines(&block.replace_content);
        let normalized_search = normalize_newlines(&block.search_content);

        let search_trim = normalized_search.trim();
        let replace_trim = normalized_replace.trim();

        // Dos formas de duplicado:
        //  a) el REPLACE es exactamente texto que ya estaba en el archivo;
        //  b) el REPLACE conserva el SEARCH y anade una cola que ya existia
        //     (el modelo "reinserta" una seccion en lugar de corregirla).
        let duplicated_tail = !search_trim.is_empty()
            && replace_trim.len() > search_trim.len()
            && replace_trim.starts_with(search_trim)
            && {
                let tail = replace_trim[search_trim.len()..].trim();
                !tail.is_empty() && normalized_current.contains(tail)
            };

        let duplicated_whole = replace_trim.len() > 20
            && replace_trim != search_trim
            && normalized_current.contains(replace_trim);

        if duplicated_tail || duplicated_whole {
            let message = format!(
                "Posible duplicado en '{}': el contenido de REPLACE ya existe en el archivo.",
                rel
            );
            if !warnings.contains(&message) {
                warnings.push(message);
            }
        }

        let (next, warning) = replace_exact(&current, block, &rel)?;
        if let Some(message) = warning {
            if !warnings.contains(&message) {
                warnings.push(message);
            }
        }
        working.insert(rel.clone(), next);
    }

    let mut computed: Vec<ComputedFile> = Vec::new();
    for rel in order {
        let full = fulls
            .get(&rel)
            .cloned()
            .ok_or_else(|| format!("Ruta no resuelta: {}", rel))?;
        let original = originals.get(&rel).cloned().unwrap_or(None);
        let updated = working.get(&rel).cloned().unwrap_or_default();

        if let Some(prev) = &original {
            if prev == &updated {
                // Sin cambios reales: se omite del resultado.
                continue;
            }
        }

        let (diff, additions, deletions) =
            unified_diff(&rel, original.as_deref().unwrap_or(""), &updated);

        computed.push(ComputedFile {
            rel,
            full,
            original,
            updated,
            diff,
            additions,
            deletions,
        });
    }

    if computed.is_empty() {
        return Err("Los bloques propuestos no producen cambios reales en el repositorio.".to_string());
    }

    Ok((computed, warnings))
}

/// Previsualiza el resultado de los bloques sin tocar el disco.
pub fn preview(repo_root: &Path, blocks: &[PatchBlock]) -> Result<Vec<FileChange>, String> {
    let (computed, _warnings) = compute(repo_root, blocks)?;
    Ok(computed
        .into_iter()
        .map(|file| FileChange {
            file_path: file.rel,
            created: file.original.is_none(),
            additions: file.additions,
            deletions: file.deletions,
            diff: file.diff,
            bytes_written: 0,
        })
        .collect())
}

/// Aplica los bloques de forma atomica: si algo falla al escribir, restaura los
/// archivos ya escritos antes de devolver el error.
pub fn apply(
    repo_root: &Path,
    blocks: &[PatchBlock],
) -> Result<(Vec<FileChange>, Vec<FileSnapshot>, Vec<String>), String> {
    let (computed, warnings) = compute(repo_root, blocks)?;

    let snapshots: Vec<FileSnapshot> = computed
        .iter()
        .map(|file| FileSnapshot {
            file_path: file.rel.clone(),
            existed: file.original.is_some(),
            content: file.original.clone(),
        })
        .collect();

    let mut written: Vec<usize> = Vec::new();

    for (index, file) in computed.iter().enumerate() {
        let write_result = (|| -> Result<(), String> {
            if let Some(parent) = file.full.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("No se pudo crear el directorio {:?}: {}", parent, e))?;
            }
            fs::write(&file.full, &file.updated)
                .map_err(|e| format!("No se pudo escribir {:?}: {}", file.full, e))
        })();

        match write_result {
            Ok(()) => written.push(index),
            Err(err) => {
                // Rollback de lo ya escrito.
                for done in written.iter().rev() {
                    let previous = &computed[*done];
                    match &previous.original {
                        Some(content) => {
                            let _ = fs::write(&previous.full, content);
                        }
                        None => {
                            let _ = fs::remove_file(&previous.full);
                        }
                    }
                }
                return Err(format!("{} (se revirtieron los archivos ya escritos)", err));
            }
        }
    }

    let changes = computed
        .into_iter()
        .map(|file| FileChange {
            file_path: file.rel,
            created: file.original.is_none(),
            additions: file.additions,
            deletions: file.deletions,
            diff: file.diff,
            bytes_written: file.updated.as_bytes().len(),
        })
        .collect();

    Ok((changes, snapshots, warnings))
}

/// Restaura el contenido previo de los archivos indicados y elimina los que
/// fueron creados por el agente.
pub fn restore(repo_root: &Path, snapshots: &[FileSnapshot]) -> Result<Vec<String>, String> {
    let mut restored: Vec<String> = Vec::new();

    for snapshot in snapshots {
        let (full, rel) = sanitize_rel_path(repo_root, &snapshot.file_path)?;
        if snapshot.existed {
            if let Some(content) = &snapshot.content {
                if let Some(parent) = full.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::write(&full, content)
                    .map_err(|e| format!("No se pudo restaurar '{}': {}", rel, e))?;
                restored.push(rel);
            }
        } else if full.exists() {
            fs::remove_file(&full)
                .map_err(|e| format!("No se pudo eliminar '{}': {}", rel, e))?;
            restored.push(rel);
        }
    }

    Ok(restored)
}

// ---------------------------------------------------------------------------
// Comandos IPC
// ---------------------------------------------------------------------------

/// Dry-run: devuelve el diff que producirian los bloques, sin escribir nada.
#[tauri::command]
pub fn patch_preview(
    state: tauri::State<'_, AppState>,
    blocks: Vec<PatchBlock>,
) -> Result<Vec<FileChange>, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;
    preview(&repo, &blocks)
}

/// Aplica los bloques en disco (escritura atomica con rollback).
#[tauri::command]
pub fn patch_apply(
    state: tauri::State<'_, AppState>,
    blocks: Vec<PatchBlock>,
) -> Result<Vec<FileChange>, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;
    let (changes, _snapshots, _warnings) = apply(&repo, &blocks)?;
    Ok(changes)
}
