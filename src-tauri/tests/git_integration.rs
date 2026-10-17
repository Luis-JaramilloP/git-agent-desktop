//! Pruebas de integracion con Git real (Seccion 8 del plan).
//!
//! Se omiten automaticamente si `git` no esta disponible en el equipo.
//! En CI/entornos de prueba se puede apuntar a un binario concreto con
//! `GAD_GIT_BIN`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use git_agent_desktop_lib::commands::git;
use git_agent_desktop_lib::commands::patch::{apply, PatchBlock};
use git_agent_desktop_lib::utils::process;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempRepo {
    path: PathBuf,
}

impl TempRepo {
    fn new(tag: &str) -> Option<Self> {
        if process::version_of("git").is_none() {
            return None;
        }

        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "gad-git-{}-{}-{}",
            tag,
            std::process::id(),
            unique
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).ok()?;

        let repo = Self { path };

        // main determinista + identidad local para poder commitear.
        run(&repo.path, &["init", "-b", "main"]);
        run(&repo.path, &["config", "user.email", "agent@example.com"]);
        run(&repo.path, &["config", "user.name", "Git Agent Desktop"]);
        run(&repo.path, &["config", "commit.gpgsign", "false"]);

        fs::write(repo.path.join("app.txt"), "linea uno\nlinea dos\n").ok()?;
        fs::write(repo.path.join("README.md"), "# Demo\n").ok()?;
        run(&repo.path, &["add", "-A"]);
        run(&repo.path, &["commit", "-m", "commit inicial"]);

        Some(repo)
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn run(repo: &Path, args: &[&str]) -> String {
    process::run_cli("git", args, Some(repo)).unwrap_or_default()
}

#[test]
fn inspecciona_un_repositorio_real() {
    let Some(repo) = TempRepo::new("inspect") else {
        eprintln!("git no disponible: prueba omitida (establece GAD_GIT_BIN)");
        return;
    };

    let info = git::inspect(&repo.path).expect("inspect");
    assert!(info.is_git_repo);
    assert_eq!(info.current_branch.as_deref(), Some("main"));
    assert_eq!(info.default_branch.as_deref(), Some("main"));
    assert!(!info.dirty, "el repositorio recien creado debe estar limpio");
    assert!(info.head_commit.is_some());
    assert!(info.git_version.is_some());
}

#[test]
fn crea_rama_aislada_y_la_elimina_sin_commits() {
    let Some(repo) = TempRepo::new("branch") else {
        return;
    };

    let output = git::create_isolated_branch(&repo.path, "agent/prueba").expect("crear rama");
    assert!(
        output.contains("agent/prueba") || git::current_branch(&repo.path).unwrap() == "agent/prueba"
    );
    assert_eq!(git::current_branch(&repo.path).unwrap(), "agent/prueba");
    assert!(git::branch_exists(&repo.path, "agent/prueba"));

    // Crear la misma rama otra vez debe fallar de forma explicita.
    let error = git::create_isolated_branch(&repo.path, "agent/prueba").expect_err("duplicada");
    assert!(error.contains("ya existe"), "mensaje: {}", error);

    let log = git::cleanup_after_merge(&repo.path, "agent/prueba", "main").expect("limpieza");
    assert!(log.contains("agent/prueba") || log.contains("main"));
    assert_eq!(git::current_branch(&repo.path).unwrap(), "main");
    assert!(!git::branch_exists(&repo.path, "agent/prueba"));
}

#[test]
fn detecta_cambios_y_genera_diff_real() {
    let Some(repo) = TempRepo::new("diff") else {
        return;
    };

    git::create_isolated_branch(&repo.path, "agent/diff").expect("rama");

    let blocks = vec![PatchBlock {
        file_path: "app.txt".to_string(),
        search_content: "linea dos".to_string(),
        replace_content: "linea DOS".to_string(),
    }];
    let (changes, _snapshots, _warnings) = apply(&repo.path, &blocks).expect("aplicar");

    assert_eq!(git::status(&repo.path).expect("status").changed_files.len(), 1);

    git::stage_paths(&repo.path, &[changes[0].file_path.clone()]).expect("stage");
    let diff = git::staged_diff(&repo.path, &[changes[0].file_path.clone()]).expect("diff");

    assert!(diff.contains("-linea dos"), "diff: {}", diff);
    assert!(diff.contains("+linea DOS"), "diff: {}", diff);
    assert!(diff.starts_with("diff --git"), "formato git: {}", diff);
}

#[test]
fn el_diff_del_agente_sobrevive_al_commit() {
    let Some(repo) = TempRepo::new("commit") else {
        return;
    };

    git::create_isolated_branch(&repo.path, "agent/commit").expect("rama");
    fs::write(repo.path.join("nuevo.txt"), "contenido nuevo\n").expect("escribir");

    git::stage_paths(&repo.path, &["nuevo.txt".to_string()]).expect("stage");
    run(&repo.path, &["commit", "-m", "feat: agrega nuevo.txt", "--", "nuevo.txt"]);

    assert_eq!(git::commits_ahead(&repo.path, "main", "agent/commit"), 1);
    assert!(!git::status(&repo.path).expect("status").dirty);
    assert_eq!(
        git::get_full_diff(&repo.path).expect("diff").trim(),
        "",
        "tras commitear no debe quedar diff pendiente"
    );

    // El contenido del commit es el esperado.
    let show = run(&repo.path, &["show", "--stat", "--oneline", "-1"]);
    assert!(show.contains("nuevo.txt"), "git show: {}", show);
}

#[test]
fn restaura_archivos_tras_aplicar_un_parche() {
    let Some(repo) = TempRepo::new("restore") else {
        return;
    };

    git::create_isolated_branch(&repo.path, "agent/restore").expect("rama");
    let original = fs::read_to_string(repo.path.join("app.txt")).expect("leer");

    let blocks = vec![PatchBlock {
        file_path: "app.txt".to_string(),
        search_content: "linea uno".to_string(),
        replace_content: "linea UNO".to_string(),
    }];
    let (changes, snapshots, _warnings) = apply(&repo.path, &blocks).expect("aplicar");
    assert_ne!(fs::read_to_string(repo.path.join("app.txt")).unwrap(), original);

    git_agent_desktop_lib::commands::patch::restore(&repo.path, &snapshots).expect("restaurar");
    git::unstage_paths(&repo.path, &[changes[0].file_path.clone()]).ok();

    assert_eq!(fs::read_to_string(repo.path.join("app.txt")).unwrap(), original);
}

#[test]
fn las_rutas_de_trabajo_se_resuelven_al_toplevel() {
    let Some(repo) = TempRepo::new("toplevel") else {
        return;
    };

    let nested = repo.path.join("src").join("modulo");
    fs::create_dir_all(&nested).expect("crear subcarpeta");

    let info = git::inspect(&nested).expect("inspect desde subcarpeta");
    let reported = PathBuf::from(&info.path);
    let expected = fs::canonicalize(&repo.path).expect("canonical repo");
    let reported_canonical = fs::canonicalize(&reported).expect("canonical reportado");

    assert_eq!(reported_canonical, expected);
}

#[test]
fn ejecuta_comandos_sin_consola_visible() {
    // Verifica el resolutor de binarios y el ejecutor de subprocesos.
    let version = process::version_of("git");
    assert!(version.is_some(), "git debe resolverse via PATH o rutas de respaldo");

    let out = process::run_cli_any("git", &["--version"], None).expect("ejecutar git");
    assert!(out.ok());
    assert!(out.stdout.to_lowercase().contains("git version"));

    // Un comando inexistente debe devolver error, no panico.
    let missing = Command::new("gad-comando-inexistente").arg("--version").output();
    assert!(missing.is_err());
}

#[test]
fn el_path_del_hijo_incluye_git_y_gh() {
    // Regresion: `gh` necesita `git` en el PATH para `gh pr create`.
    // La app inyecta los directorios resueltos, aunque la consola que la
    // lanzo tuviera un PATH antiguo (por ejemplo, abierta antes de instalar Git).
    if process::version_of("git").is_none() {
        eprintln!("git no disponible: prueba omitida");
        return;
    }

    let dirs = process::augmented_path_entries();
    assert!(!dirs.is_empty(), "debe haber directorios que inyectar");

    let has_git = dirs.iter().any(|dir| {
        dir.join("git.exe").exists() || dir.join("git.cmd").exists()
    });
    assert!(
        has_git,
        "la lista debe contener la carpeta de git: {:?}",
        dirs
    );

    if process::version_of("gh").is_some() {
        let has_gh = dirs.iter().any(|dir| dir.join("gh.exe").exists());
        assert!(has_gh, "la lista debe contener la carpeta de gh: {:?}", dirs);
    }
}

#[test]
fn gh_resuelve_su_dependencia_de_git() {
    // Comprobacion real: `gh --version` y `gh auth status` a traves del
    // ejecutor de la app (que inyecta el PATH).
    if process::version_of("gh").is_none() {
        eprintln!("gh no disponible: prueba omitida");
        return;
    }

    let out = process::run_cli_any("gh", &["--version"], None).expect("ejecutar gh");
    assert!(out.ok());
    assert!(out.stdout.to_lowercase().contains("gh version"));
}
#[test]
fn borra_la_rama_remota_tras_integrar_el_pr() {
    let Some(repo) = TempRepo::new("remote") else {
        return;
    };

    // Remoto local (bare) para no depender de la red ni de credenciales.
    let bare = repo
        .path
        .with_file_name(format!("{}-origin.git", repo.path.file_name().unwrap().to_string_lossy()));
    let _ = fs::remove_dir_all(&bare);
    run(&repo.path, &["init", "--bare", bare.to_str().unwrap()]);
    run(&repo.path, &["remote", "add", "origin", bare.to_str().unwrap()]);
    run(&repo.path, &["push", "-u", "origin", "main"]);

    git::create_isolated_branch(&repo.path, "agent/remoto").expect("rama aislada");
    fs::write(repo.path.join("nuevo.txt"), "contenido\n").expect("escribir");
    run(&repo.path, &["add", "-A"]);
    run(&repo.path, &["commit", "-m", "feat: prueba de limpieza remota"]);
    run(&repo.path, &["push", "-u", "origin", "agent/remoto"]);

    assert!(
        git::remote_branch_live(&repo.path, "agent/remoto"),
        "la rama debe existir en el remoto antes de borrarla"
    );

    git::delete_remote_branch(&repo.path, "agent/remoto").expect("borrar rama remota");
    assert!(
        !git::remote_branch_live(&repo.path, "agent/remoto"),
        "tras integrar el PR la rama remota no debe seguir existiendo"
    );

    // Idempotente: si ya no existe (GitHub con borrado automatico) no falla.
    git::delete_remote_branch(&repo.path, "agent/remoto").expect("borrado idempotente");

    let _ = fs::remove_dir_all(&bare);
}