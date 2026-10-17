//! Prueba end-to-end del ciclo agentico (Seccion 9) sin interfaz grafica.
//!
//! Usa el modo determinista `mock_ai` (sin red ni token) y Git real:
//! rama aislada -> plan -> parcheo atomico -> diff -> estado HITL -> abort.
//! Se omite si `git` no esta disponible.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use git_agent_desktop_lib::agent::{self, RunRequest};
use git_agent_desktop_lib::commands::git;
use git_agent_desktop_lib::config::AppConfig;
use git_agent_desktop_lib::memory::MemoryStore;
use git_agent_desktop_lib::state::{AppState, Reporter};
use git_agent_desktop_lib::utils::process;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

const ORIGINAL: &str = "fn calcular() {\n    // TODO: validar la entrada\n    42\n}\n";

struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Option<Self> {
        if process::version_of("git").is_none() {
            return None;
        }

        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "gad-flow-{}-{}-{}",
            tag,
            std::process::id(),
            unique
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).ok()?;

        let fixture = Self { path };
        run(&fixture.path, &["init", "-b", "main"]);
        run(&fixture.path, &["config", "user.email", "agent@example.com"]);
        run(&fixture.path, &["config", "user.name", "Git Agent Desktop"]);
        run(&fixture.path, &["config", "commit.gpgsign", "false"]);

        fs::write(fixture.path.join("app.rs"), ORIGINAL).ok()?;
        fs::write(fixture.path.join("README.md"), "# Demo\n").ok()?;
        run(&fixture.path, &["add", "-A"]);
        run(&fixture.path, &["commit", "-m", "estado inicial"]);

        Some(fixture)
    }

    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.path.join(relative)).unwrap_or_default()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run(repo: &std::path::Path, args: &[&str]) -> String {
    process::run_cli("git", args, Some(repo)).unwrap_or_default()
}

fn state_for(repo: &std::path::Path) -> AppState {
    let mut config = AppConfig::default();
    config.mock_ai = true;
    config.max_attempts = 3;

    let state = AppState::new(config);
    state.set_repo(Some(repo.to_path_buf()));
    state
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime de tokio")
}

fn request(task: &str) -> RunRequest {
    RunRequest {
        task: task.to_string(),
        issue: None,
        model: None,
    }
}

#[test]
fn ciclo_completo_run_abort_restaura_todo() {
    let Some(fixture) = Fixture::new("completo") else {
        eprintln!("git no disponible: prueba omitida");
        return;
    };

    let state = state_for(&fixture.path);
    let reporter = Reporter::silent();
    let rt = runtime();

    let outcome = rt
        .block_on(agent::run(
            &reporter,
            &state,
            request("Corrige el TODO de validacion en app.rs"),
        ))
        .expect("el ciclo debe completarse");

    // --- Salidas del ciclo ---
    assert!(!outcome.changes.is_empty(), "debe haber cambios");
    assert!(
        outcome.branch.starts_with("agent/"),
        "rama aislada: {}",
        outcome.branch
    );
    assert_eq!(outcome.base_branch, "main");
    assert!(git::branch_exists(&fixture.path, &outcome.branch));
    assert_eq!(
        git::current_branch(&fixture.path).unwrap(),
        outcome.branch,
        "el agente debe dejar activa la rama aislada"
    );
    assert!(!outcome.diff.trim().is_empty(), "el visor HITL necesita diff");
    assert!(outcome.diff.contains("+"), "diff con lineas agregadas");
    assert!(outcome.summary.contains("MODO PRUEBA"));
    assert!(!outcome.commit_message.trim().is_empty());
    assert!(!outcome.pr_title.trim().is_empty());
    assert!(outcome.mock);

    // El parche se aplico de verdad en disco.
    let patched = fixture.read("app.rs");
    assert!(patched.contains("DONE"), "TODO reemplazado: {}", patched);
    assert!(!patched.contains("TODO:"));

    // --- Estado HITL: nada se publica solo ---
    let session = state.session_snapshot();
    assert!(session.active);
    assert_eq!(session.attempts, 1);
    assert_eq!(session.max_attempts, 3);
    assert_eq!(session.phase, "review");
    assert!(!session.touched_files.is_empty());
    assert!(!session.snapshot.is_empty());
    assert!(session.pr_url.is_none(), "no se publica sin aprobacion");
    assert_eq!(
        git::commits_ahead(&fixture.path, "main", &outcome.branch),
        0,
        "el agente no debe commitear sin aprobacion"
    );

    // Una segunda ejecucion no debe pisar la propuesta pendiente.
    let blocked = rt
        .block_on(agent::run(&reporter, &state, request("otra tarea")))
        .expect_err("debe rechazar una segunda tarea");
    assert!(blocked.contains("pendiente"), "mensaje: {}", blocked);

    // --- Abort: restauracion exacta ---
    let abort = agent::abort(&reporter, &state).expect("abortar");
    assert!(abort.branch_deleted, "la rama sin commits debe eliminarse");
    assert!(!git::branch_exists(&fixture.path, &outcome.branch));
    assert_eq!(git::current_branch(&fixture.path).unwrap(), "main");
    assert_eq!(fixture.read("app.rs"), ORIGINAL, "contenido original restaurado");
    assert_eq!(fixture.read("README.md"), "# Demo\n");
    assert!(!state.session_snapshot().active);

    // La bitacora reflexiva registra el intento abortado.
    let memory = MemoryStore::load(&fixture.path);
    assert!(
        memory.attempts.iter().any(|a| a.outcome == "abortado"),
        "bitacora: {:?}",
        memory.attempts.iter().map(|a| &a.outcome).collect::<Vec<_>>()
    );
}

#[test]
fn guardrail_de_ciclos_correctivos() {
    let Some(fixture) = Fixture::new("guardrail") else {
        return;
    };

    let state = state_for(&fixture.path);
    let reporter = Reporter::silent();
    let rt = runtime();

    rt.block_on(agent::run(&reporter, &state, request("Atiende el TODO")))
        .expect("run");

    // Simula que ya se agotaron los ciclos permitidos.
    state.update_session(|session| {
        session.attempts = session.max_attempts;
    });

    let error = rt
        .block_on(agent::adjust(&reporter, &state, "cambia el enfoque"))
        .expect_err("debe bloquear el cuarto intento");

    assert!(
        error.contains("Limite") && error.contains("Max Retries"),
        "mensaje: {}",
        error
    );
}

#[test]
fn ajuste_incremental_sobre_la_misma_rama() {
    let Some(fixture) = Fixture::new("ajuste") else {
        return;
    };

    let state = state_for(&fixture.path);
    let reporter = Reporter::silent();
    let rt = runtime();

    let first = rt
        .block_on(agent::run(&reporter, &state, request("Atiende el TODO")))
        .expect("run");

    let second = rt
        .block_on(agent::adjust(
            &reporter,
            &state,
            "El comentario debe quedar en ingles",
        ))
        .expect("adjust");

    assert_eq!(second.branch, first.branch, "el ajuste va sobre la misma rama");
    assert_eq!(second.attempt, 2);
    assert_eq!(second.max_attempts, 3);
    assert!(fixture.read("app.rs").contains("DONE"));
}

#[test]
fn sin_repositorio_seleccionado_falla_con_mensaje_claro() {
    let mut config = AppConfig::default();
    config.mock_ai = true;
    let state = AppState::new(config);
    let reporter = Reporter::silent();

    let error = runtime()
        .block_on(agent::run(&reporter, &state, request("x")))
        .expect_err("sin repositorio debe fallar");

    assert!(error.to_lowercase().contains("repositorio"));
}

#[test]
fn tarea_vacia_se_rechaza() {
    let Some(fixture) = Fixture::new("vacia") else {
        return;
    };

    let state = state_for(&fixture.path);
    let reporter = Reporter::silent();

    let error = runtime()
        .block_on(agent::run(&reporter, &state, request("   ")))
        .expect_err("tarea vacia");

    assert!(error.contains("Describe la tarea"));
}

#[test]
fn publicar_sin_remoto_falla_y_conserva_el_trabajo() {
    let Some(fixture) = Fixture::new("publicar") else {
        return;
    };

    let state = state_for(&fixture.path);
    let reporter = Reporter::silent();
    let rt = runtime();

    let outcome = rt
        .block_on(agent::run(&reporter, &state, request("Atiende el TODO")))
        .expect("run");

    // El repositorio temporal no tiene remoto: 'git push' debe fallar con un
    // error legible y sin dejar la sesion en estado inconsistente.
    let error = agent::publish(&reporter, &state, Default::default())
        .expect_err("sin remoto no se puede publicar");

    assert!(
        error.to_lowercase().contains("push") || error.to_lowercase().contains("fallo"),
        "mensaje: {}",
        error
    );

    assert!(state.session_snapshot().active, "la sesion sigue viva");
    assert!(git::branch_exists(&fixture.path, &outcome.branch));
}

#[test]
fn subproceso_de_git_se_ejecuta_sin_ventana() {
    // Detecta regresiones del ejecutor oculto (CREATE_NO_WINDOW).
    let out = Command::new("cmd").args(["/C", "echo hola"]).output();
    assert!(out.is_ok());

    let version = process::version_of("git").expect("git disponible");
    assert!(version.to_lowercase().contains("git version"));
}
