//! Pruebas del catalogo de proveedores, resolucion de configuracion y almacen
//! cifrado de la clave (DPAPI).

use git_agent_desktop_lib::ai::client::{parse_model_ids, AiClient};
use git_agent_desktop_lib::ai::providers;
use git_agent_desktop_lib::config::AppConfig;
use git_agent_desktop_lib::secrets;
use git_agent_desktop_lib::state::AppState;

#[test]
fn el_catalogo_ofrece_alternativas_locales_y_marca_lo_retirado() {
    let presets = providers::presets();
    assert!(presets.len() >= 5, "debe haber varias opciones");

    let ollama = providers::find("ollama").expect("ollama");
    assert!(!ollama.requires_key, "Ollama no necesita clave");
    assert!(ollama.local);
    assert!(ollama.base_url.starts_with("http://127.0.0.1"));

    let retired = providers::find("github-models").expect("entrada historica");
    assert!(retired.retired, "GitHub Models debe figurar como retirado");

    for preset in &presets {
        if !preset.retired && preset.id != "custom" {
            assert!(
                preset.base_url.starts_with("http"),
                "el preset {} necesita URL base",
                preset.id
            );
        }
        if preset.requires_key {
            assert!(!preset.docs_url.is_empty(), "{} deberia enlazar a su consola", preset.id);
        }
    }
}

#[test]
fn la_url_efectiva_sale_del_preset_si_no_hay_personalizada() {
    let mut config = AppConfig::default();
    config.provider = "groq".to_string();
    config.base_url = String::new();
    assert_eq!(config.effective_base_url(), "https://api.groq.com/openai/v1");

    config.base_url = "http://127.0.0.1:8080/v1/".to_string();
    assert_eq!(config.effective_base_url(), "http://127.0.0.1:8080/v1");
}

#[test]
fn el_modelo_efectivo_cae_al_sugerido() {
    let mut config = AppConfig::default();
    config.provider = "gemini".to_string();
    config.model = String::new();
    assert_eq!(config.effective_model(), "gemini-3.8-flash");
}

#[test]
fn sanitize_corrige_proveedor_desconocido() {
    let mut config = AppConfig::default();
    config.provider = "proveedor-inexistente".to_string();
    let clean = config.sanitize();
    assert_eq!(clean.provider, "ollama");
}

#[test]
fn un_proveedor_retirado_falla_con_mensaje_accionable() {
    let mut config = AppConfig::default();
    config.provider = "github-models".to_string();
    config.model = "openai/gpt-4o".to_string();
    config.base_url = "https://models.github.ai/inference".to_string();

    let state = AppState::new(config);
    let error = AiClient::new(&state).expect_err("no debe permitir el proveedor retirado");

    assert!(error.contains("ya no existe"), "mensaje: {}", error);
    assert!(
        error.contains("Ajustes"),
        "debe indicar donde arreglarlo: {}",
        error
    );
}

#[test]
fn un_proveedor_con_clave_no_arranca_sin_clave() {
    // No se puede asumir que la maquina no tenga clave guardada.
    let status = secrets::status();
    if status.stored || status.from_env {
        eprintln!("hay una clave configurada: prueba omitida");
        return;
    }

    let mut config = AppConfig::default();
    config.provider = "groq".to_string();
    config.model = "llama-3.3-70b-versatile".to_string();
    config.base_url = String::new();

    let state = AppState::new(config);
    let error = AiClient::new(&state).expect_err("sin clave debe fallar");

    assert!(error.contains("Falta la clave"), "mensaje: {}", error);
}

#[test]
fn el_modo_prueba_no_exige_proveedor() {
    let mut config = AppConfig::default();
    config.mock_ai = true;
    config.provider = "ollama".to_string();
    config.model = "cualquiera".to_string();

    let state = AppState::new(config);
    let client = AiClient::new(&state).expect("el mock no necesita red ni clave");
    assert!(client.is_mock());
    assert_eq!(client.model(), "cualquiera");
}

#[cfg(windows)]
#[test]
fn la_clave_se_cifra_y_se_recupera_con_dpapi() {
    if secrets::env_key().is_some() {
        eprintln!("GAD_API_KEY definida: la lectura prioriza el entorno, prueba omitida");
        return;
    }

    // Se preserva cualquier clave real del usuario antes de tocar el almacen.
    let backup = secrets::load().ok().flatten();
    let sample = "clave-de-prueba-1234567890";

    let saved = secrets::save(sample).expect("guardar clave");
    assert!(saved.stored);

    let raw = std::fs::read_to_string(secrets::secret_path()).expect("leer blob");
    assert!(
        !raw.contains(sample),
        "el archivo no debe contener la clave en texto plano"
    );
    assert!(
        raw.chars().all(|c| c.is_ascii_hexdigit()),
        "el blob debe estar en hexadecimal"
    );

    let loaded = secrets::load().expect("leer clave").expect("clave presente");
    assert_eq!(loaded, sample, "DPAPI debe devolver la clave original");

    // Borrado explicito.
    let cleared = secrets::clear().expect("borrar clave");
    assert!(!cleared.stored, "tras borrar no debe quedar clave");
    assert!(
        secrets::load().expect("leer tras borrar").is_none(),
        "sin archivo no hay clave"
    );

    // Restauracion del estado previo del usuario.
    if let Some(previous) = backup {
        secrets::save(&previous).expect("restaurar clave previa");
    }
}

#[test]
fn interpreta_las_listas_de_modelos_del_proveedor() {
    use serde_json::json;

    // Formato OpenAI / Gemini / OpenRouter: {"data":[{"id":...}]}
    let openai = json!({"data": [{"id": "gemini-3.8-flash"}, {"id": "gemini-3.8-pro"}]});
    assert_eq!(
        parse_model_ids(&openai),
        vec!["gemini-3.8-flash", "gemini-3.8-pro"]
    );

    // Arreglo plano.
    let plain = json!([{"id": "llama-3.3-70b-versatile"}]);
    assert_eq!(parse_model_ids(&plain), vec!["llama-3.3-70b-versatile"]);

    // Variante con "name" y claves alternativas.
    let named = json!({"models": [{"name": "deepseek-chat"}, {"id": "deepseek-reasoner"}]});
    assert_eq!(parse_model_ids(&named), vec!["deepseek-chat", "deepseek-reasoner"]);

    // Google devuelve los ids con prefijo "models/": se normalizan.
    let google = json!({"data": [{"id": "models/gemini-3.8-flash"}]});
    assert_eq!(parse_model_ids(&google), vec!["gemini-3.8-flash"]);

    // Respuestas sin modelos no rompen.
    assert!(parse_model_ids(&json!({"error": {"code": 404}})).is_empty());
    assert!(parse_model_ids(&json!({"data": [{"id": ""}]})).is_empty());
}

#[test]
fn el_catalogo_incluye_deepseek_y_los_ids_vigentes_de_gemini() {
    let gemini = providers::find("gemini").expect("gemini");
    assert!(
        gemini.suggested_models.iter().any(|m| m.contains("gemini-3")),
        "los ids de Gemini deben ser recientes: {:?}",
        gemini.suggested_models
    );

    let deepseek = providers::find("deepseek").expect("deepseek directo");
    assert!(deepseek.requires_key);
    assert!(deepseek.notes.contains("NO gratuito"), "nota honesta sobre el coste");

    let openrouter = providers::find("openrouter").expect("openrouter");
    assert!(
        openrouter.suggested_models.iter().any(|m| m.ends_with(":free")),
        "OpenRouter debe ofrecer variantes gratuitas"
    );
}