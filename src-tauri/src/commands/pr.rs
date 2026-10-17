//! Operaciones de Pull Request sobre GitHub CLI: lectura de estado, revisiones,
//! comentarios y logs de CI fallidos (Secciones 9, 10 y 11 del plan).

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::state::AppState;
use crate::utils::process;

/// Veredictos usados por la matriz de resolucion de eventos (Seccion 10).
pub const VERDICT_MERGED: &str = "MERGED";
pub const VERDICT_CHANGES_REQUESTED: &str = "CHANGES_REQUESTED";
pub const VERDICT_CI_FAILURE: &str = "CI_FAILURE";
pub const VERDICT_CONCEPTUAL_REJECTION: &str = "CONCEPTUAL_REJECTION";
pub const VERDICT_TECHNICAL_REJECTION: &str = "TECHNICAL_REJECTION";
pub const VERDICT_PENDING: &str = "PENDING";
pub const VERDICT_NONE: &str = "NONE";

const JSON_FIELDS: &str = "number,url,title,state,isDraft,reviewDecision,mergeable,baseRefName,headRefName,additions,deletions,files,comments,reviews,statusCheckRollup";

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CheckRun {
    pub name: String,
    pub status: String,
    pub conclusion: String,
    pub url: Option<String>,
    pub failed: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
    /// comment | review
    pub kind: String,
    pub state: Option<String>,
    pub path: Option<String>,
    pub line: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PrReport {
    pub found: bool,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub state: String,
    pub is_draft: bool,
    pub review_decision: Option<String>,
    pub mergeable: Option<String>,
    pub base_ref: String,
    pub head_ref: String,
    pub additions: u64,
    pub deletions: u64,
    pub files: Vec<String>,
    pub checks: Vec<CheckRun>,
    pub comments: Vec<PrComment>,
    pub verdict: String,
    pub verdict_reason: String,
    pub summary_text: String,
}

fn as_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn login_of(value: &Value) -> String {
    value
        .get("author")
        .and_then(|author| author.get("login"))
        .and_then(|login| login.as_str())
        .unwrap_or("desconocido")
        .to_string()
}

fn parse_checks(value: &Value) -> Vec<CheckRun> {
    let mut checks = Vec::new();
    if let Some(items) = value.as_array() {
        for item in items {
            let typename = item
                .get("__typename")
                .and_then(|v| v.as_str())
                .unwrap_or("CheckRun");

            if typename == "StatusContext" {
                let context = as_str(item, "context").unwrap_or_else(|| "status".to_string());
                let state = as_str(item, "state").unwrap_or_else(|| "PENDING".to_string());
                let failed = matches!(state.as_str(), "FAILURE" | "ERROR");
                checks.push(CheckRun {
                    name: context,
                    status: state.clone(),
                    conclusion: state,
                    url: as_str(item, "targetUrl"),
                    failed,
                });
                continue;
            }

            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("workflowName").and_then(|v| v.as_str()))
                .unwrap_or("check")
                .to_string();
            let status = as_str(item, "status").unwrap_or_else(|| "UNKNOWN".to_string());
            let conclusion = as_str(item, "conclusion").unwrap_or_else(|| "".to_string());
            let failed = matches!(
                conclusion.as_str(),
                "FAILURE" | "TIMED_OUT" | "CANCELLED" | "STARTUP_FAILURE" | "ACTION_REQUIRED"
            );
            checks.push(CheckRun {
                name,
                status,
                conclusion,
                url: as_str(item, "detailsUrl").or_else(|| as_str(item, "targetUrl")),
                failed,
            });
        }
    }
    checks
}

fn parse_comments(value: &Value) -> Vec<PrComment> {
    let mut comments = Vec::new();

    if let Some(items) = value.get("comments").and_then(|v| v.as_array()) {
        for item in items {
            comments.push(PrComment {
                author: login_of(item),
                body: as_str(item, "body").unwrap_or_default(),
                created_at: as_str(item, "createdAt").unwrap_or_default(),
                kind: "comment".to_string(),
                state: None,
                path: None,
                line: None,
            });
        }
    }

    if let Some(items) = value.get("reviews").and_then(|v| v.as_array()) {
        for item in items {
            let body = as_str(item, "body").unwrap_or_default();
            let state = as_str(item, "state");
            // Se incluyen reviews con cuerpo y tambien las que solo marcan
            // CHANGES_REQUESTED.
            if body.trim().is_empty() && state.as_deref() != Some("CHANGES_REQUESTED") {
                continue;
            }
            comments.push(PrComment {
                author: login_of(item),
                body,
                created_at: as_str(item, "submittedAt").unwrap_or_default(),
                kind: "review".to_string(),
                state,
                path: None,
                line: None,
            });
        }
    }

    comments
}

const CONCEPTUAL_MARKERS: &[&str] = &[
    "no agregaremos",
    "no vamos a agregar",
    "no lo agregaremos",
    "fuera de alcance",
    "fuera del alcance",
    "out of scope",
    "not planned",
    "wontfix",
    "won't fix",
    "no es deseado",
    "no encaja con la vision",
    "by design",
    "no forma parte del alcance",
    "no lo necesitamos",
    "we don't want",
    "we do not want",
    "declined",
    "rechazado por diseño",
    "no se aceptara",
    "no se aceptará",
];

fn looks_conceptual(text: &str) -> bool {
    let lower = text.to_lowercase();
    CONCEPTUAL_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Clasifica el estado del PR segun la matriz de la Seccion 10.
fn classify(
    state: &str,
    review_decision: &Option<String>,
    checks: &[CheckRun],
    comments: &[PrComment],
) -> (String, String) {
    match state {
        "MERGED" => (
            VERDICT_MERGED.to_string(),
            "El PR fue integrado a la rama base.".to_string(),
        ),
        "CLOSED" => {
            let closing: String = comments
                .iter()
                .rev()
                .take(4)
                .map(|c| c.body.clone())
                .collect::<Vec<_>>()
                .join("\n");

            if looks_conceptual(&closing) {
                (
                    VERDICT_CONCEPTUAL_REJECTION.to_string(),
                    "El cierre del PR indica que el cambio no se desea en el proyecto; \
                     se cancela la tarea sin abrir nuevos PRs."
                        .to_string(),
                )
            } else {
                (
                    VERDICT_TECHNICAL_REJECTION.to_string(),
                    "El PR se cerro sin integrar por motivos tecnicos; procede un plan nuevo en rama limpia."
                        .to_string(),
                )
            }
        }
        "OPEN" => {
            let failed: Vec<&CheckRun> = checks.iter().filter(|check| check.failed).collect();
            if !failed.is_empty() {
                let names = failed
                    .iter()
                    .map(|check| check.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                return (
                    VERDICT_CI_FAILURE.to_string(),
                    format!("GitHub Actions reporta fallos en: {}.", names),
                );
            }
            if review_decision.as_deref() == Some("CHANGES_REQUESTED") {
                return (
                    VERDICT_CHANGES_REQUESTED.to_string(),
                    "Un revisor solicito cambios sobre el PR.".to_string(),
                );
            }
            (
                VERDICT_PENDING.to_string(),
                "El PR esta abierto, sin fallos de CI ni cambios solicitados.".to_string(),
            )
        }
        _ => (
            VERDICT_NONE.to_string(),
            format!("Estado de PR no reconocido: {}", state),
        ),
    }
}

fn build_summary(report: &PrReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("Veredicto: {}\n", report.verdict));
    out.push_str(&format!("Motivo: {}\n", report.verdict_reason));
    out.push_str(&format!(
        "PR #{} \"{}\" ({}) base={} head={}\n",
        report.number, report.title, report.state, report.base_ref, report.head_ref
    ));

    let failed: Vec<&CheckRun> = report.checks.iter().filter(|c| c.failed).collect();
    if !failed.is_empty() {
        out.push_str("Checks fallidos:\n");
        for check in failed {
            out.push_str(&format!(
                "- {} ({}/{})\n",
                check.name, check.status, check.conclusion
            ));
        }
    }

    let relevant: Vec<&PrComment> = report
        .comments
        .iter()
        .filter(|comment| !comment.body.trim().is_empty())
        .collect();

    if !relevant.is_empty() {
        out.push_str("Comentarios y revisiones:\n");
        for comment in relevant.iter().rev().take(12) {
            out.push_str(&format!(
                "- [{}] {}: {}\n",
                comment.kind,
                comment.author,
                comment.body.trim()
            ));
        }
    }

    out
}

fn read_report(repo: &Path, pr: Option<&str>) -> Result<PrReport, String> {
    let mut args: Vec<&str> = vec!["pr", "view"];
    if let Some(reference) = pr {
        args.push(reference);
    }
    args.push("--json");
    args.push(JSON_FIELDS);

    let out = process::run_cli_raw("gh", &args, Some(repo), None)?;

    if !out.ok() {
        let combined = out.combined();
        let lower = combined.to_lowercase();
        if lower.contains("no pull requests found")
            || lower.contains("no open pull requests")
            || lower.contains("could not find")
        {
            return Ok(PrReport {
                found: false,
                verdict: VERDICT_NONE.to_string(),
                verdict_reason: "Aun no existe un Pull Request para esta rama.".to_string(),
                summary_text: combined,
                ..Default::default()
            });
        }
        return Err(format!("'gh pr view' fallo: {}", combined));
    }

    let value: Value = serde_json::from_str(out.stdout.trim())
        .map_err(|e| format!("Respuesta JSON inesperada de 'gh pr view': {}", e))?;

    let mut report = PrReport {
        found: true,
        number: value.get("number").and_then(|v| v.as_u64()).unwrap_or(0),
        url: as_str(&value, "url").unwrap_or_default(),
        title: as_str(&value, "title").unwrap_or_default(),
        state: as_str(&value, "state").unwrap_or_default(),
        is_draft: value.get("isDraft").and_then(|v| v.as_bool()).unwrap_or(false),
        review_decision: as_str(&value, "reviewDecision"),
        mergeable: as_str(&value, "mergeable"),
        base_ref: as_str(&value, "baseRefName").unwrap_or_default(),
        head_ref: as_str(&value, "headRefName").unwrap_or_default(),
        additions: value.get("additions").and_then(|v| v.as_u64()).unwrap_or(0),
        deletions: value.get("deletions").and_then(|v| v.as_u64()).unwrap_or(0),
        files: Vec::new(),
        checks: Vec::new(),
        comments: Vec::new(),
        verdict: VERDICT_NONE.to_string(),
        verdict_reason: String::new(),
        summary_text: String::new(),
    };

    if let Some(items) = value.get("files").and_then(|v| v.as_array()) {
        report.files = items
            .iter()
            .filter_map(|item| as_str(item, "path"))
            .collect();
    }

    if let Some(rollup) = value.get("statusCheckRollup") {
        report.checks = parse_checks(rollup);
    }

    report.comments = parse_comments(&value);

    let (verdict, reason) = classify(
        &report.state,
        &report.review_decision,
        &report.checks,
        &report.comments,
    );
    report.verdict = verdict;
    report.verdict_reason = reason;
    report.summary_text = build_summary(&report);

    Ok(report)
}

pub fn view(repo: &Path, pr: Option<&str>) -> Result<PrReport, String> {
    read_report(repo, pr)
}

/// Log de los checks fallidos del ultimo run de Actions (`gh run view --log-failed`).
pub fn failed_log(repo: &Path, limit_chars: usize) -> Result<String, String> {
    let out = process::run_cli_raw("gh", &["run", "view", "--log-failed"], Some(repo), None)?;

    if !out.ok() {
        let combined = out.combined();
        // Sin ejecuciones registradas no es un error fatal para el ciclo.
        if combined.to_lowercase().contains("no runs found") {
            return Ok(String::new());
        }
        return Err(format!("'gh run view --log-failed' fallo: {}", combined));
    }

    let text = out.stdout;
    if text.chars().count() <= limit_chars {
        return Ok(text);
    }

    // Se conserva la cola: alli aparecen los tracebacks relevantes.
    let tail: String = text
        .chars()
        .skip(text.chars().count().saturating_sub(limit_chars))
        .collect();
    Ok(format!(
        "(log truncado a los ultimos {} caracteres)\n{}",
        limit_chars, tail
    ))
}

pub fn comment(repo: &Path, body: &str) -> Result<String, String> {
    process::run_cli("gh", &["pr", "comment", "--body", body], Some(repo))
}

pub fn close(repo: &Path, body: &str) -> Result<String, String> {
    process::run_cli("gh", &["pr", "close", "--comment", body], Some(repo))
}

// ---------------------------------------------------------------------------
// Comandos IPC
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn pr_view(
    state: tauri::State<'_, AppState>,
    pr: Option<String>,
) -> Result<PrReport, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;
    view(&repo, pr.as_deref())
}

#[tauri::command]
pub fn pr_failed_log(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;
    failed_log(&repo, 9000)
}

#[tauri::command]
pub fn pr_comment(state: tauri::State<'_, AppState>, body: String) -> Result<String, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;
    comment(&repo, &body)
}

#[tauri::command]
pub fn pr_close(state: tauri::State<'_, AppState>, body: String) -> Result<String, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;
    close(&repo, &body)
}
