// Punto de entrada de la interfaz: enlaza el estado del backend con la UI y
// gobierna el flujo Human-in-the-Loop.

import { listen } from "@tauri-apps/api/event";

import { api, errorMessage } from "./api";
import { AuthBadge } from "./components/AuthBadge";
import { DiffViewer } from "./components/DiffViewer";
import { FeedbackModal } from "./components/FeedbackModal";
import { PromptBox } from "./components/PromptBox";
import { RepoSelector } from "./components/RepoSelector";
import { SettingsModal } from "./components/SettingsModal";
import { UpdateModal } from "./components/UpdateModal";
import { clear, create, el, formatNumber } from "./dom";
import type { AgentOutcome, AppConfig, AttemptRecord, LogLine, ProviderPreset } from "./types";

const logNode = el<HTMLDivElement>("log");
const historyNode = el<HTMLDivElement>("history-view");
const diffNode = el<HTMLDivElement>("diff-viewer");
const metaNode = el<HTMLDivElement>("workspace-meta");
const tabDiff = el<HTMLButtonElement>("tab-diff");
const tabHistory = el<HTMLButtonElement>("tab-history");
const btnApprove = el<HTMLButtonElement>("btn-approve");
const btnAdjust = el<HTMLButtonElement>("btn-adjust");
const btnCheck = el<HTMLButtonElement>("btn-check");
const btnReset = el<HTMLButtonElement>("btn-reset");
const btnAbort = el<HTMLButtonElement>("btn-abort");

const MAX_LOG_LINES = 600;

let activeConfig: AppConfig | null = null;
let activePresets: ProviderPreset[] = [];

let outcome: AgentOutcome | null = null;
let busy = false;
let published = false;

const viewer = new DiffViewer(diffNode);

// ------------------------------- Bitacora ---------------------------------

function appendLog(level: string, message: string, ts?: string): void {
  const line = create("div", `log-line level-${level}`);
  line.appendChild(create("span", "ts", ts ?? new Date().toLocaleTimeString("es-ES")));
  line.appendChild(create("span", "msg", message));
  logNode.appendChild(line);

  while (logNode.childElementCount > MAX_LOG_LINES) {
    logNode.removeChild(logNode.firstElementChild as HTMLElement);
  }
  logNode.scrollTop = logNode.scrollHeight;
}

// ------------------------------- Estado UI --------------------------------

function setMeta(): void {
  clear(metaNode);
  const current = repoSelector.current();

  if (!current) {
    metaNode.appendChild(create("span", "meta-tag", "Sin repositorio"));
  } else {
    metaNode.appendChild(create("span", "meta-tag", current.name));
  }

  if (activeConfig) {
    metaNode.appendChild(
      create("span", "meta-tag", `${activeConfig.provider} · ${activeConfig.model}`),
    );
  }

  if (outcome) {
    metaNode.appendChild(create("span", "meta-tag accent", outcome.branch));
    metaNode.appendChild(
      create("span", "meta-tag", `base ${outcome.baseBranch}`),
    );
    metaNode.appendChild(
      create("span", "meta-tag", `ciclo ${outcome.attempt}/${outcome.maxAttempts}`),
    );
    if (outcome.usage.totalTokens > 0) {
      metaNode.appendChild(
        create("span", "meta-tag", `${formatNumber(outcome.usage.totalTokens)} tokens`),
      );
    }
    if (outcome.mock) {
      metaNode.appendChild(create("span", "meta-tag amber", "modo prueba"));
    }
    if (published) {
      metaNode.appendChild(create("span", "meta-tag green", "PR publicado"));
    }
  } else {
    metaNode.appendChild(create("span", "meta-tag", "Sin propuesta activa"));
  }
}

function refreshActions(): void {
  const hasOutcome = outcome !== null;
  btnApprove.disabled = busy || !hasOutcome;
  btnAdjust.disabled = busy || !hasOutcome;
  btnCheck.disabled = busy || !hasOutcome || !published;
  btnReset.disabled = busy || !hasOutcome;
  btnAbort.disabled = busy || !hasOutcome;
  promptBox.setReady(repoSelector.current() !== null && !busy);
  promptBox.setBusy(busy);
  promptBox.setAbortEnabled(!busy && hasOutcome);
  setMeta();
}

function setBusy(value: boolean): void {
  busy = value;
  refreshActions();
}

function adoptOutcome(next: AgentOutcome): void {
  outcome = next;
  published = false;
  viewer.render(next);
  switchTab("diff");
  refreshActions();

  if (next.warnings.length > 0) {
    for (const warning of next.warnings) appendLog("warn", warning);
  }
}

// ------------------------------- Tabs ------------------------------------

function switchTab(target: "diff" | "history"): void {
  const isDiff = target === "diff";
  tabDiff.classList.toggle("active", isDiff);
  tabHistory.classList.toggle("active", !isDiff);
  diffNode.classList.toggle("hidden", !isDiff);
  historyNode.classList.toggle("hidden", isDiff);
  if (!isDiff) void renderHistory();
}

// ------------------------------- Historial --------------------------------

async function renderHistory(): Promise<void> {
  clear(historyNode);
  historyNode.appendChild(create("div", "attempt-label", "Bitacora de intentos"));

  let attempts: AttemptRecord[] = [];
  try {
    attempts = await api.agentHistory();
  } catch (error) {
    historyNode.appendChild(
      create("div", "placeholder", `No se pudo leer la bitacora: ${errorMessage(error)}`),
    );
    return;
  }

  if (attempts.length === 0) {
    historyNode.appendChild(
      create("div", "placeholder", "Todavia no hay intentos registrados para este repositorio."),
    );
    return;
  }

  for (const attempt of attempts) {
    const card = create("div", "attempt-card");

    const head = create("div", "attempt-head");
    head.appendChild(create("span", "attempt-index", String(attempt.attempt)));
    head.appendChild(create("span", undefined, attempt.summary || attempt.task));
    const badgeTone =
      attempt.outcome === "aprobado" || attempt.outcome === "merged"
        ? "badge ok"
        : attempt.outcome === "rechazado"
          ? "badge err"
          : "badge warn";
    head.appendChild(create("span", badgeTone, attempt.outcome));
    head.appendChild(create("span", "attempt-time", attempt.timestamp));
    card.appendChild(head);

    const body = create("div", "attempt-body");
    body.appendChild(create("div", "attempt-label", "Tarea"));
    body.appendChild(create("p", "attempt-text", attempt.task));
    body.appendChild(
      create(
        "div",
        "hint",
        `Rama ${attempt.branch || "—"} · modelo ${attempt.model || "—"}${
          attempt.usageTotalTokens ? ` · ${formatNumber(attempt.usageTotalTokens)} tokens` : ""
        }`,
      ),
    );

    if (attempt.reviewerFeedback) {
      body.appendChild(create("div", "attempt-label", "Evidencia del revisor"));
      body.appendChild(create("p", "attempt-text", attempt.reviewerFeedback));
    }

    if (attempt.prUrl) {
      body.appendChild(create("div", "hint", attempt.prUrl));
    }

    if (attempt.diff.trim()) {
      const details = create("details");
      const summary = create("summary", "attempt-label", "Diff del intento");
      details.appendChild(summary);
      const pre = create("div", "code-block", attempt.diff.slice(0, 12000));
      details.appendChild(pre);
      body.appendChild(details);
    }

    card.appendChild(body);
    historyNode.appendChild(card);
  }
}

// ------------------------------- Acciones ---------------------------------

async function runAgent(): Promise<void> {
  setBusy(true);
  appendLog("agent", "Iniciando el ciclo agentico (ReAct + HITL)...");

  try {
    const model = promptBox.model;
    const issueValue = el<HTMLInputElement>("issue-input").value.trim();
    const issue = issueValue ? Number.parseInt(issueValue, 10) : null;

    const result = await api.agentRun({
      task: el<HTMLTextAreaElement>("task-input").value.trim(),
      issue: issue !== null && Number.isFinite(issue) ? issue : null,
      model: model || null,
    });

    adoptOutcome(result);
    appendLog("ok", `Propuesta generada en '${result.branch}'. Revisa el diff y aprueba.`);
  } catch (error) {
    const message = errorMessage(error);
    appendLog("error", message);
    FeedbackModal.error("No se pudo ejecutar el agente", message);
    maybeOpenSettings(message);
  } finally {
    setBusy(false);
  }
}

async function approveAndPublish(): Promise<void> {
  if (!outcome) return;

  const confirmed = await FeedbackModal.confirm(
    "Aprobar y abrir Pull Request",
    `Se hara commit en '${outcome.branch}', push a origin y se creara el PR contra '${outcome.baseBranch}'.\n\nCommit: ${outcome.commitMessage}\nTitulo: ${outcome.prTitle}`,
    "Aprobar y publicar",
  );
  if (!confirmed) return;

  setBusy(true);
  try {
    const result = await api.agentPublish({});
    published = true;
    appendLog("ok", `PR publicado: ${result.url}`);
    FeedbackModal.publishResult(result, (url) => {
      void api.openExternal(url).catch((error: unknown) => {
        appendLog("error", `No se pudo abrir el navegador: ${errorMessage(error)}`);
      });
    });
  } catch (error) {
    appendLog("error", errorMessage(error));
    FeedbackModal.error("No se pudo publicar el Pull Request", errorMessage(error));
  } finally {
    setBusy(false);
  }
}

async function requestAdjustment(): Promise<void> {
  const feedback = await FeedbackModal.prompt(
    "Solicitar ajuste",
    "Explica que debe cambiar el agente (se usara como evidencia de auto-correccion)",
    "Ej: El manejo de errores debe usar try/catch y registrar el fallo con el logger existente.",
  );
  if (!feedback) return;

  setBusy(true);
  appendLog("agent", "Re-planificando con la evidencia del ajuste...");
  try {
    const result = await api.agentAdjust(feedback);
    adoptOutcome(result);
    appendLog("ok", `Nuevo diff listo en '${result.branch}' (ciclo ${result.attempt}).`);
  } catch (error) {
    const message = errorMessage(error);
    appendLog("error", message);
    FeedbackModal.error("No se pudo re-planificar", message);
    maybeOpenSettings(message);
  } finally {
    setBusy(false);
  }
}

async function checkPr(): Promise<void> {
  setBusy(true);
  try {
    const check = await api.agentCheckPr();
    FeedbackModal.prCheck(
      check,
      (url) => {
        void api.openExternal(url).catch((error: unknown) => {
          appendLog("error", `No se pudo abrir el navegador: ${errorMessage(error)}`);
        });
      },
      () => {
        void replanFromPr();
      },
    );
  } catch (error) {
    appendLog("error", errorMessage(error));
    FeedbackModal.error("No se pudo consultar el PR", errorMessage(error));
  } finally {
    setBusy(false);
  }
}

async function replanFromPr(): Promise<void> {
  setBusy(true);
  appendLog("agent", "Re-planificando a partir del estado del PR...");
  try {
    const result = await api.agentReplanFromPr();
    adoptOutcome(result);
    appendLog("ok", `Nueva propuesta en '${result.branch}'.`);
  } catch (error) {
    const message = errorMessage(error);
    appendLog("error", message);
    FeedbackModal.error("No se pudo re-planificar", message);
    maybeOpenSettings(message);
  } finally {
    setBusy(false);
  }
}

async function abortSession(): Promise<void> {
  const confirmed = await FeedbackModal.confirm(
    "Abortar la sesion",
    "Se restaurara el contenido previo de los archivos que toco el agente y se eliminara la rama local si no tiene commits.",
    "Abortar y restaurar",
  );
  if (!confirmed) return;

  setBusy(true);
  try {
    const result = await api.agentAbort();
    appendLog("warn", result.message);
    if (result.branchDeleted && result.branch) {
      appendLog("info", `Rama local '${result.branch}' eliminada.`);
    }
    outcome = null;
    published = false;
    viewer.clear();
    refreshActions();
  } catch (error) {
    appendLog("error", errorMessage(error));
    FeedbackModal.error("No se pudo abortar", errorMessage(error));
  } finally {
    setBusy(false);
  }
}

async function resetSession(): Promise<void> {
  try {
    await api.agentReset();
  } catch (error) {
    appendLog("warn", `No se pudo reiniciar la sesion: ${errorMessage(error)}`);
  }
  outcome = null;
  published = false;
  viewer.clear();
  refreshActions();
  appendLog("info", "Sesion reiniciada. Listo para una nueva tarea.");
}

// ------------------------------- Arranque ---------------------------------

const authBadge = new AuthBadge(el("auth-badge"), appendLog, (status) => {
  promptBox.setReady(repoSelector.current() !== null && !busy);
  if (status.authenticated && status.user) {
    appendLog("ok", `Sesion de GitHub activa como ${status.user}.`);
  }
});

const repoSelector = new RepoSelector(
  el("repo-body"),
  el("repo-chip"),
  appendLog,
  () => {
    refreshActions();
  },
);

const promptBox = new PromptBox(appendLog, () => void runAgent(), () => void abortSession());

btnApprove.addEventListener("click", () => void approveAndPublish());
btnAdjust.addEventListener("click", () => void requestAdjustment());
btnCheck.addEventListener("click", () => void checkPr());
btnReset.addEventListener("click", () => void resetSession());
tabDiff.addEventListener("click", () => switchTab("diff"));
tabHistory.addEventListener("click", () => switchTab("history"));
el("btn-clear-log").addEventListener("click", () => clear(logNode));
el("btn-settings").addEventListener("click", () => openSettings());

promptBox.onChange((model) => {
  if (!activeConfig || !model || model === activeConfig.model) return;
  const updated: AppConfig = { ...activeConfig, model };
  activeConfig = updated;
  setMeta();
  void api
    .configSet(updated)
    .then((saved) => {
      activeConfig = saved;
      setMeta();
    })
    .catch((error: unknown) => {
      appendLog("warn", `No se pudo guardar el modelo: ${errorMessage(error)}`);
    });
});

/** Si el fallo es de configuracion del proveedor, abre Ajustes directamente. */
function maybeOpenSettings(message: string): void {
  if (/falta la clave|abre ajustes|no tiene url base|no tiene modelo|retirado|fuera de servicio/i.test(message)) {
    window.setTimeout(() => openSettings(), 250);
  }
}

function openSettings(): void {
  void SettingsModal.open({
    log: appendLog,
    onSaved: (config) => {
      activeConfig = config;
      applyProvider(config);
      refreshActions();
    },
    onCheckUpdates: () => void checkUpdates(true),
  });
}

/** Consulta el canal de actualizaciones (al arrancar y a peticion del usuario). */
async function checkUpdates(manual: boolean): Promise<void> {
  try {
    const info = await api.updateCheck();
    if (info.available) {
      void UpdateModal.open(info, appendLog);
      return;
    }
    if (manual) {
      FeedbackModal.info(
        "Actualizaciones",
        info.configured
          ? `Estas al dia (v${info.currentVersion}).`
          : `No hay canal de actualizaciones configurado. Version actual: v${info.currentVersion}.`,
      );
    }
  } catch (error) {
    const message = errorMessage(error);
    appendLog("warn", `No se pudo comprobar actualizaciones: ${message}`);
    if (manual) {
      FeedbackModal.error("Actualizaciones", message);
    }
  }
}

async function bootstrap(): Promise<void> {
  appendLog(
    "info",
    "Git Agent Desktop 2.0 - motor determinista en Rust + inferencia compatible con OpenAI.",
  );

  // Proveedor de IA, catalogo y estado de la clave
  try {
    const [presets, config, secret] = await Promise.all([
      api.providersList(),
      api.configGet(),
      api.secretStatus(),
    ]);

    activePresets = presets;
    activeConfig = config;
    applyProvider(config);

    const preset = presets.find((item) => item.id === config.provider);
    const baseUrl = config.baseUrl || preset?.baseUrl || "";

    if (config.mockAi) {
      appendLog("warn", "Modo de pruebas activo (GAD_MOCK_AI=1): no se llamara al proveedor.");
    } else if (preset?.retired) {
      appendLog(
        "error",
        `El proveedor "${preset.label}" esta fuera de servicio. Abre Ajustes (⚙) y elige otro ` +
          "(Ollama local, Gemini, Groq u OpenRouter).",
      );
    } else if (preset?.requiresKey && !secret.stored && !secret.fromEnv) {
      appendLog(
        "warn",
        `Falta la clave de ${preset.label}. Abre Ajustes (⚙) para pegarla y probar la conexion.`,
      );
    } else {
      appendLog("info", `Proveedor: ${preset?.label ?? config.provider} · modelo ${config.model}`);
    }
    if (baseUrl) {
      appendLog("info", `Endpoint de inferencia: ${baseUrl}`);
    }
  } catch (error) {
    appendLog("warn", `No se pudo cargar la configuracion: ${errorMessage(error)}`);
  }

  // Diagnostico del entorno
  try {
    const doctor = await api.envDoctor();
    appendLog(
      "info",
      `git: ${doctor.gitVersion ?? "no detectado"} · gh: ${doctor.ghVersion ?? "no detectado"}`,
    );
    if (!doctor.gitVersion) {
      appendLog("error", "Git no esta disponible en este equipo.");
    }
    updateEnvironmentLine(doctor.gitVersion ?? null, doctor.ghVersion ?? null);
  } catch (error) {
    appendLog("warn", `Diagnostico no disponible: ${errorMessage(error)}`);
    updateEnvironmentLine(null, null);
  }

  await authBadge.refresh();
  repoSelector.render();
  refreshActions();
  viewer.clear();

  try {
    await listen<LogLine>("agent://log", (event) => {
      appendLog(event.payload.level, event.payload.message, event.payload.ts);
    });
  } catch (error) {
    appendLog("warn", `No se pudo suscribir a los eventos del backend: ${errorMessage(error)}`);
  }

  // Comprobacion silenciosa de actualizaciones al arrancar.
  window.setTimeout(() => void checkUpdates(false), 3000);
}

function applyProvider(config: AppConfig): void {
  const preset = activePresets.find((item) => item.id === config.provider);
  promptBox.setModels(preset?.suggestedModels ?? [], config.model);
  promptBox.setReady(repoSelector.current() !== null);
  setMeta();
}

/** Linea de diagnostico del encabezado (versiones y geometria del webview). */
function updateEnvironmentLine(gitVersion: string | null, ghVersion: string | null): void {
  const subtitle = document.querySelector(".brand-text p");
  if (!subtitle) return;

  const short = (version: string | null, label: string) => {
    if (!version) return `${label} —`;
    const match = version.match(/(\d+\.\d+(?:\.\d+)?)/);
    return `${label} ${match ? match[1] : version}`;
  };

  const width = Math.round(window.innerWidth);
  const height = Math.round(window.innerHeight);
  const dpr = window.devicePixelRatio ? window.devicePixelRatio.toFixed(2) : "1";

  subtitle.textContent = `${short(gitVersion, "git")} · ${short(
    ghVersion,
    "gh",
  )} · ${width}x${height} @${dpr}x`;
  subtitle.setAttribute("title", "Versiones detectadas y geometria del webview");
}

window.addEventListener("resize", () => {
  const subtitle = document.querySelector(".brand-text p");
  if (subtitle) {
    const width = Math.round(window.innerWidth);
    const height = Math.round(window.innerHeight);
    const dpr = window.devicePixelRatio ? window.devicePixelRatio.toFixed(2) : "1";
    const base = subtitle.textContent ?? "";
    const parts = base.split(" · ");
    if (parts.length >= 3) {
      parts[parts.length - 1] = `${width}x${height} @${dpr}x`;
      subtitle.textContent = parts.join(" · ");
    }
  }
});

void bootstrap();
