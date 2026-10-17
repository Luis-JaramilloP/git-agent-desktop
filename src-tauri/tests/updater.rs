//! Pruebas del actualizador: versiones, resolucion del instalador, integridad
//! y comportamiento cuando no hay canal configurado.

use git_agent_desktop_lib::config::AppConfig;
use git_agent_desktop_lib::state::AppState;
use git_agent_desktop_lib::updater::{self, is_newer, resolve_artifact, UpdateManifest};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime de tokio")
}

#[test]
fn compara_versiones_semver_simples() {
    assert!(is_newer("2.1.0", "2.0.0"));
    assert!(is_newer("2.1.1", "2.1.0"));
    assert!(is_newer("3.0.0", "2.9.9"));
    assert!(is_newer("2.10.0", "2.9.0"));
    assert!(is_newer("v2.1.0", "2.0.0"), "tolera el prefijo v");

    assert!(!is_newer("2.0.0", "2.0.0"), "igual no es mas nueva");
    assert!(!is_newer("1.9.9", "2.0.0"), "mas antigua no es actualizacion");
    assert!(!is_newer("2.0", "2.0.0"), "2.0 equivale a 2.0.0");

    // Un sufijo de pre-release no cuenta como version superior.
    assert!(is_newer("2.1.0-beta.1", "2.0.0"));
    assert!(!is_newer("2.0.0-beta", "2.0.0"));
}

#[test]
fn resuelve_el_artefacto_respecto_al_manifiesto() {
    // Base remota
    assert_eq!(
        resolve_artifact("https://releases.example.com/app", "GitAgentSetup.exe"),
        "https://releases.example.com/app/GitAgentSetup.exe"
    );
    // URL absoluta en el manifiesto tiene prioridad
    assert_eq!(
        resolve_artifact(
            "https://releases.example.com/app",
            "https://cdn.example.com/x.exe"
        ),
        "https://cdn.example.com/x.exe"
    );
    // Base local (carpeta)
    let local = resolve_artifact(r"C:\canal\updates", "GitAgentSetup.exe");
    assert!(local.starts_with(r"C:\canal\updates"));
    assert!(local.ends_with("GitAgentSetup.exe"));
}

#[test]
fn sha256_coincide_con_el_valor_conocido() {
    assert_eq!(
        updater::sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        updater::sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn el_manifiesto_admite_campos_opcionales() {
    let minimo: UpdateManifest =
        serde_json::from_str(r#"{"version":"2.1.0","url":"GitAgentSetup.exe"}"#).expect("minimo");
    assert_eq!(minimo.version, "2.1.0");
    assert_eq!(minimo.sha256, "");
    assert!(!minimo.mandatory);
    assert_eq!(minimo.size, 0);

    let completo: UpdateManifest = serde_json::from_str(
        r#"{"version":"2.1.0","notes":"Corrige X","url":"a.exe","sha256":"AB","size":123,"mandatory":true}"#,
    )
    .expect("completo");
    assert!(completo.mandatory);
    assert_eq!(completo.size, 123);
    assert_eq!(completo.notes, "Corrige X");
}

#[test]
fn sin_canal_configurado_no_hay_actualizacion() {
    let mut config = AppConfig::default();
    // Un canal inexistente evita depender del estado de la maquina.
    config.update_url = r"C:\ruta\que\no\existe\latest.json".to_string();

    let state = AppState::new(config);
    let error = runtime()
        .block_on(updater::check(&state))
        .expect_err("un manifiesto inexistente debe devolver error legible");

    assert!(
        error.contains("No se pudo leer el manifiesto"),
        "mensaje: {}",
        error
    );
}

#[test]
fn la_version_compilada_es_la_esperada() {
    // Si se publica una version nueva hay que subir tambien Cargo.toml.
    let actual = updater::CURRENT_VERSION;
    assert!(
        actual.split('.').count() >= 2,
        "la version debe ser semver: {}",
        actual
    );
    assert!(!actual.is_empty());
}

#[test]
fn la_info_de_version_expone_canal_y_ruta() {
    let state = AppState::new(AppConfig::default());
    let config = state.config_snapshot();
    let canal = updater::manifest_source(&config);
    assert!(
        canal.ends_with("latest.json"),
        "el canal por defecto apunta al manifiesto: {}",
        canal
    );
}
