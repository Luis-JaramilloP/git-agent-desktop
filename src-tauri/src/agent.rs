//! Ciclo agentico completo: Entradas -> Razomiento+Accion -> HITL -> Salidas ->
//! Retroalimentacion (Secciones 9, 10 y 11 del plan).
//!
//! El agente NO publica nada de forma autonoma: `run`/`adjust` dejan los cambios
//! en disco y en una rama aislada, y solo `publish` (disparado por el usuario en
//! el visor de diferencias) hace commit + push + `gh pr create`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use crate::ai::client::AiClient;
use crate::ai::prompts;
use crate::commands::git;
use crate::commands::patch::{self, FileChange, PatchBlock};
use crate::commands::pr::{self, PrReport};
use crate::memory::{self, AttemptRecord, MemoryStore};
use crate::state::{AppState, Reporter, SessionState, Usage};
use crate::tree;

// ---------------------------------------------------------------------------
// Tipos IPC
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub task: String,
    pub issue: Option<u64>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PublishRequest {
    pub commit_message: Option<String>,
    pub pr_title: Option<String>,
    pub pr_body: Option<String>,
    pub base_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentOutcome {
    pub repo_path: String,
    pub branch: String,
    pub base_branch: String,
    pub summary: String,
    pub notes: Option<String>,
    pub changes: Vec<FileChange>,
    pub diff: String,
    pub commit_message: String,
    pub pr_title: String,
    pub pr_body: String,
    pub attempt: u32,
    pub max_attempts: u32,
    pub model: String,
    pub selected_files: Vec<String>,
    pub usage: Usage,
    pub warnings: Vec<String>,
    pub mock: bool,
    pub phase: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishResult {
    pub url: String,
    pub branch: String,
    pub base_branch: String,
    pub commit: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbortResult {
    pub restored: Vec<String>,
    pub branch: Option<String>,
    pub branch_deleted: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCheck {
    pub report: PrReport,
    #[serde(default)]
    pub remote_branch_deleted: bool,
    pub action: String,
    pub can_replan: bool,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub cleanup_performed: bool,
}

fn truncate_line(text: &str, max: usize) -> String {
    let single = text.lines().next().unwrap_or("").trim();
    if single.chars().count() <= max {
        single.to_string()
    } else {
        let short: String = single.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", short)
    }
}

/// Convierte texto libre en slug kebab-case apto para nombre de rama.
pub fn slugify(input: &str, max: usize) -> String {
    let mut out = String::new();
    let mut last_dash = false;

    for character in input.chars() {
        if out.chars().count() >= max {
            break;
        }
        if character.is_alphanumeric() {
            for lower in character.to_lowercase() {
                out.push(lower);
            }
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }

    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "tarea".to_string()
    } else {
        trimmed
    }
}

fn normalize_branch_name(candidate: &str, task: &str) -> String {
    let cleaned = candidate.trim().replace('\\', "/");
    let without_prefix = cleaned.strip_prefix("agent/").unwrap_or(&cleaned);
    let slug = slugify(without_prefix, 48);
    let slug = if slug == "tarea" { slugify(task, 40) } else { slug };
    format!("agent/{}", slug)
}

fn unique_branch(repo: &Path, base: &str) -> String {
    let mut candidate = base.to_string();
    let mut counter = 2;

    while git::branch_exists(repo, &candidate) || git::remote_branch_exists(repo, &candidate) {
        candidate = format!("{}-{}", base, counter);
        counter += 1;
        if counter > 60 {
            break;
        }
    }

    candidate
}

fn read_selected_files(
    repo: &Path,
    paths: &[String],
    max_file_bytes: usize,
    warnings: &mut Vec<String>,
) -> Vec<(String, String)> {
    let mut files = Vec::new();
    for path in paths {
        match tree::read_file(repo, path, max_file_bytes) {
            Ok(content) => files.push((path.clone(), content)),
            Err(err) => {
                warnings.push(err);
            }
        }
    }
    files
}

fn record_attempt(
    repo: &Path,
    session: &SessionState,
    outcome: &str,
    feedback: Option<&str>,
) -> Result<(), String> {
    let mut store = MemoryStore::load(repo);
    let record = AttemptRecord {
        attempt: session.attempts.max(1),
        timestamp: memory::now(),
        model: session.model.clone().unwrap_or_default(),
        task: session.task.clone().unwrap_or_default(),
        branch: session.branch.clone().unwrap_or_default(),
        summary: session.summary.clone().unwrap_or_default(),
        diff: session.last_diff.clone(),
        files: session.touched_files.clone(),
        outcome: outcome.to_string(),
        reviewer_feedback: feedback.map(|text| text.to_string()),
        pr_url: session.pr_url.clone(),
        usage_total_tokens: session.usage.as_ref().map(|usage| usage.total_tokens),
    };
    store.push(record)
}

// ---------------------------------------------------------------------------
// Aplicacion del plan (rama + parcheo + diff + sesion)
// ---------------------------------------------------------------------------

struct ApplyContext<'a> {
    repo: &'a Path,
    base_branch: &'a str,
    branch: &'a str,
    task: &'a str,
    model: &'a str,
    attempt: u32,
    max_attempts: u32,
    mock: bool,
}

#[allow(clippy::too_many_arguments)]
fn apply_plan(
    reporter: &Reporter,
    state: &AppState,
    context: ApplyContext<'_>,
    summary: &str,
    notes: Option<&str>,
    edits: &[PatchBlock],
    commit_message: &str,
    pr_title: &str,
    pr_body: &str,
    selected: &[String],
    usage: Usage,
    mut warnings: Vec<String>,
    fresh_branch: bool,
) -> Result<AgentOutcome, String> {
    let repo = context.repo;

    state.ensure_not_aborted()?;

    // 1. Rama aislada (Seccion 9, paso 2)
    if fresh_branch {
        let current = git::current_branch(repo).unwrap_or_default();
        if current != context.base_branch {
            let status = git::status(repo)?;
            if status.dirty {
                return Err(format!(
                    "Hay cambios sin confirmar en la rama '{}'. Confirma o descarta esos cambios \
                     antes de que el agente cree una rama nueva desde '{}'.",
                    current, context.base_branch
                ));
            }
            git::checkout(repo, context.base_branch)?;
            reporter.log(
                "info",
                format!("Checkout a la rama base '{}'.", context.base_branch),
            );
        }
        git::create_isolated_branch(repo, context.branch)?;
        reporter.log(
            "ok",
            format!("Rama aislada creada: {}", context.branch),
        );
    } else {
        let current = git::current_branch(repo).unwrap_or_default();
        if current != context.branch {
            git::checkout(repo, context.branch)?;
            reporter.log(
                "info",
                format!("Checkout a la rama del agente '{}'.", context.branch),
            );
        }
    }

    state.ensure_not_aborted()?;

    // 2. Parcheo atomico en disco (Seccion 7)
    let (changes, snapshots, patch_warnings) = patch::apply(repo, edits)?;
    warnings.extend(patch_warnings);

    for change in &changes {
        reporter.log(
            "ok",
            format!(
                "{} '{}' (+{} -{})",
                if change.created { "Creado" } else { "Editado" },
                change.file_path,
                change.additions,
                change.deletions
            ),
        );
    }

    let touched: Vec<String> = changes.iter().map(|change| change.file_path.clone()).collect();

    // 3. Diff real extraido de git (Seccion 9, paso 2)
    let mut diff = String::new();
    if git::stage_paths(repo, &touched).is_ok() {
        diff = git::staged_diff(repo, &touched).unwrap_or_default();
    }
    if diff.trim().is_empty() {
        // Respaldo: diff unificado calculado por el motor de parcheo.
        diff = changes
            .iter()
            .map(|change| change.diff.clone())
            .collect::<Vec<_>>()
            .join("\n");
    }

    // 4. Metadatos semanticos con respaldo determinista
    let final_summary = if summary.trim().is_empty() {
        format!("Cambios aplicados para: {}", context.task)
    } else {
        summary.trim().to_string()
    };

    let final_commit = if commit_message.trim().is_empty() {
        format!("chore(agent): {}", truncate_line(&final_summary, 60))
    } else {
        commit_message.trim().to_string()
    };

    let final_pr_title = if pr_title.trim().is_empty() {
        truncate_line(&final_summary, 70)
    } else {
        pr_title.trim().to_string()
    };

    let final_pr_body = if pr_body.trim().is_empty() {
        let files_list = touched
            .iter()
            .map(|file| format!("- `{}`", file))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "## Que cambia\n{}\n\n## Archivos\n{}\n\n## Tarea\n{}\n\n---\nGenerado por Git Agent Desktop (modelo `{}`).",
            final_summary, files_list, context.task, context.model
        )
    } else {
        pr_body.trim().to_string()
    };

    // 5. Estado de la sesion para el visor HITL
    let outcome_summary = final_summary.clone();
    let outcome_commit = final_commit.clone();
    let outcome_title = final_pr_title.clone();
    let outcome_body = final_pr_body.clone();
    let outcome_diff = diff.clone();
    let repo_string = repo.to_string_lossy().to_string();
    let branch_string = context.branch.to_string();
    let base_string = context.base_branch.to_string();
    let model_string = context.model.to_string();
    let selected_owned = selected.to_vec();
    let touched_owned = touched.clone();
    let snapshots_owned = snapshots.clone();
    let usage_owned = usage.clone();

    state.update_session(|session| {
        session.active = true;
        session.repo_path = Some(repo_string.clone());
        session.task = Some(context.task.to_string());
        session.model = Some(model_string.clone());
        session.branch = Some(branch_string.clone());
        session.base_branch = Some(base_string.clone());
        session.attempts = context.attempt;
        session.max_attempts = context.max_attempts;
        session.phase = "review".to_string();
        session.summary = Some(outcome_summary.clone());
        session.selected_files = selected_owned.clone();
        session.touched_files = touched_owned.clone();
        session.commit_message = Some(outcome_commit.clone());
        session.pr_title = Some(outcome_title.clone());
        session.pr_body = Some(outcome_body.clone());
        session.last_diff = outcome_diff.clone();
        session.usage = Some(usage_owned.clone());
        session.snapshot = snapshots_owned.clone();
    });

    reporter.log(
        "agent",
        format!(
            "Propuesta lista en '{}': {} archivo(s). Revisa el diff antes de aprobar.",
            context.branch,
            changes.len()
        ),
    );

    Ok(AgentOutcome {
        repo_path: repo_string,
        branch: branch_string,
        base_branch: base_string,
        summary: outcome_summary,
        notes: notes.map(|text| text.to_string()),
        changes,
        diff: outcome_diff,
        commit_message: outcome_commit,
        pr_title: outcome_title,
        pr_body: outcome_body,
        attempt: context.attempt,
        max_attempts: context.max_attempts,
        model: model_string,
        selected_files: selected_owned,
        usage,
        warnings,
        mock: context.mock,
        phase: "review".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Entrada: ejecutar la tarea
// ---------------------------------------------------------------------------

pub async fn run(
    reporter: &Reporter,
    state: &AppState,
    request: RunRequest,
) -> Result<AgentOutcome, String> {
    let task = request.task.trim().to_string();
    if task.is_empty() {
        return Err("Describe la tarea antes de ejecutar el agente.".to_string());
    }

    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;

    // Evita pisar una propuesta pendiente de revision.
    let previous = state.session_snapshot();
    if previous.active && previous.phase == "review" && !previous.touched_files.is_empty() {
        return Err(
            "Hay una propuesta pendiente de revision en el visor de diferencias. \
             Apruébala (o ábrela como PR) o aborta la sesion antes de ejecutar una tarea nueva."
                .to_string(),
        );
    }

    state.clear_abort();
    state.clear_session();

    let mut config = state.config_snapshot().sanitize();
    if let Some(model) = request.model.as_ref() {
        if !model.trim().is_empty() {
            config.model = model.trim().to_string();
        }
    }
    state.set_config(config.clone());

    reporter.log(
        "info",
        format!("Repositorio activo: {}", repo.to_string_lossy()),
    );

    // Verificaciones previas
    let info = git::inspect(&repo)?;
    if !info.is_git_repo {
        return Err(format!(
            "'{}' no es un repositorio Git. Inicializa uno con 'git init'.",
            info.path
        ));
    }

    let base_branch = info
        .default_branch
        .clone()
        .unwrap_or_else(|| "main".to_string());

    let mut warnings: Vec<String> = Vec::new();
    if info.dirty {
        warnings.push(format!(
            "El repositorio tenia {} archivo(s) modificados antes de iniciar; el agente solo \
             propone cambios sobre los archivos que toca.",
            info.changed_files
        ));
        reporter.log( "warn", warnings.last().cloned().unwrap_or_default());
    }

    if !config.mock_ai {
        let auth = crate::commands::auth::status();
        if !auth.authenticated {
            return Err(
                "No hay una sesion activa de GitHub: el motor de IA necesita el token de 'gh'. \
                 Pulsa 'Conectar cuenta de GitHub' y reintenta."
                    .to_string(),
            );
        }
    }

    let client = AiClient::new(state)?;

    // ---------------- Etapa 1: seleccion ligera de archivos ----------------
    reporter.log( "agent", "Etapa 1/2: buscando los archivos relevantes...");
    let repo_tree = tree::build(&repo, config.max_files_in_tree);

    if !repo_tree.is_git_repo {
        warnings.push(
            "No se pudo usar 'git ls-files'; el arbol se construyo recorriendo el sistema de archivos."
                .to_string(),
        );
    }
    if !repo_tree.skipped_secrets.is_empty() {
        reporter.log(
            "info",
            format!(
                "Se excluyeron {} archivo(s) con apariencia de secretos (.env, claves).",
                repo_tree.skipped_secrets.len()
            ),
        );
    }

    reporter.log(
        "info",
        format!(
            "Arbol filtrado: {} archivos visibles ({} totales).",
            repo_tree.shown, repo_tree.total_files
        ),
    );

    if repo_tree.files.is_empty() {
        return Err(
            "No hay archivos de texto editables en el repositorio (todo parece binario o ignorado)."
                .to_string(),
        );
    }

    let (selected, usage_stage_one) = client
        .select_files(
            &repo_tree.prompt_view(),
            &task,
            request.issue,
            &repo_tree.files,
        )
        .await?;

    if selected.is_empty() {
        return Err("El modelo no selecciono archivos editables para esta tarea.".to_string());
    }

    reporter.log(
        "ok",
        format!("Archivos seleccionados: {}", selected.join(", ")),
    );

    state.ensure_not_aborted()?;

    // ---------------- Etapa 2: edicion focalizada ----------------
    reporter.log( "agent", "Etapa 2/2: generando bloques SEARCH/REPLACE...");
    let files_content = read_selected_files(&repo, &selected, config.max_file_bytes, &mut warnings);
    if files_content.is_empty() {
        return Err("No se pudo leer ninguno de los archivos seleccionados.".to_string());
    }

    let memory_store = MemoryStore::load(&repo);
    let memory_context = memory_store.corrective_context(3, 4000);

    let (plan, usage_stage_two) = client
        .generate_plan(
            &task,
            &files_content,
            request.issue,
            None,
            &memory_context,
        )
        .await?;

    let mut usage = usage_stage_one;
    usage.accumulate(&usage_stage_two);

    if plan.edits.is_empty() {
        let detail = plan
            .notas
            .clone()
            .unwrap_or_else(|| "sin detalle adicional".to_string());
        return Err(format!(
            "El modelo no propuso ediciones para esta tarea ({}). Reformula la instruccion.",
            detail
        ));
    }

    reporter.log(
        "info",
        format!(
            "Plan recibido: {} bloque(s) sobre {} archivo(s). Tokens: {}",
            plan.edits.len(),
            plan.edits
                .iter()
                .map(|edit| edit.file_path.clone())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            usage.total_tokens
        ),
    );

    let requested = plan
        .branch_name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("agent/{}", slugify(&task, 40)));
    let branch = unique_branch(&repo, &normalize_branch_name(&requested, &task));

    let summary = plan.summary.clone();
    let notes = plan.notas.clone();
    let edits = plan.edits.clone();
    let commit_message = plan.commit_message.clone().unwrap_or_default();
    let pr_title = plan.pr_title.clone().unwrap_or_default();
    let pr_body = plan.pr_body.clone().unwrap_or_default();

    apply_plan(
        reporter,
        state,
        ApplyContext {
            repo: &repo,
            base_branch: &base_branch,
            branch: &branch,
            task: &task,
            model: &config.model,
            attempt: 1,
            max_attempts: config.max_attempts,
            mock: client.is_mock(),
        },
        &summary,
        notes.as_deref(),
        &edits,
        &commit_message,
        &pr_title,
        &pr_body,
        &selected,
        usage,
        warnings,
        true,
    )
}

// ---------------------------------------------------------------------------
// Self-correction: ajuste solicitado por el usuario o por el revisor
// ---------------------------------------------------------------------------

async fn replan_internal(
    reporter: &Reporter,
    state: &AppState,
    feedback: &str,
    fresh_branch: bool,
) -> Result<AgentOutcome, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;

    let session = state.session_snapshot();
    if !session.active {
        return Err("No hay una tarea activa: ejecuta primero el agente.".to_string());
    }

    let feedback = feedback.trim().to_string();
    if feedback.is_empty() {
        return Err("Describe el ajuste que debe aplicarse.".to_string());
    }

    // Guardrail Max Retries = 3 (Seccion 11)
    if session.attempts >= session.max_attempts {
        return Err(format!(
            "Limite de {} ciclos correctivos alcanzado (guardrail Max Retries). \
             La ejecucion automatica se detiene: revisa el PR manualmente o aborta la tarea.",
            session.max_attempts
        ));
    }

    state.ensure_not_aborted()?;

    let attempt = session.attempts + 1;
    let max_attempts = session.max_attempts.max(1);
    let task = session.task.clone().unwrap_or_default();
    let base_branch = session
        .base_branch
        .clone()
        .unwrap_or_else(|| "main".to_string());
    let config = state.config_snapshot().sanitize();

    record_attempt(&repo, &session, "rechazado", Some(&feedback))?;
    reporter.log(
        "agent",
        format!(
            "Ciclo correctivo {}/{}: re-planificando con la evidencia recibida...",
            attempt, max_attempts
        ),
    );

    let client = AiClient::new(state)?;

    // Archivos objetivo: los ya tocados + los preseleccionados.
    let mut targets: Vec<String> = session.touched_files.clone();
    for file in &session.selected_files {
        if !targets.contains(file) {
            targets.push(file.clone());
        }
    }

    let mut warnings: Vec<String> = Vec::new();
    let mut files_content =
        read_selected_files(&repo, &targets, config.max_file_bytes, &mut warnings);

    // Si el contenido actual ya no es legible, recalcula el arbol.
    if files_content.is_empty() {
        let repo_tree = tree::build(&repo, config.max_files_in_tree);
        let (selected, _usage) = client
            .select_files(&repo_tree.prompt_view(), &task, session.issue, &repo_tree.files)
            .await?;
        files_content = read_selected_files(&repo, &selected, config.max_file_bytes, &mut warnings);
    }

    if files_content.is_empty() {
        return Err(
            "No se pudieron leer los archivos necesarios para el ciclo correctivo.".to_string(),
        );
    }

    let evidence = format!("{}\n\n{}", feedback, prompts::NEGATIVE_DIRECTIVE);
    let memory_store = MemoryStore::load(&repo);
    let memory_context = memory_store.corrective_context(3, 6000);

    let (plan, usage) = client
        .generate_plan(
            &task,
            &files_content,
            session.issue,
            Some(&evidence),
            &memory_context,
        )
        .await
        .map_err(|err| format!("No se pudo generar el plan correctivo: {}", err))?;

    // Clasificador de rechazo conceptual (Seccion 11)
    if plan.edits.is_empty() {
        let detail = plan
            .notas
            .clone()
            .unwrap_or_else(|| "sin detalle adicional".to_string());
        state.update_session(|current| {
            current.active = false;
            current.phase = "rejected".to_string();
        });
        return Err(format!(
            "CONCEPTUAL_REJECTION: el modelo no propone cambios porque el enfoque no aplica ({}). \
             La tarea se cancela sin abrir nuevos PRs.",
            detail
        ));
    }

    let branch = if fresh_branch {
        let slug = slugify(&task, 32);
        unique_branch(&repo, &format!("agent/{}-v{}", slug, attempt))
    } else {
        session
            .branch
            .clone()
            .ok_or_else(|| "La sesion no tiene una rama asociada.".to_string())?
    };

    let summary = plan.summary.clone();
    let notes = plan.notas.clone();
    let edits = plan.edits.clone();
    let commit_message = plan.commit_message.clone().unwrap_or_default();
    let pr_title = plan.pr_title.clone().unwrap_or_else(|| {
        format!(
            "{} (ciclo correctivo {})",
            truncate_line(&task, 55),
            attempt
        )
    });
    let pr_body = plan.pr_body.clone().unwrap_or_default();

    let selected: Vec<String> = files_content
        .iter()
        .map(|(path, _)| path.clone())
        .collect();

    apply_plan(
        reporter,
        state,
        ApplyContext {
            repo: &repo,
            base_branch: &base_branch,
            branch: &branch,
            task: &task,
            model: &config.model,
            attempt,
            max_attempts,
            mock: client.is_mock(),
        },
        &summary,
        notes.as_deref(),
        &edits,
        &commit_message,
        &pr_title,
        &pr_body,
        &selected,
        usage,
        warnings,
        fresh_branch,
    )
}

pub async fn adjust(
    reporter: &Reporter,
    state: &AppState,
    feedback: &str,
) -> Result<AgentOutcome, String> {
    replan_internal(reporter, state, feedback, false).await
}

// ---------------------------------------------------------------------------
// Salidas: publicacion del PR (siempre tras aprobacion humana)
// ---------------------------------------------------------------------------

pub fn publish(
    reporter: &Reporter,
    state: &AppState,
    request: PublishRequest,
) -> Result<PublishResult, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;

    let session = state.session_snapshot();
    if !session.active {
        return Err("No hay una propuesta pendiente de publicar.".to_string());
    }
    if session.touched_files.is_empty() {
        return Err("La propuesta no contiene archivos modificados.".to_string());
    }

    let branch = session
        .branch
        .clone()
        .ok_or_else(|| "La sesion no tiene una rama asociada.".to_string())?;

    let base_branch = request
        .base_branch
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| session.base_branch.clone())
        .unwrap_or_else(|| "main".to_string());

    let commit_message = request
        .commit_message
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| session.commit_message.clone())
        .unwrap_or_else(|| {
            format!(
                "chore(agent): {}",
                truncate_line(
                    session.summary.as_deref().unwrap_or("cambios del agente"),
                    60
                )
            )
        });

    let pr_title = request
        .pr_title
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| session.pr_title.clone())
        .unwrap_or_else(|| {
            truncate_line(
                session.summary.as_deref().unwrap_or("Cambios del agente"),
                70,
            )
        });

    let pr_body = request
        .pr_body
        .clone()
        .or_else(|| session.pr_body.clone())
        .unwrap_or_default();

    // El checkout debe estar en la rama del agente antes de commitear.
    let current = git::current_branch(&repo)?;
    if current != branch {
        git::checkout(&repo, &branch)?;
    }

    reporter.log(
        "agent",
        format!(
            "Publicando en GitHub: commit + push de '{}' + 'gh pr create --base {}'...",
            branch, base_branch
        ),
    );

    let url = git::commit_and_create_pr(
        &repo,
        &branch,
        &commit_message,
        &pr_title,
        &pr_body,
        &base_branch,
        &session.touched_files,
    )?;

    let commit = git::commit_sha(&repo);
    let pr_number = url
        .rsplit('/')
        .next()
        .and_then(|tail| tail.parse::<u64>().ok());

    state.update_session(|current| {
        current.phase = "published".to_string();
        current.pr_url = Some(url.clone());
        current.pr_number = pr_number;
        current.pr_title = Some(pr_title.clone());
        current.pr_body = Some(pr_body.clone());
        current.commit_message = Some(commit_message.clone());
        current.base_branch = Some(base_branch.clone());
    });

    let published_session = state.session_snapshot();
    if let Err(err) = record_attempt(&repo, &published_session, "aprobado", None) {
        reporter.log( "warn", format!("No se pudo guardar la bitacora: {}", err));
    }

    reporter.log( "ok", format!("Pull Request publicado: {}", url));

    Ok(PublishResult {
        message: "Pull Request creado correctamente.".to_string(),
        url,
        branch,
        base_branch,
        commit,
    })
}

// ---------------------------------------------------------------------------
// Abortar sesion
// ---------------------------------------------------------------------------

pub fn abort(reporter: &Reporter, state: &AppState) -> Result<AbortResult, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;

    let session = state.session_snapshot();
    state.request_abort();

    let mut restored: Vec<String> = Vec::new();
    if !session.snapshot.is_empty() {
        restored = patch::restore(&repo, &session.snapshot)?;
    }

    git::unstage_paths(&repo, &session.touched_files).ok();

    let mut branch_deleted = false;
    let mut branch_name = session.branch.clone();

    if let Some(branch) = session.branch.clone() {
        let base = session
            .base_branch
            .clone()
            .unwrap_or_else(|| "main".to_string());

        if branch != base {
            // Solo se elimina si no tiene commits propios (si ya se publico, el
            // PR vive en GitHub y la rama se conserva).
            let ahead = git::commits_ahead(&repo, &base, &branch);
            if ahead == 0 {
                if git::current_branch(&repo).ok().as_deref() == Some(branch.as_str()) {
                    let _ = git::checkout(&repo, &base);
                }
                branch_deleted = git::branch_exists(&repo, &branch) && git::delete_branch(&repo, &branch).is_ok();
            } else {
                reporter.log(
                    "warn",
                    format!(
                        "La rama '{}' tiene {} commit(s): se conserva. Eliminala manualmente si ya no la necesitas.",
                        branch, ahead
                    ),
                );
            }
        }
        branch_name = Some(branch);
    }

    if session.active {
        if let Err(err) = record_attempt(&repo, &session, "abortado", None) {
            reporter.log( "warn", format!("No se pudo guardar la bitacora: {}", err));
        }
    }

    state.clear_session();
    state.clear_abort();

    reporter.log( "warn", "Sesion abortada: se restauro el estado previo de los archivos.");

    Ok(AbortResult {
        message: if restored.is_empty() {
            "No habia cambios que revertir.".to_string()
        } else {
            format!(
                "Se restauraron {} archivo(s) a su contenido previo.",
                restored.len()
            )
        },
        restored,
        branch: branch_name,
        branch_deleted,
    })
}

// ---------------------------------------------------------------------------
// Retroalimentacion: estado del PR y re-planificacion (Seccion 10)
// ---------------------------------------------------------------------------

pub fn check_pr(reporter: &Reporter, state: &AppState) -> Result<PrCheck, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;

    let session = state.session_snapshot();
    let reference = session
        .pr_number
        .map(|number| number.to_string())
        .or_else(|| session.pr_url.as_ref().map(|url| url.clone()));

    let report = pr::view(&repo, reference.as_deref())?;

    let action: String;
    let mut can_replan = false;
    let mut cleanup_performed = false;
    let mut remote_branch_deleted = false;
    let mut status_update: Option<(String, bool)> = None;

    match report.verdict.as_str() {
        pr::VERDICT_MERGED => {
            let base = session
                .base_branch
                .clone()
                .or_else(|| (!report.base_ref.is_empty()).then(|| report.base_ref.clone()))
                .unwrap_or_else(|| "main".to_string());
            let branch = session
                .branch
                .clone()
                .or_else(|| (!report.head_ref.is_empty()).then(|| report.head_ref.clone()));

            if let Some(branch) = branch.as_deref() {
                match git::cleanup_after_merge(&repo, branch, &base) {
                    Ok(log) => {
                        cleanup_performed = true;
                        reporter.log("ok", format!("Limpieza posterior al merge: {}", log));
                    }
                    Err(err) => reporter.log("warn", format!("Limpieza incompleta: {}", err)),
                }

                // Borrado de la rama remota: completa la limpieza del ciclo.
                match git::delete_remote_branch(&repo, branch) {
                    Ok(()) => {
                        remote_branch_deleted = true;
                        reporter.log(
                            "ok",
                            format!("Rama remota '{}' eliminada en origin.", branch),
                        );
                    }
                    Err(err) => reporter.log("warn", format!("{}", err)),
                }
            }

            action = if remote_branch_deleted {
                "PR integrado. Se actualizo la rama base, se limpio la rama local y se elimino la rama remota."
                    .to_string()
            } else {
                "PR integrado. Se actualiza la rama base y se limpia la rama local.".to_string()
            };

            if session.active {
                let _ = record_attempt(&repo, &session, "merged", None);
            }
            status_update = Some(("merged".to_string(), false));
        }
        pr::VERDICT_CHANGES_REQUESTED => {
            action = "Un revisor solicito cambios: re-planifica para generar un parche incremental sobre la misma rama."
                .to_string();
            can_replan = true;
        }
        pr::VERDICT_CI_FAILURE => {
            action = "GitHub Actions fallo: extrae el log y re-planifica para corregir la causa raiz."
                .to_string();
            can_replan = true;
        }
        pr::VERDICT_TECHNICAL_REJECTION => {
            action = "PR cerrado por motivos tecnicos: procede un plan nuevo en una rama limpia (v2)."
                .to_string();
            can_replan = true;
        }
        pr::VERDICT_CONCEPTUAL_REJECTION => {
            action = "Rechazo conceptual: el cambio no se desea en el proyecto. La tarea se cancela sin abrir nuevos PRs."
                .to_string();
            can_replan = false;
            status_update = Some(("rejected".to_string(), false));
            if session.active {
                let feedback = report
                    .comments
                    .last()
                    .map(|comment| comment.body.clone())
                    .unwrap_or_else(|| report.verdict_reason.clone());
                let _ = record_attempt(&repo, &session, "rechazado", Some(&feedback));
            }
        }
        pr::VERDICT_PENDING => {
            action =
                "PR abierto y sin acciones automaticas pendientes: esperando revision humana."
                    .to_string();
        }
        _ => {
            action = if report.found {
                format!("Estado '{}': sin acciones automaticas.", report.state)
            } else {
                "Todavia no existe un Pull Request para esta rama.".to_string()
            };
        }
    }

    state.update_session(|current| {
        if let Some((phase, active)) = status_update {
            current.phase = phase;
            current.active = active;
        }
        if report.found {
            current.pr_url = Some(report.url.clone());
            current.pr_number = Some(report.number);
        }
    });

    if session.active && !report.found {
        can_replan = false;
    }

    Ok(PrCheck {
        action,
        remote_branch_deleted,
        can_replan,
        branch: session.branch.clone(),
        base_branch: session.base_branch.clone(),
        cleanup_performed,
        report,
    })
}

/// Re-planificacion disparada por el estado del PR, con la evidencia del revisor
/// y/o el log de CI como contexto reflexivo.
pub async fn replan_from_pr(reporter: &Reporter, state: &AppState) -> Result<AgentOutcome, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;

    let session = state.session_snapshot();
    if !session.active {
        return Err("No hay una tarea activa sobre la que re-planificar.".to_string());
    }

    let reference = session
        .pr_number
        .map(|number| number.to_string())
        .or_else(|| session.pr_url.clone());
    let report = pr::view(&repo, reference.as_deref())?;

    if !report.found {
        return Err("Todavia no existe un PR para la rama del agente.".to_string());
    }

    match report.verdict.as_str() {
        pr::VERDICT_MERGED => {
            return Err("PR_MERGED: el PR ya fue integrado; no hace falta re-planificar.".to_string())
        }
        pr::VERDICT_CONCEPTUAL_REJECTION => {
            state.update_session(|current| {
                current.active = false;
                current.phase = "rejected".to_string();
            });
            return Err(format!(
                "CONCEPTUAL_REJECTION: el cambio no se desea en el proyecto. {}",
                report.verdict_reason
            ));
        }
        pr::VERDICT_PENDING => {
            reporter.log(
                "warn",
                "El PR no tiene fallos ni cambios solicitados: se re-planifica bajo tu responsabilidad.",
            );
        }
        _ => {}
    }

    let mut evidence = report.summary_text.clone();

    if report.verdict == pr::VERDICT_CI_FAILURE {
        match pr::failed_log(&repo, 9000) {
            Ok(log) if !log.trim().is_empty() => {
                evidence.push_str("\n\nLog de CI fallido (tail):\n```\n");
                evidence.push_str(&log);
                evidence.push_str("\n```\n");
            }
            Ok(_) => {}
            Err(err) => reporter.log( "warn", format!("No se pudo leer el log de CI: {}", err)),
        }
    }

    // Un PR cerrado exige rama limpia (Seccion 10).
    let fresh_branch = report.state == "CLOSED";

    replan_internal(reporter, state, &evidence, fresh_branch).await
}

/// Reinicia la UI/sesion sin tocar el repositorio.
pub fn reset(state: &AppState) {
    state.clear_session();
    state.clear_abort();
}

// ---------------------------------------------------------------------------
// Historial (bitacora persistente)
// ---------------------------------------------------------------------------

pub fn history(state: &AppState) -> Result<Vec<AttemptRecord>, String> {
    let repo = state
        .repo_path()
        .ok_or_else(|| "Selecciona un repositorio local primero.".to_string())?;
    let store = MemoryStore::load(&repo);
    let mut attempts = store.attempts;
    attempts.reverse();
    Ok(attempts)
}
