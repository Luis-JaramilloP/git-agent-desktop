//! Construccion del arbol de archivos filtrado (Seccion 12.3, etapa 1 del
//! Two-Stage Context Retrieval).
//!
//! Reglas:
//! * Se respeta `.gitignore` usando `git ls-files`.
//! * Se descartan carpetas pesadas, binarios y lockfiles.
//! * Nunca se envian al modelo archivos con apariencia de secretos
//!   (`.env`, claves privadas, credenciales).

use std::path::Path;

use serde::Serialize;

use crate::commands::patch::{normalize_newlines, sanitize_rel_path};
use crate::utils::process;

const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".idea",
    ".vscode",
    ".gradle",
    "vendor",
    "Pods",
    "coverage",
    ".turbo",
    ".cache",
    ".terraform",
    "bin",
    "obj",
    ".dart_tool",
    "storybook-static",
    ".git-agent",
];

const IGNORED_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "icns", "pdf", "zip", "gz", "tgz", "rar",
    "7z", "xz", "bz2", "exe", "dll", "so", "dylib", "a", "lib", "obj", "pdb", "class", "jar",
    "war", "pyc", "pyo", "pyd", "wasm", "mp3", "mp4", "mov", "avi", "wav", "flac", "woff",
    "woff2", "ttf", "eot", "otf", "sqlite", "sqlite3", "db", "mdb", "pak", "snap", "onnx",
    "pt", "pth", "safetensors", "nupkg", "msi", "dmg", "iso", "img",
];

const IGNORED_FILES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "Cargo.lock",
    "poetry.lock",
    "composer.lock",
    "Gemfile.lock",
    "pubspec.lock",
    "go.sum",
    ".terraform.lock.hcl",
    "mix.lock",
    "flake.lock",
    "bun.lockb",
];

const SECRET_FILES: &[&str] = &[
    ".env",
    ".env.local",
    ".env.production",
    ".env.development",
    ".env.test",
    ".npmrc",
    ".pypirc",
    ".netrc",
    "_netrc",
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    "secrets.json",
    "credentials.json",
    "service-account.json",
    "keystore.jks",
    ".git-credentials",
];

const SECRET_EXTENSIONS: &[&str] = &["pem", "key", "pfx", "p12", "jks", "keystore", "asc", "gpg"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Include,
    Skip,
    Secret,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoTree {
    pub root: String,
    pub files: Vec<String>,
    pub total_files: usize,
    pub shown: usize,
    pub truncated: bool,
    pub is_git_repo: bool,
    pub skipped_secrets: Vec<String>,
}

fn verdict_for(rel: &str) -> Verdict {
    if rel.trim().is_empty() {
        return Verdict::Skip;
    }

    let normalized = rel.replace('\\', "/");
    let lower = normalized.to_lowercase();

    let name = lower.rsplit('/').next().unwrap_or(&lower);
    let dirs: Vec<&str> = lower.split('/').collect();

    if dirs
        .iter()
        .take(dirs.len().saturating_sub(1))
        .any(|segment| IGNORED_DIRS.contains(segment))
    {
        return Verdict::Skip;
    }

    let extension = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");

    if SECRET_FILES.contains(&name) || SECRET_EXTENSIONS.contains(&extension) {
        return Verdict::Secret;
    }
    if IGNORED_FILES.contains(&name) {
        return Verdict::Skip;
    }
    if IGNORED_EXTENSIONS.contains(&extension) {
        return Verdict::Skip;
    }
    if lower.contains(".min.") {
        return Verdict::Skip;
    }

    Verdict::Include
}

fn list_via_git(repo: &Path) -> Option<Vec<String>> {
    let out = process::run_cli_any(
        "git",
        &["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        Some(repo),
    )
    .ok()?;

    if !out.ok() {
        return None;
    }

    let files: Vec<String> = out
        .stdout
        .split('\0')
        .map(|entry| entry.trim().replace('\\', "/"))
        .filter(|entry| !entry.is_empty())
        .collect();

    Some(files)
}

fn list_via_walk(repo: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, depth: usize, out: &mut Vec<String>) {
        if depth > 14 || out.len() > 20_000 {
            return;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if IGNORED_DIRS.contains(&name.to_lowercase().as_str()) {
                    continue;
                }
                walk(&path, base, depth + 1, out);
            } else if let Ok(relative) = path.strip_prefix(base) {
                let rel = relative.to_string_lossy().replace('\\', "/");
                // Los archivos con apariencia de secretos se devuelven para que
                // `build` los registre como omitidos (nunca se envian al modelo).
                match verdict_for(&rel) {
                    Verdict::Include | Verdict::Secret => out.push(rel),
                    Verdict::Skip => {}
                }
            }
        }
    }

    let mut files = Vec::new();
    walk(repo, repo, 0, &mut files);
    files
}

/// Construye el arbol filtrado del repositorio.
pub fn build(repo: &Path, max_files: usize) -> RepoTree {
    let git_files = list_via_git(repo);
    let is_git_repo = git_files.is_some();
    let raw = git_files.unwrap_or_else(|| list_via_walk(repo));

    let mut included: Vec<String> = Vec::new();
    let mut secrets: Vec<String> = Vec::new();

    for rel in raw {
        match verdict_for(&rel) {
            Verdict::Include => included.push(rel),
            Verdict::Secret => secrets.push(rel),
            Verdict::Skip => {}
        }
    }

    included.sort();
    included.dedup();

    let total = included.len();
    let truncated = total > max_files;
    included.truncate(max_files);

    RepoTree {
        root: repo.to_string_lossy().to_string(),
        shown: included.len(),
        files: included,
        total_files: total,
        truncated,
        is_git_repo,
        skipped_secrets: secrets,
    }
}

impl RepoTree {
    /// Representacion compacta (menos de ~300 tokens para repos pequenos) que se
    /// envia al modelo en la etapa 1.
    pub fn prompt_view(&self) -> String {
        let mut out = String::new();
        for file in &self.files {
            out.push_str("- ");
            out.push_str(file);
            out.push('\n');
        }
        if self.truncated {
            out.push_str(&format!(
                "- ... ({} archivos adicionales omitidos)\n",
                self.total_files.saturating_sub(self.shown)
            ));
        }
        out
    }

    pub fn contains(&self, rel: &str) -> bool {
        let normalized = rel.replace('\\', "/");
        self.files.iter().any(|file| file == &normalized)
    }
}

/// Lee un archivo del repositorio como texto UTF-8 (normalizado a LF) con
/// limite de tamano y deteccion de binarios.
pub fn read_file(repo: &Path, rel: &str, max_bytes: usize) -> Result<String, String> {
    let (full, rel_norm) = sanitize_rel_path(repo, rel)?;

    if !full.exists() {
        return Err(format!("El archivo '{}' no existe en el repositorio.", rel_norm));
    }

    let metadata = std::fs::metadata(&full)
        .map_err(|e| format!("No se pudo inspeccionar '{}': {}", rel_norm, e))?;

    if metadata.len() as usize > max_bytes {
        return Err(format!(
            "El archivo '{}' pesa {} bytes y supera el limite de {} bytes para el contexto del modelo.",
            rel_norm,
            metadata.len(),
            max_bytes
        ));
    }

    let bytes = std::fs::read(&full)
        .map_err(|e| format!("No se pudo leer '{}': {}", rel_norm, e))?;

    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Err(format!("El archivo '{}' parece binario.", rel_norm));
    }

    Ok(normalize_newlines(&String::from_utf8_lossy(&bytes)))
}
