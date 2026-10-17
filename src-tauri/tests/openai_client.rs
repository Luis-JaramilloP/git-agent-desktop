//! Pruebas del cliente de inferencia contra un servidor OpenAI-compatible
//! falso levantado en localhost.
//!
//! Cubre el camino que usan Groq, OpenRouter, LM Studio, vLLM y Foundry sin
//! necesitar claves ni red: descubrimiento de modelos, prueba de conexion,
//! esquema de respuesta y los dos errores tipicos (ruta inexistente y el stub
//! "OK" en texto plano que devolvia el endpoint retirado de GitHub Models).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use git_agent_desktop_lib::ai::client::{list_models, test_connection};
use git_agent_desktop_lib::config::AppConfig;
use git_agent_desktop_lib::state::AppState;

#[derive(Clone, Copy)]
enum Mode {
    /// Endpoint compatible con OpenAI correcto.
    Full,
    /// 200 text/plain "OK": el caso del endpoint retirado.
    StubOk,
    /// 404 con JSON de error.
    NotFound,
}

struct FakeOpenAi {
    port: u16,
    hits: Arc<AtomicUsize>,
}

impl FakeOpenAi {
    fn start(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("puerto libre");
        let port = listener.local_addr().expect("addr").port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();

        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(stream) => stream,
                    Err(_) => continue,
                };
                counter.fetch_add(1, Ordering::SeqCst);

                let mut reader = BufReader::new(stream.try_clone().expect("clonar stream"));
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }

                // Cabeceras: se consume el cuerpo segun Content-Length.
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let trimmed = line.trim_end();
                    if trimmed.is_empty() {
                        break;
                    }
                    let lower = trimmed.to_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                }
                if content_length > 0 {
                    let mut body = vec![0u8; content_length];
                    let _ = reader.read_exact(&mut body);
                }

                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();

                let (status, content_type, payload) = match mode {
                    Mode::StubOk => ("200 OK", "text/plain", "OK".to_string()),
                    Mode::NotFound => (
                        "404 Not Found",
                        "application/json",
                        r#"{"error":{"code":404,"message":"modelo no encontrado"}}"#.to_string(),
                    ),
                    Mode::Full => {
                        if path.ends_with("/models") {
                            (
                                "200 OK",
                                "application/json",
                                r#"{"object":"list","data":[{"id":"mock-coder-7b"},{"id":"mock-coder-70b"}]}"#
                                    .to_string(),
                            )
                        } else if path.ends_with("/chat/completions") {
                            (
                                "200 OK",
                                "application/json",
                                r#"{"id":"chatcmpl-1","object":"chat.completion","model":"mock-coder-7b","choices":[{"index":0,"message":{"role":"assistant","content":"listo"},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":2,"total_tokens":9}}"#
                                    .to_string(),
                            )
                        } else {
                            ("404 Not Found", "application/json", r#"{"error":"ruta"}"#.to_string())
                        }
                    }
                };

                let response = format!(
                    "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    content_type,
                    payload.len(),
                    payload
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        Self { port, hits }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// Proveedor local (sin clave) apuntando al servidor falso.
fn state_for(server: &FakeOpenAi) -> AppState {
    let mut config = AppConfig::default();
    config.provider = "ollama".to_string(); // requiresKey = false
    config.base_url = server.base_url();
    config.model = "mock-coder-7b".to_string();
    config.max_tokens = 2048;
    AppState::new(config)
}

#[test]
fn lista_los_modelos_del_endpoint_compatible() {
    let server = FakeOpenAi::start(Mode::Full);
    let state = state_for(&server);

    let models = runtime()
        .block_on(list_models(&state))
        .expect("listar modelos");

    assert_eq!(models, vec!["mock-coder-70b", "mock-coder-7b"]);
    assert!(server.hits() >= 1, "debe haber consultado el endpoint");
}

#[test]
fn la_prueba_de_conexion_lee_la_respuesta_del_modelo() {
    let server = FakeOpenAi::start(Mode::Full);
    let state = state_for(&server);

    let test = runtime()
        .block_on(test_connection(&state))
        .expect("prueba de conexion");

    assert!(test.ok);
    assert_eq!(test.reply, "listo");
    assert_eq!(test.model, "mock-coder-7b");
    assert_eq!(test.total_tokens, 9);
    assert!(test.base_url.ends_with("/v1"));
}

#[test]
fn el_stub_ok_en_texto_plano_se_explica_con_claridad() {
    // Reproduce el fallo real: el endpoint retirado responde 200 "OK".
    let server = FakeOpenAi::start(Mode::StubOk);
    let state = state_for(&server);

    let error = runtime()
        .block_on(test_connection(&state))
        .expect_err("debe fallar");

    assert!(error.contains("OK"), "mensaje: {}", error);
    assert!(
        error.to_lowercase().contains("no json"),
        "debe indicar que no es JSON: {}",
        error
    );
}

#[test]
fn un_404_del_proveedor_se_propaga_con_el_detalle() {
    let server = FakeOpenAi::start(Mode::NotFound);
    let state = state_for(&server);

    let error = runtime()
        .block_on(test_connection(&state))
        .expect_err("debe fallar");

    assert!(error.contains("404"), "mensaje: {}", error);
    assert!(
        error.contains("modelo no encontrado"),
        "debe incluir el detalle del proveedor: {}",
        error
    );
}
