//! Cliente HTTP de inferencia compatible con la API de chat de OpenAI.
//!
//! Contexto: **GitHub Models fue retirado el 30 de julio de 2026**; su endpoint
//! solo responde un stub `200 OK` en texto plano. Este cliente habla el
//! protocolo de OpenAI contra cualquier `{base}/chat/completions`
//! (Ollama, LM Studio, Gemini, Groq, OpenRouter, Foundry, vLLM...), con
//! Two-Stage Context Retrieval (Seccion 12.3) y tolerancia a cuotas.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ai::providers;
use crate::commands::patch::PatchBlock;
use crate::config::AppConfig;
use crate::secrets;
use crate::state::{AppState, Usage};

use super::prompts;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }
}

/// Plan de edicion devuelto por el modelo.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EditPlan {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub branch_name: Option<String>,
    #[serde(default)]
    pub commit_message: Option<String>,
    #[serde(default)]
    pub pr_title: Option<String>,
    #[serde(default)]
    pub pr_body: Option<String>,
    #[serde(default, alias = "patches", alias = "changes", alias = "blocks")]
    pub edits: Vec<PatchBlock>,
    #[serde(default, alias = "notes")]
    pub notas: Option<String>,
}

/// Resultado de la prueba de conexion del proveedor.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTest {    pub ok: bool,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub latency_ms: u64,
    pub reply: String,
    pub total_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct AiResponse {
    pub content: String,
    pub usage: Usage,
}

#[derive(Debug, Clone)]
enum AuthMode {
    /// Proveedor local sin autenticacion (Ollama, LM Studio).
    None,
    /// `Authorization: Bearer <clave>`.
    Bearer(String),
    /// Azure/Foundry: `Authorization: Bearer` + `api-key`.
    BearerAndApiKey(String),
}

#[derive(Debug)]
pub struct AiClient {
    http: reqwest::Client,
    config: AppConfig,
    base_url: String,
    model: String,
    auth: AuthMode,
}

impl AiClient {
    pub fn new(state: &AppState) -> Result<Self, String> {
        let config = state.config_snapshot().sanitize();
        let base_url = config.effective_base_url();
        let model = config.effective_model();

        if base_url.trim().is_empty() {
            return Err(format!(
                "El proveedor '{}' no tiene URL base. Abre Ajustes (⚙) y definela.",
                config.provider
            ));
        }
        if model.trim().is_empty() {
            return Err(format!(
                "El proveedor '{}' no tiene modelo configurado. Abre Ajustes (⚙) y escribe uno \
                 (por ejemplo: {}).",
                config.provider,
                providers::find(&config.provider)
                    .and_then(|preset| preset.suggested_models.first().cloned())
                    .unwrap_or_else(|| "qwen2.5-coder:7b".to_string())
            ));
        }

        let auth = resolve_auth(&config)?;

        Ok(Self {
            http: state.http().clone(),
            config,
            base_url,
            model,
            auth,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn is_mock(&self) -> bool {
        self.config.mock_ai
    }

    // -----------------------------------------------------------------
    // Transporte HTTP
    // -----------------------------------------------------------------

    async fn chat(&self, messages: &[ChatMessage], max_tokens: u32) -> Result<AiResponse, String> {
        if self.config.mock_ai {
            return Err(
                "El cliente esta en modo de pruebas (GAD_MOCK_AI); no hay llamada HTTP.".to_string(),
            );
        }

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": max_tokens,
            "stream": false,
        });

        if supports_temperature(&self.model) {
            body["temperature"] = json!(self.config.temperature);
        }

        let mut last_error = String::from("No se pudo contactar con el proveedor de IA.");

        for attempt in 1..=3u32 {
            let mut request = self
                .http
                .post(&url)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(&body);

            match &self.auth {
                AuthMode::None => {}
                AuthMode::Bearer(key) => {
                    request = request.bearer_auth(key);
                }
                AuthMode::BearerAndApiKey(key) => {
                    request = request.bearer_auth(key).header("api-key", key);
                }
            }

            let response = request.send().await;

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    let content_type = resp
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let text = resp.text().await.unwrap_or_default();

                    if status.is_success() {
                        return parse_response(&text, status.as_u16(), &content_type, &url);
                    }

                    let code = status.as_u16();
                    last_error = format!(
                        "El proveedor respondio HTTP {} en {}: {}",
                        code,
                        url,
                        truncate(&text, 700)
                    );

                    match code {
                        429 => {
                            let wait = 2u64.saturating_mul(attempt as u64);
                            tokio::time::sleep(Duration::from_secs(wait)).await;
                            continue;
                        }
                        500..=599 => {
                            tokio::time::sleep(Duration::from_secs(2)).await;
                            continue;
                        }
                        401 | 403 => {
                            return Err(format!(
                                "El proveedor rechazo las credenciales (HTTP {}). Revisa la clave en \
                                 Ajustes (⚙) o la variable GAD_API_KEY. Detalle: {}",
                                code,
                                truncate(&text, 400)
                            ));
                        }
                        404 => {
                            return Err(format!(
                                "HTTP 404: el modelo '{}' o la ruta '{}' no existen en este proveedor. \
                                 Verifica el id del modelo en Ajustes. Detalle: {}",
                                self.model,
                                url,
                                truncate(&text, 400)
                            ));
                        }
                        413 => {
                            return Err(
                                "El contexto enviado es demasiado grande (HTTP 413). Reduce el numero \
                                 de archivos de la tarea o baja maxContextChars."
                                    .to_string(),
                            );
                        }
                        _ => return Err(last_error),
                    }
                }
                Err(err) => {
                    let hint = if self.config.provider == "ollama"
                        || self.config.provider == "lmstudio"
                    {
                        format!(
                            " ¿Esta levantado el servidor local en {}? \
                             (Ollama: 'ollama serve' y 'ollama pull {}')",
                            self.base_url,
                            self.model
                        )
                    } else {
                        String::new()
                    };
                    return Err(format!(
                        "No se pudo conectar con {}: {}{}",
                        url, err, hint
                    ));
                }
            }
        }

        Err(last_error)
    }

    // -----------------------------------------------------------------
    // Descubrimiento de modelos disponibles
    // -----------------------------------------------------------------

    /// Consulta `GET {base}/models` y devuelve los ids disponibles.
    ///
    /// Evita depender de listas fijas: los proveedores retiran modelos a
    /// menudo (por ejemplo, Google dejo de ofrecer `gemini-2.5-flash` a
    /// cuentas nuevas y sugiere el id vigente en el propio error).
    pub async fn list_models(&self) -> Result<Vec<String>, String> {
        if self.config.mock_ai {
            return Err("El modo de pruebas no consulta modelos.".to_string());
        }

        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let mut request = self
            .http
            .get(&url)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(30));

        match &self.auth {
            AuthMode::None => {}
            AuthMode::Bearer(key) => {
                request = request.bearer_auth(key);
            }
            AuthMode::BearerAndApiKey(key) => {
                request = request.bearer_auth(key).header("api-key", key);
            }
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("No se pudo consultar {}: {}", url, e))?;

        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(format!(
                "El proveedor no devolvio la lista de modelos (HTTP {} en {}): {}",
                status.as_u16(),
                url,
                truncate(&text, 400)
            ));
        }

        let value: Value = serde_json::from_str(text.trim()).map_err(|_| {
            format!(
                "La lista de modelos no es JSON ({}). Respuesta: {}",
                url,
                truncate(&text, 300)
            )
        })?;

        let mut ids = parse_model_ids(&value);
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    // -----------------------------------------------------------------
    // Etapa 1: seleccion ligera de archivos
    // -----------------------------------------------------------------

    pub async fn select_files(
        &self,
        tree: &str,
        task: &str,
        issue: Option<u64>,
        allowed: &[String],
    ) -> Result<(Vec<String>, Usage), String> {
        if self.config.mock_ai {
            let files: Vec<String> = allowed.iter().take(2).cloned().collect();
            return Ok((files, Usage::default()));
        }

        let messages = vec![
            ChatMessage::system(prompts::PLANNER_SYSTEM),
            ChatMessage::user(prompts::planner_user_message(tree, task, issue)),
        ];

        let response = self.chat(&messages, 700).await?;
        let mut chosen: Vec<String> = Vec::new();

        if let Some(value) = extract_json(&response.content) {
            let raw_list: Vec<String> = if let Some(array) = value.as_array() {
                array
                    .iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect()
            } else if let Some(array) = value.get("files").and_then(|v| v.as_array()) {
                array
                    .iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect()
            } else if let Some(single) = value.get("files").and_then(|v| v.as_str()) {
                vec![single.to_string()]
            } else {
                Vec::new()
            };

            for candidate in raw_list {
                let normalized = candidate.trim().trim_start_matches("./").replace('\\', "/");
                if allowed.iter().any(|allowed_path| allowed_path == &normalized)
                    && !chosen.contains(&normalized)
                {
                    chosen.push(normalized);
                }
            }
        }

        if chosen.is_empty() {
            // Respaldo determinista: los dos primeros archivos del arbol.
            chosen = allowed.iter().take(2).cloned().collect();
        }

        chosen.truncate(4);
        Ok((chosen, response.usage))
    }

    // -----------------------------------------------------------------
    // Etapa 2: edicion focalizada
    // -----------------------------------------------------------------

    pub async fn generate_plan(
        &self,
        task: &str,
        files: &[(String, String)],
        issue: Option<u64>,
        feedback: Option<&str>,
        memory_context: &str,
    ) -> Result<(EditPlan, Usage), String> {
        if self.config.mock_ai {
            return Ok((mock_plan(task, files), Usage::default()));
        }

        let files_block = build_files_block(files, self.config.max_context_chars);

        let (system, user) = match feedback {
            Some(evidence) => (
                prompts::CORRECTOR_SYSTEM,
                prompts::corrector_user_message(task, &files_block, evidence, memory_context),
            ),
            None => (
                prompts::EDITOR_SYSTEM,
                prompts::editor_user_message(task, &files_block, issue),
            ),
        };

        let messages = vec![ChatMessage::system(system), ChatMessage::user(user)];

        let response = self
            .chat(&messages, self.config.max_tokens.min(16_000))
            .await?;

        let plan = parse_plan(&response.content)?;
        Ok((plan, response.usage))
    }
}

/// Resuelve el modo de autenticacion segun el proveedor configurado.
fn resolve_auth(config: &AppConfig) -> Result<AuthMode, String> {
    if config.mock_ai {
        return Ok(AuthMode::None);
    }

    let preset = providers::find(&config.provider);

    if let Some(preset) = &preset {
        if preset.retired {
            return Err(format!(
                "El proveedor '{}' ya no existe. {}\n\nAbre Ajustes (⚙) y elige otro: Ollama (local, \
                 gratis), Gemini, Groq u OpenRouter tienen plan gratuito.",
                preset.label, preset.notes
            ));
        }
    }

    let requires_key = preset.as_ref().map(|p| p.requires_key).unwrap_or(true);

    if let Some(key) = secrets::load()? {
        let azure = config.provider == "azure-foundry"
            || config.base_url.contains("services.ai.azure.com");
        return Ok(if azure {
            AuthMode::BearerAndApiKey(key)
        } else {
            AuthMode::Bearer(key)
        });
    }

    if requires_key {
        return Err(format!(
            "Falta la clave del proveedor '{}'.\n\nAbre Ajustes (⚙), pega la clave y pulsa \
             'Probar conexion'. Tambien puedes definirla en la variable GAD_API_KEY.",
            preset.map(|p| p.label).unwrap_or_else(|| config.provider.clone())
        ));
    }

    Ok(AuthMode::None)
}

/// Lista los modelos del proveedor configurado (comando IPC `ai_models`).
pub async fn list_models(state: &AppState) -> Result<Vec<String>, String> {
    AiClient::new(state)?.list_models().await
}

/// Prueba real del proveedor: envia un prompt minimo y devuelve la latencia.
pub async fn test_connection(state: &AppState) -> Result<AiTest, String> {
    let client = AiClient::new(state)?;

    if client.is_mock() {
        return Ok(AiTest {
            ok: true,
            provider: client.config.provider.clone(),
            model: client.model.clone(),
            base_url: client.base_url.clone(),
            latency_ms: 0,
            reply: "modo de pruebas (GAD_MOCK_AI): no se llamo al proveedor".to_string(),
            total_tokens: 0,
        });
    }

    let started = std::time::Instant::now();
    // Presupuesto amplio: los modelos con razonamiento (Gemini 3.x, o-series)
    // consumen tokens "pensando" y con un limite bajo devolverian contenido vacio.
    let response = client
        .chat(&[ChatMessage::user("Responde solo con la palabra: listo")], 512)
        .await?;

    Ok(AiTest {
        ok: true,
        provider: client.config.provider.clone(),
        model: client.model.clone(),
        base_url: client.base_url.clone(),
        latency_ms: started.elapsed().as_millis() as u64,
        reply: truncate(response.content.trim(), 300),
        total_tokens: response.usage.total_tokens,
    })
}

// ---------------------------------------------------------------------------
// Interpretacion de respuestas
// ---------------------------------------------------------------------------

fn supports_temperature(model: &str) -> bool {
    let lower = model.to_lowercase();
    !(lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
        || lower.contains("gpt-5"))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let head: String = text.chars().take(max).collect();
        format!("{}…", head)
    }
}

fn extract_content(value: &Value) -> Option<String> {
    let choice = value.get("choices")?.get(0)?;

    if let Some(message) = choice.get("message") {
        if let Some(content) = message.get("content") {
            if let Some(text) = content.as_str() {
                if !text.trim().is_empty() {
                    return Some(text.to_string());
                }
            }
            if let Some(parts) = content.as_array() {
                let joined: String = parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("");
                if !joined.trim().is_empty() {
                    return Some(joined);
                }
            }
        }
    }

    choice
        .get("text")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}

fn parse_response(
    text: &str,
    status: u16,
    content_type: &str,
    url: &str,
) -> Result<AiResponse, String> {
    let value: Value = serde_json::from_str(text).map_err(|_| {
        // Respuesta 2xx que no es JSON: el caso tipico es el stub "OK" que
        // devolvia el endpoint retirado de GitHub Models.
        let body = truncate(text.trim(), 200);
        let hint = if body.eq_ignore_ascii_case("ok") {
            " El cuerpo es literalmente \"OK\": ese host no expone la API de chat \
             (GitHub Models fue retirado el 30 de julio de 2026). Elige otro proveedor en Ajustes."
        } else {
            " Revisa la URL base del proveedor en Ajustes: debe aceptar POST {base}/chat/completions."
        };
        format!(
            "El proveedor respondio HTTP {} ({}) con contenido no JSON en {}: \"{}\".{}",
            status,
            if content_type.is_empty() { "sin content-type" } else { content_type },
            url,
            body,
            hint
        )
    })?;

    let content = match extract_content(&value) {
        Some(content) => content,
        None => {
            let finish = value
                .get("choices")
                .and_then(|choices| choices.get(0))
                .and_then(|choice| choice.get("finish_reason"))
                .and_then(|reason| reason.as_str())
                .unwrap_or("desconocido");

            let hint = if finish == "length" {
                "El modelo agoto 'maxTokens' antes de escribir la respuesta (finish_reason=length). \
                 Sube maxTokens en Ajustes: los modelos con razonamiento consumen presupuesto en pensar."
            } else {
                "Verifica que la URL base corresponda a un endpoint compatible con OpenAI."
            };

            return Err(format!(
                "La respuesta no contiene 'choices[0].message.content' (finish_reason='{}'). {} Respuesta: {}",
                finish,
                hint,
                truncate(text, 500)
            ));
        }
    };

    let usage = Usage {
        prompt_tokens: value
            .get("usage")
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        completion_tokens: value
            .get("usage")
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        total_tokens: value
            .get("usage")
            .and_then(|u| u.get("total_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
    };

    Ok(AiResponse { content, usage })
}

/// Extrae los ids de modelo de las formas habituales de `/models`.
pub fn parse_model_ids(value: &Value) -> Vec<String> {
    let collect = |items: &[Value]| -> Vec<String> {
        items
            .iter()
            .filter_map(|item| {
                item.get("id")
                    .or_else(|| item.get("name"))
                    .and_then(|id| id.as_str())
                    .map(|id| id.trim().trim_start_matches("models/").to_string())
                    .filter(|id| !id.is_empty())
            })
            .collect()
    };

    if let Some(array) = value.as_array() {
        return collect(array);
    }
    if let Some(array) = value.get("data").and_then(|v| v.as_array()) {
        return collect(array);
    }
    if let Some(array) = value.get("models").and_then(|v| v.as_array()) {
        return collect(array);
    }
    Vec::new()
}

/// Extrae el primer objeto/arreglo JSON valido de una respuesta del modelo,
/// tolerando bloques de codigo Markdown y texto alrededor.
pub fn extract_json(text: &str) -> Option<Value> {
    let trimmed = text.trim();

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }

    for fence in ["```json", "```JSON", "```"] {
        if let Some(start) = trimmed.find(fence) {
            let after = &trimmed[start + fence.len()..];
            if let Some(end) = after.rfind("```") {
                let candidate = after[..end].trim();
                if let Ok(value) = serde_json::from_str::<Value>(candidate) {
                    return Some(value);
                }
                if let Some(repaired) = repair_json(candidate) {
                    return Some(repaired);
                }
            }
        }
    }

    for (open, close) in [('{', '}'), ('[', ']')] {
        if let Some(start) = trimmed.find(open) {
            if let Some(end) = trimmed.rfind(close) {
                if end > start {
                    let candidate = &trimmed[start..=end];
                    if let Ok(value) = serde_json::from_str::<Value>(candidate) {
                        return Some(value);
                    }
                    if let Some(repaired) = repair_json(candidate) {
                        return Some(repaired);
                    }
                }
            }
        }
    }

    None
}

/// Reparaciones conservadoras: comas finales y comentarios de linea.
fn repair_json(candidate: &str) -> Option<Value> {
    let mut cleaned = String::with_capacity(candidate.len());
    for line in candidate.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        cleaned.push_str(line);
        cleaned.push('\n');
    }

    let mut previous;
    let mut current = cleaned;
    for _ in 0..12 {
        previous = current.clone();
        current = previous.replace(",}", "}").replace(",]", "]");
        if current == previous {
            break;
        }
    }

    serde_json::from_str::<Value>(&current).ok()
}

pub fn parse_plan(raw: &str) -> Result<EditPlan, String> {
    let value = extract_json(raw).ok_or_else(|| {
        format!(
            "El modelo no devolvio JSON valido. Respuesta recibida: {}",
            truncate(raw, 800)
        )
    })?;

    let plan: EditPlan = serde_json::from_value(value.clone()).map_err(|e| {
        format!(
            "El JSON del modelo no cumple el esquema esperado ({}). Datos: {}",
            e,
            truncate(&value.to_string(), 600)
        )
    })?;

    Ok(plan)
}

fn build_files_block(files: &[(String, String)], max_chars: usize) -> String {
    let mut block = String::new();
    let mut used = 0usize;

    for (path, content) in files {
        let remaining = max_chars.saturating_sub(used);
        if remaining < 400 {
            block.push_str(&format!(
                "\n### Archivo: {}\n(omitido: presupuesto de contexto agotado)\n",
                path
            ));
            continue;
        }

        let clipped: String = if content.chars().count() > remaining {
            let head: String = content.chars().take(remaining).collect();
            format!("{}\n... (contenido truncado por presupuesto de tokens)\n", head)
        } else {
            content.clone()
        };

        used += clipped.chars().count();
        block.push_str(&format!("\n### Archivo: {}\n```\n{}\n```\n", path, clipped));
    }

    block
}

// ---------------------------------------------------------------------------
// Modo de pruebas determinista (GAD_MOCK_AI=1)
// ---------------------------------------------------------------------------

/// Genera un plan valido sin red, para verificar el pipeline completo
/// (parcheo, diff, rama) sin credenciales ni proveedor externo.
fn mock_plan(task: &str, files: &[(String, String)]) -> EditPlan {
    let mut edits: Vec<PatchBlock> = Vec::new();

    for (path, content) in files {
        let marker = content.find("TODO").or_else(|| content.find("FIXME"));
        let Some(offset) = marker else { continue };

        let line_start = content[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_end = content[offset..]
            .find('\n')
            .map(|i| offset + i)
            .unwrap_or(content.len());
        let line = content[line_start..line_end].to_string();

        let occurrences = content.matches(line.as_str()).count();
        if occurrences != 1 || line.trim().is_empty() {
            continue;
        }

        let replaced = line.replace("TODO", "DONE").replace("FIXME", "DONE");
        edits.push(PatchBlock {
            file_path: path.clone(),
            search_content: line,
            replace_content: replaced,
        });
        break;
    }

    if edits.is_empty() {
        if let Some((path, content)) = files.first() {
            edits.push(PatchBlock {
                file_path: path.clone(),
                search_content: content.clone(),
                replace_content: format!(
                    "{}\n// Revisado por Git Agent Desktop (modo de prueba GAD_MOCK_AI)\n",
                    content.trim_end_matches('\n')
                ),
            });
        }
    }

    EditPlan {
        summary: format!("[MODO PRUEBA] Cambio simulado para la tarea: {}", task),
        branch_name: Some("agent/mock-prueba".to_string()),
        commit_message: Some("chore(agent): cambio de prueba generado en modo offline".to_string()),
        pr_title: Some("[PRUEBA] Cambio simulado por Git Agent Desktop".to_string()),
        pr_body: Some(
            "## Que cambia\nCambio generado en modo de pruebas determinista (sin proveedor externo).\n\n\
             ## Por que\nVerificacion del pipeline de parcheo atomico, diff y rama aislada.\n\n\
             ## Como se verifico\nEjecucion local con `GAD_MOCK_AI=1`."
                .to_string(),
        ),
        edits,
        notas: Some("Plan generado en modo GAD_MOCK_AI (sin red).".to_string()),
    }
}
