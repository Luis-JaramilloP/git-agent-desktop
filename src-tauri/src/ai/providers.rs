//! Catalogo de proveedores de inferencia compatibles con la API de chat de
//! OpenAI (`POST {base}/chat/completions`).
//!
//! Contexto: **GitHub Models fue retirado el 30 de julio de 2026** (playground,
//! catalogo, API de inferencia y BYOK), por lo que `models.github.ai` solo
//! devuelve un stub `200 OK` en texto plano. La app mantiene el id como
//! referencia historica, pero el valor por defecto es un runtime local.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPreset {
    pub id: String,
    pub label: String,
    pub base_url: String,
    pub requires_key: bool,
    pub local: bool,
    pub suggested_models: Vec<String>,
    pub notes: String,
    pub docs_url: String,
    /// Proveedor fuera de servicio (se avisa en la interfaz).
    pub retired: bool,
}

fn preset(
    id: &str,
    label: &str,
    base_url: &str,
    requires_key: bool,
    local: bool,
    models: &[&str],
    notes: &str,
    docs_url: &str,
) -> ProviderPreset {
    ProviderPreset {
        id: id.to_string(),
        label: label.to_string(),
        base_url: base_url.to_string(),
        requires_key,
        local,
        suggested_models: models.iter().map(|m| m.to_string()).collect(),
        notes: notes.to_string(),
        docs_url: docs_url.to_string(),
        retired: false,
    }
}

/// Catalogo en orden de recomendacion (primero lo gratuito y local).
pub fn presets() -> Vec<ProviderPreset> {
    vec![
        preset(
            "ollama",
            "Ollama (local, sin clave)",
            "http://127.0.0.1:11434/v1",
            false,
            true,
            &[
                "qwen2.5-coder:7b",
                "qwen2.5-coder:14b",
                "llama3.1:8b",
                "deepseek-coder-v2:16b",
            ],
            "Gratis y privado: el codigo no sale de tu equipo. Instala Ollama y descarga un modelo de codigo.",
            "https://ollama.com/download",
        ),
        preset(
            "lmstudio",
            "LM Studio (local, sin clave)",
            "http://127.0.0.1:1234/v1",
            false,
            true,
            &["qwen2.5-coder-7b-instruct", "llama-3.3-70b-instruct"],
            "Alternativa local con interfaz grafica; activa el servidor OpenAI-compatible en LM Studio.",
            "https://lmstudio.ai/",
        ),
        preset(
            "gemini",
            "Google AI Studio (Gemini, plan gratuito)",
            "https://generativelanguage.googleapis.com/v1beta/openai",
            true,
            false,
            &["gemini-3.8-flash", "gemini-3.8-pro"],
            "Tiene nivel gratuito. Google retira ids antiguos: pulsa 'Ver modelos disponibles' para \
             consultar los que acepta tu cuenta.",
            "https://aistudio.google.com/apikey",
        ),
        preset(
            "groq",
            "Groq (plan gratuito)",
            "https://api.groq.com/openai/v1",
            true,
            false,
            &["llama-3.3-70b-versatile", "qwen-2.5-coder-32b", "llama-3.1-8b-instant"],
            "Muy rapido y con cuota gratuita. Clave en la consola de Groq (empieza por gsk_).",
            "https://console.groq.com/keys",
        ),
        preset(
            "openrouter",
            "OpenRouter (modelos gratis y de pago)",
            "https://openrouter.ai/api/v1",
            true,
            false,
            &[
                "deepseek/deepseek-chat-v3-0324:free",
                "qwen/qwen-2.5-coder-32b-instruct:free",
                "meta-llama/llama-3.3-70b-instruct:free",
            ],
            "Agrega ':free' al id para usar los modelos gratuitos (incluido DeepSeek). Clave en openrouter.ai/keys.",
            "https://openrouter.ai/keys",
        ),
        preset(
            "deepseek",
            "DeepSeek (API directa, de pago)",
            "https://api.deepseek.com/v1",
            true,
            false,
            &["deepseek-chat", "deepseek-reasoner"],
            "Muy economico pero NO gratuito. Para DeepSeek sin coste usa OpenRouter con el sufijo ':free'.",
            "https://platform.deepseek.com/api_keys",
        ),        preset(
            "custom",
            "Otro endpoint compatible con OpenAI",
            "",
            true,
            false,
            &[],
            "Cualquier servidor que exponga POST {base}/chat/completions (vLLM, LocalAI, Foundry, DeepSeek...).",
            "https://platform.openai.com/docs/api-reference/chat",
        ),
        ProviderPreset {
            id: "github-models".to_string(),
            label: "GitHub Models (RETIRADO)".to_string(),
            base_url: "https://models.github.ai/inference".to_string(),
            requires_key: false,
            local: false,
            suggested_models: vec!["openai/gpt-4o".to_string()],
            notes: "Retirado el 30 de julio de 2026: el endpoint solo responde 'OK' en texto plano. \
                    Usa otro proveedor."
                .to_string(),
            docs_url: "https://docs.github.com/en/github-models/".to_string(),
            retired: true,
        },
    ]
}

pub fn find(id: &str) -> Option<ProviderPreset> {
    let wanted = id.trim().to_lowercase();
    presets().into_iter().find(|preset| preset.id == wanted)
}

/// Proveedores que no requieren clave (locales).
pub fn is_keyless(id: &str) -> bool {
    find(id).map(|preset| !preset.requires_key).unwrap_or(false)
}
