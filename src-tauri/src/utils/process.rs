//! Ejecutor de subprocesos con resolucion de binarios en Windows.
//!
//! Resuelve el problema descrito en la Seccion 12.2 del plan: tras una
//! instalacion silenciosa con `winget`, el PATH del proceso en ejecucion puede
//! no incluir todavia `git`/`gh`, por lo que primero se prueba el PATH activo y
//! despues las rutas absolutas por defecto de cada instalador.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Resultado completo de una ejecucion de CLI.
#[derive(Debug, Clone)]
pub struct CliOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CliOutput {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    /// Salida combinada, util para mensajes de error y logs.
    pub fn combined(&self) -> String {
        let mut out = self.stdout.trim().to_string();
        let err = self.stderr.trim();
        if !err.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(err);
        }
        out
    }
}

/// Evita que Windows abra una consola parpadeante por cada subproceso.
pub fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

fn cache() -> &'static Mutex<HashMap<String, PathBuf>> {
    static CACHE: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn env_override(bin_name: &str) -> Option<PathBuf> {
    let key = match bin_name {
        "git" => "GAD_GIT_BIN",
        "gh" => "GAD_GH_BIN",
        _ => return None,
    };
    let value = std::env::var(key).ok()?;
    let path = PathBuf::from(value.trim());
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn probe(bin: &Path) -> bool {
    let mut cmd = Command::new(bin);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut cmd);
    matches!(cmd.status(), Ok(status) if status.success())
}

fn default_candidates(bin_name: &str) -> Vec<PathBuf> {
    let program_files = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let program_files_x86 =
        std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".to_string());
    let local_app = std::env::var("LOCALAPPDATA").unwrap_or_default();

    match bin_name {
        "git" => vec![
            PathBuf::from(r"C:\Program Files\Git\cmd\git.exe"),
            PathBuf::from(format!(r"{}\Git\cmd\git.exe", program_files)),
            PathBuf::from(format!(r"{}\Git\cmd\git.exe", program_files_x86)),
            PathBuf::from(format!(r"{}\Programs\Git\cmd\git.exe", local_app)),
            PathBuf::from(format!(r"{}\Microsoft\WinGet\Links\git.exe", local_app)),
            PathBuf::from(r"C:\Program Files\Git\bin\git.exe"),
        ],
        "gh" => vec![
            PathBuf::from(r"C:\Program Files\GitHub CLI\gh.exe"),
            PathBuf::from(format!(r"{}\GitHub CLI\gh.exe", program_files)),
            PathBuf::from(format!(r"{}\GitHub CLI\bin\gh.exe", program_files)),
            PathBuf::from(format!(r"{}\Programs\GitHub CLI\gh.exe", local_app)),
            PathBuf::from(format!(r"{}\GitHub CLI\gh.exe", local_app)),
            PathBuf::from(format!(r"{}\Microsoft\WinGet\Links\gh.exe", local_app)),
        ],
        _ => vec![],
    }
}

/// Devuelve la ruta ejecutable de `bin_name` (PATH -> overrides -> rutas
/// absolutas de respaldo -> nombre plano como ultimo recurso).
pub fn resolve_binary(bin_name: &str) -> PathBuf {
    let path = Path::new(bin_name);
    if path.is_absolute() {
        return path.to_path_buf();
    }

    if let Some(found) = env_override(bin_name) {
        return found;
    }

    if let Ok(guard) = cache().lock() {
        if let Some(hit) = guard.get(bin_name) {
            if hit.exists() {
                return hit.clone();
            }
        }
    }

    let resolved = if probe(path) {
        PathBuf::from(bin_name)
    } else {
        default_candidates(bin_name)
            .into_iter()
            .find(|candidate| candidate.exists() && probe(candidate))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(bin_name))
    };

    if resolved.is_absolute() && resolved.exists() {
        if let Ok(mut guard) = cache().lock() {
            guard.insert(bin_name.to_string(), resolved.clone());
        }
    }

    resolved
}

fn spawn_error(bin_name: &str, exe: &Path, err: std::io::Error) -> String {
    if err.kind() == std::io::ErrorKind::NotFound {
        format!(
            "No se encontro el ejecutable '{}' (buscado como {:?}). \
             Instalalo o define GAD_{}_BIN con la ruta absoluta.",
            bin_name,
            exe,
            bin_name.to_uppercase()
        )
    } else {
        format!("Error al iniciar '{}' ({:?}): {}", bin_name, exe, err)
    }
}

/// Asegura que el subproceso encuentre `git` y `gh` aunque el PATH del proceso
/// actual este desactualizado (por ejemplo, tras instalar con `winget`, o al
/// lanzar la app desde una consola abierta antes de la instalacion).
/// `gh` necesita `git` en el PATH para `gh pr create` / `gh pr view`.
pub fn augmented_path_entries() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    for bin in ["git", "gh"] {
        let resolved = resolve_binary(bin);
        if resolved.is_absolute() {
            if let Some(parent) = resolved.parent() {
                if parent.exists() {
                    dirs.push(parent.to_path_buf());
                }
            }
        }
    }

    for candidate in [
        r"C:\Program Files\Git\cmd",
        r"C:\Program Files\Git\bin",
        r"C:\Program Files\GitHub CLI",
        r"C:\Program Files\GitHub CLI\bin",
    ] {
        let path = PathBuf::from(candidate);
        if path.exists() {
            dirs.push(path);
        }
    }

    dirs.dedup();
    dirs
}

fn augment_child_path(cmd: &mut Command) {
    let dirs = augmented_path_entries();

    if dirs.is_empty() {
        return;
    }

    let mut paths = dirs.clone();
    if let Some(current) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current));
    }

    match std::env::join_paths(paths) {
        Ok(joined) => {
            cmd.env("PATH", joined);
        }
        Err(_) => {
            let fallback = dirs
                .iter()
                .map(|dir| dir.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(";");
            cmd.env("PATH", fallback);
        }
    }
}

/// Ejecuta un comando y devuelve codigo, stdout y stderr sin interpretar.
pub fn run_cli_raw(
    bin_name: &str,
    args: &[&str],
    cwd: Option<&Path>,
    stdin_data: Option<&str>,
) -> Result<CliOutput, String> {
    let exe = resolve_binary(bin_name);

    let mut cmd = Command::new(&exe);
    cmd.args(args);
    if let Some(dir) = cwd {
        if !dir.exists() {
            return Err(format!("El directorio de trabajo no existe: {:?}", dir));
        }
        cmd.current_dir(dir);
    }
    cmd.stdin(if stdin_data.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    hide_console(&mut cmd);
    augment_child_path(&mut cmd);

    let mut child = cmd.spawn().map_err(|e| spawn_error(bin_name, &exe, e))?;

    if let Some(data) = stdin_data {
        if let Some(mut handle) = child.stdin.take() {
            handle
                .write_all(data.as_bytes())
                .map_err(|e| format!("Error escribiendo en stdin de '{}': {}", bin_name, e))?;
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Error esperando a '{}': {}", bin_name, e))?;

    Ok(CliOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// Ejecuta un comando y devuelve `stdout` recortado; en caso de codigo distinto
/// de cero devuelve `Err` con la salida de error.
pub fn run_cli(bin_name: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, String> {
    let out = run_cli_raw(bin_name, args, cwd, None)?;
    if out.ok() {
        Ok(out.stdout.trim().to_string())
    } else {
        let detail = out.combined();
        Err(format!(
            "'{} {}' fallo (codigo {}): {}",
            bin_name,
            args.join(" "),
            out.code,
            if detail.is_empty() { "sin salida".to_string() } else { detail }
        ))
    }
}

/// Igual que `run_cli` pero no falla si el codigo de salida no es cero
/// (util para `gh auth status`).
pub fn run_cli_any(bin_name: &str, args: &[&str], cwd: Option<&Path>) -> Result<CliOutput, String> {
    run_cli_raw(bin_name, args, cwd, None)
}

/// Version corta de un binario (`git version 2.x`), o `None` si no existe.
pub fn version_of(bin_name: &str) -> Option<String> {
    let out = run_cli_raw(bin_name, &["--version"], None, None).ok()?;
    if !out.ok() {
        return None;
    }
    let first = out.stdout.lines().next().unwrap_or("").trim().to_string();
    if first.is_empty() {
        None
    } else {
        Some(first)
    }
}

/// Comprueba si un binario esta disponible en el sistema.
pub fn is_installed(bin_name: &str) -> bool {
    version_of(bin_name).is_some()
}
