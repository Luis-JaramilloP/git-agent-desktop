//! Control determinista de subprocesos Git y GitHub CLI (Seccion 8 del plan).
//!
//! El modelo de IA no tiene acceso a una consola: todas las operaciones pasan
//! por estas funciones y las que publican en remoto exigen aprobacion humana
//! previa en la interfaz.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::state::AppState;
use crate::utils::process;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub path: String,
    pub name: String,
    pub is_git_repo: bool,
    pub remote_url: Option<String>,
    pub current_branch: Option<String>,
    pub default_branch: Option<String>,
    pub head_commit: Option<String>,
    pub head_subject: Option<String>,
    pub dirty: bool,
    pub changed_files: usize,
    pub git_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    pub branch: String,
    pub default_branch: String,
    pub dirty: bool,
    pub changed_files: Vec<String>,
    pub staged_files: usize,
    pub ahead: usize,
    pub behind: usize,
}

fn opt_cli(bin: &str, args: &[&str], cwd: &Path) -> Option<String> {
    process::run_cli(bin, args, Some(cwd))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Rama base del repositorio: `origin/HEAD` -> main/master local -> main/master
/// remoto -> rama actual.
pub fn detect_default_branch(repo: &Path) -> String {
    if let Some(symbolic) = opt_cli("git", &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"], repo) {
        if let Some(name) = symbolic.strip_prefix("origin/") {
            return name.to_string();
        }
    }

    for candidate in ["main", "master"] {
        let reference = format!("refs/heads/{}", candidate);
        if process::run_cli("git", &["rev-parse", "--verify", "--quiet", &reference], Some(repo)).is_ok() {
            return candidate.to_string();
        }
    }

    for candidate in ["main", "master"] {
        let reference = format!("refs/remotes/origin/{}", candidate);
        if process::run_cli("git", &["rev-parse", "--verify", "--quiet", &reference], Some(repo)).is_ok() {
            return candidate.to_string();
        }
    }

    current_branch(repo).unwrap_or_else(|_| "main".to_string())
}

pub fn current_branch(repo: &Path) -> Result<String, String> {
    let out = process::run_cli_any("git", &["rev-parse", "--abbrev-ref", "HEAD"], Some(repo))?;
    if out.ok() {
        let branch = out.stdout.trim().to_string();
        if !branch.is_empty() && branch != "HEAD" {
            return Ok(branch);
        }
    }
    Err(format!(
        "No se pudo determinar la rama actual: {}",
        out.combined()
    ))
}

pub fn branch_exists(repo: &Path, branch: &str) -> bool {
    let reference = format!("refs/heads/{}", branch);
    process::run_cli(
        "git",
        &["rev-parse", "--verify", "--quiet", &reference],
        Some(repo),
    )
    .is_ok()
}

pub fn remote_branch_exists(repo: &Path, branch: &str) -> bool {
    let reference = format!("refs/remotes/origin/{}", branch);
    process::run_cli(
        "git",
        &["rev-parse", "--verify", "--quiet", &reference],
        Some(repo),
    )
    .is_ok()
}

pub fn has_commits(repo: &Path) -> bool {
    process::run_cli("git", &["rev-parse", "--verify", "HEAD"], Some(repo)).is_ok()
}

pub fn commit_sha(repo: &Path) -> Option<String> {
    opt_cli("git", &["rev-parse", "--short", "HEAD"], repo)
}

pub fn checkout(repo: &Path, branch: &str) -> Result<String, String> {
    process::run_cli("git", &["checkout", branch], Some(repo))
}

pub fn delete_branch(repo: &Path, branch: &str) -> Result<String, String> {
    process::run_cli("git", &["branch", "-D", branch], Some(repo))
}

/// Numero de commits que `branch` tiene por delante de `base`.
pub fn commits_ahead(repo: &Path, base: &str, branch: &str) -> usize {
    let range = format!("{}..{}", base, branch);
    match process::run_cli("git", &["rev-list", "--count", &range], Some(repo)) {
        Ok(out) => out.trim().parse::<usize>().unwrap_or(0),
        Err(_) => 0,
    }
}

/// `git diff` del arbol de trabajo (incluye archivos sin seguimiento si ya se
/// agregaron al indice).
pub fn get_repository_diff(repo_dir: &Path) -> Result<String, String> {
    process::run_cli("git", &["diff"], Some(repo_dir))
}

/// Diff completo (staged + unstaged) contra HEAD; si no hay HEAD, muestra el
/// indice.
pub fn get_full_diff(repo_dir: &Path) -> Result<String, String> {
    let mut out = String::new();
    let staged = process::run_cli("git", &["diff", "--cached"], Some(repo_dir)).unwrap_or_default();
    let unstaged = process::run_cli("git", &["diff"], Some(repo_dir)).unwrap_or_default();
    if !staged.trim().is_empty() {
        out.push_str(&staged);
    }
    if !unstaged.trim().is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&unstaged);
    }
    Ok(out)
}

pub fn stage_paths(repo_dir: &Path, paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("No hay rutas para preparar en el indice.".to_string());
    }
    let mut args: Vec<&str> = vec!["add", "-A", "--"];
    for path in paths {
        args.push(path.as_str());
    }
    process::run_cli("git", &args, Some(repo_dir))?;
    Ok(())
}

pub fn staged_diff(repo_dir: &Path, paths: &[String]) -> Result<String, String> {
    if paths.is_empty() {
        return process::run_cli("git", &["diff", "--cached"], Some(repo_dir));
    }
    let mut args: Vec<&str> = vec!["diff", "--cached", "--"];
    for path in paths {
        args.push(path.as_str());
    }
    process::run_cli("git", &args, Some(repo_dir))
}

pub fn unstage_paths(repo_dir: &Path, paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut args: Vec<&str> = vec!["reset", "--quiet", "--"];
    for path in paths {
        args.push(path.as_str());
    }
    let _ = process::run_cli("git", &args, Some(repo_dir));
    Ok(())
}

/// Crea la rama aislada `agent/...` (Seccion 9, paso 2).
pub fn create_isolated_branch(repo_dir: &Path, branch_name: &str) -> Result<String, String> {
    if branch_exists(repo_dir, branch_name) {
        return Err(format!(
            "La rama '{}' ya existe en el repositorio. Elige otro nombre.",
            branch_name
        ));
    }
    process::run_cli("git", &["checkout", "-b", branch_name], Some(repo_dir))
}

/// Commit semantico + push de la rama aislada + `gh pr create`.
/// Devuelve la URL del Pull Request creado (o el existente).
pub fn commit_and_create_pr(
    repo_dir: &Path,
    branch_name: &str,
    commit_msg: &str,
    pr_title: &str,
    pr_body: &str,
    base_branch: &str,
    paths: &[String],
) -> Result<String, String> {
    if paths.is_empty() {
        process::run_cli("git", &["add", "."], Some(repo_dir))?;
    } else {
        stage_paths(repo_dir, paths)?;
    }

    // Commitea solo los archivos que toco el agente cuando se conocen.
    // Si ya existia un commit con esos cambios (reintento tras un fallo en el
    // push o en `gh pr create`) se continua en lugar de abortar.
    let commit_output = if paths.is_empty() {
        process::run_cli_raw("git", &["commit", "-m", commit_msg], Some(repo_dir), None)?
    } else {
        let mut args: Vec<&str> = vec!["commit", "-m", commit_msg, "--"];
        for path in paths {
            args.push(path.as_str());
        }
        process::run_cli_raw("git", &args, Some(repo_dir), None)?
    };

    if !commit_output.ok() {
        let detail = commit_output.combined().to_lowercase();
        let already_committed = detail.contains("nothing to commit")
            || detail.contains("no changes added")
            || detail.contains("nothing added to commit");
        if !already_committed {
            return Err(format!("'git commit' fallo: {}", commit_output.combined()));
        }
    }

    process::run_cli(
        "git",
        &["push", "-u", "origin", branch_name],
        Some(repo_dir),
    )?;

    let create = process::run_cli_any(
        "gh",
        &[
            "pr",
            "create",
            "--title",
            pr_title,
            "--body",
            pr_body,
            "--base",
            base_branch,
            "--head",
            branch_name,
        ],
        Some(repo_dir),
    )?;

    if create.ok() {
        let url = create
            .stdout
            .lines()
            .rev()
            .find(|line| line.trim().starts_with("http"))
            .map(|line| line.trim().to_string())
            .unwrap_or_default();
        if url.is_empty() {
            return Err(format!(
                "'gh pr create' no devolvio una URL. Salida: {}",
                create.combined()
            ));
        }
        return Ok(url);
    }

    // Si el PR ya existia para esa rama, se recupera su URL.
    let combined = create.combined();
    if combined.to_lowercase().contains("already exists") {
        if let Ok(existing) = process::run_cli(
            "gh",
            &["pr", "view", "--json", "url", "--jq", ".url"],
            Some(repo_dir),
        ) {
            if existing.starts_with("http") {
                return Ok(existing);
            }
        }
    }

    Err(format!("'gh pr create' fallo: {}", combined))
}

/// Comprueba en el remoto (no en las referencias locales) si la rama existe.
pub fn remote_branch_live(repo: &Path, branch: &str) -> bool {
    process::run_cli("git", &["ls-remote", "--heads", "origin", branch], Some(repo))
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false)
}

/// Elimina la rama del agente en `origin` despues de integrar el PR.
///
/// Si la rama ya no existe (por ejemplo, porque GitHub tiene activado el
/// borrado automatico de ramas) se considera exito.
pub fn delete_remote_branch(repo: &Path, branch: &str) -> Result<(), String> {
    if branch.trim().is_empty() {
        return Ok(());
    }

    if !remote_branch_live(repo, branch) {
        return Ok(());
    }

    let output = process::run_cli_raw(
        "git",
        &["push", "origin", "--delete", branch],
        Some(repo),
        None,
    )?;

    if output.ok() {
        return Ok(());
    }

    let detail = output.combined().to_lowercase();
    if detail.contains("remote ref does not exist") || detail.contains("no such ref") {
        return Ok(());
    }

    Err(format!(
        "No se pudo eliminar la rama remota '{}': {}",
        branch,
        output.combined()
    ))
}

/// Vuelve a la rama base y actualiza (`git pull`), luego borra la rama del
/// agente. Usado cuando el PR ya fue integrado (Seccion 10).
pub fn cleanup_after_merge(repo_dir: &Path, branch_name: &str, base_branch: &str) -> Result<String, String> {
    let mut log = Vec::new();

    if current_branch(repo_dir).ok().as_deref() != Some(base_branch) {
        checkout(repo_dir, base_branch)?;
        log.push(format!("Checkout a '{}'.", base_branch));
    }

    match process::run_cli("git", &["pull", "--ff-only"], Some(repo_dir)) {
        Ok(out) => {
            let summary = out.lines().next().unwrap_or("git pull sin cambios").to_string();
            log.push(format!("git pull: {}", summary));
        }
        Err(err) => log.push(format!("git pull omitido: {}", err)),
    }

    if branch_name != base_branch && branch_exists(repo_dir, branch_name) {
        match delete_branch(repo_dir, branch_name) {
            Ok(_) => log.push(format!("Rama local '{}' eliminada.", branch_name)),
            Err(err) => log.push(format!("No se pudo eliminar la rama: {}", err)),
        }
    }

    Ok(log.join(" "))
}

// ---------------------------------------------------------------------------
// Inspeccion
// ---------------------------------------------------------------------------

fn resolve_worktree_root(path: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("La ruta no existe: {:?}", path));
    }
    if !path.is_dir() {
        return Err(format!("La ruta no es una carpeta: {:?}", path));
    }
    if let Ok(toplevel) = process::run_cli("git", &["rev-parse", "--show-toplevel"], Some(path)) {
        let trimmed = toplevel.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    Ok(path.to_path_buf())
}

pub fn inspect(path: &Path) -> Result<RepoInfo, String> {
    let root = resolve_worktree_root(path)?;

    let is_git_repo = process::run_cli_any(
        "git",
        &["rev-parse", "--is-inside-work-tree"],
        Some(&root),
    )
    .map(|out| out.ok() && out.stdout.trim() == "true")
    .unwrap_or(false);

    if !is_git_repo {
        return Ok(RepoInfo {
            path: root.to_string_lossy().to_string(),
            name: root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| root.to_string_lossy().to_string()),
            is_git_repo: false,
            remote_url: None,
            current_branch: None,
            default_branch: None,
            head_commit: None,
            head_subject: None,
            dirty: false,
            changed_files: 0,
            git_version: process::version_of("git"),
        });
    }

    let status_output = process::run_cli("git", &["status", "--porcelain"], Some(&root)).unwrap_or_default();
    let changed_files = status_output.lines().filter(|line| !line.trim().is_empty()).count();

    Ok(RepoInfo {
        path: root.to_string_lossy().to_string(),
        name: root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| root.to_string_lossy().to_string()),
        is_git_repo: true,
        remote_url: opt_cli("git", &["remote", "get-url", "origin"], &root),
        current_branch: current_branch(&root).ok(),
        default_branch: Some(detect_default_branch(&root)),
        head_commit: opt_cli("git", &["rev-parse", "--short", "HEAD"], &root),
        head_subject: opt_cli("git", &["log", "-1", "--pretty=%s"], &root),
        dirty: changed_files > 0,
        changed_files,
        git_version: process::version_of("git"),
    })
}

pub fn status(repo: &Path) -> Result<RepoStatus, String> {
    let out = process::run_cli_any("git", &["status", "--porcelain=v1"], Some(repo))?;
    let mut changed: Vec<String> = Vec::new();
    let mut staged = 0usize;

    for line in out.stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let code = line.get(0..2).unwrap_or("  ");
        if code.trim() != "??" {
            staged += 1;
        }
        if let Some(path) = line.get(3..) {
            changed.push(path.trim().to_string());
        }
    }

    let (ahead, behind) = ahead_behind(repo).unwrap_or((0, 0));

    Ok(RepoStatus {
        branch: current_branch(repo).unwrap_or_else(|_| "(sin commits)".to_string()),
        default_branch: detect_default_branch(repo),
        dirty: !changed.is_empty(),
        changed_files: changed,
        staged_files: staged,
        ahead,
        behind,
    })
}

fn ahead_behind(repo: &Path) -> Option<(usize, usize)> {
    let base = detect_default_branch(repo);
    let reference = format!("origin/{}...HEAD", base);
    let out = process::run_cli(
        "git",
        &["rev-list", "--left-right", "--count", &reference],
        Some(repo),
    )
    .ok()?;
    let mut parts = out.split_whitespace();
    let behind = parts.next()?.parse::<usize>().ok()?;
    let ahead = parts.next()?.parse::<usize>().ok()?;
    Some((ahead, behind))
}

// ---------------------------------------------------------------------------
// Comandos IPC
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn repo_inspect(path: String) -> Result<RepoInfo, String> {
    inspect(Path::new(&path))
}

/// Abre un repositorio y lo deja como repositorio activo de la sesion.
#[tauri::command]
pub fn repo_open(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<RepoInfo, String> {
    let info = inspect(Path::new(&path))?;
    if !info.is_git_repo {
        return Err(format!(
            "'{}' no es un repositorio Git. Inicializa uno con 'git init' o elige otra carpeta.",
            info.path
        ));
    }
    state.set_repo(Some(PathBuf::from(&info.path)));
    state.update_session(|session| {
        session.repo_path = Some(info.path.clone());
        session.base_branch = info.default_branch.clone();
    });
    Ok(info)
}

#[tauri::command]
pub fn repo_status(state: tauri::State<'_, AppState>) -> Result<RepoStatus, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;
    status(&repo)
}

#[tauri::command]
pub fn git_create_branch(
    state: tauri::State<'_, AppState>,
    branch: String,
) -> Result<String, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;
    create_isolated_branch(&repo, &branch)
}

#[tauri::command]
pub fn git_diff(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;
    get_full_diff(&repo)
}

/// Restaura archivos desde HEAD (recuperacion manual).
#[tauri::command]
pub fn git_restore(
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
) -> Result<String, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;

    if paths.is_empty() {
        process::run_cli("git", &["restore", "."], Some(&repo))?;
        return Ok("Se restauraron todos los archivos del arbol de trabajo.".to_string());
    }

    let mut args: Vec<&str> = vec!["restore", "--"];
    for path in &paths {
        args.push(path.as_str());
    }
    process::run_cli("git", &args, Some(&repo))?;
    Ok(format!("Se restauraron {} archivo(s) desde HEAD.", paths.len()))
}

#[tauri::command]
pub fn git_cleanup_after_merge(
    state: tauri::State<'_, AppState>,
    branch: String,
    base_branch: String,
) -> Result<String, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;
    cleanup_after_merge(&repo, &branch, &base_branch)
}
