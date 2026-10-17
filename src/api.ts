// Enlace IPC tipado con el backend de Rust (@tauri-apps/api).
import { invoke } from "@tauri-apps/api/core";

import type {
  AbortResult,
  AgentOutcome,
  AiTest,
  AppConfig,
  AttemptRecord,
  AuthStatus,
  EnvDoctor,
  PrCheck,
  PrReport,
  ProviderPreset,
  PublishRequest,
  PublishResult,
  RepoInfo,
  RepoStatus,
  RunRequest,
  SecretStatus,
  UpdateInfo,
  VersionInfo,
} from "./types";

export const api = {
  // Autenticacion GitHub (solo para git/gh, no para inferencia)
  authStatus: () => invoke<AuthStatus>("auth_status"),
  authLogin: () => invoke<AuthStatus>("auth_login"),
  authLogout: () => invoke<AuthStatus>("auth_logout"),
  envDoctor: () => invoke<EnvDoctor>("env_doctor"),

  // Repositorio y Git
  repoInspect: (path: string) => invoke<RepoInfo>("repo_inspect", { path }),
  repoOpen: (path: string) => invoke<RepoInfo>("repo_open", { path }),
  repoStatus: () => invoke<RepoStatus>("repo_status"),
  gitDiff: () => invoke<string>("git_diff"),
  gitRestore: (paths: string[]) => invoke<string>("git_restore", { paths }),
  gitCleanup: (branch: string, baseBranch: string) =>
    invoke<string>("git_cleanup_after_merge", { branch, baseBranch }),

  // Pull Requests
  prView: (pr?: string | null) => invoke<PrReport>("pr_view", { pr: pr ?? null }),
  prFailedLog: () => invoke<string>("pr_failed_log"),
  prComment: (body: string) => invoke<string>("pr_comment", { body }),
  prClose: (body: string) => invoke<string>("pr_close", { body }),

  // Ciclo agentico
  agentRun: (request: RunRequest) => invoke<AgentOutcome>("agent_run", { request }),
  agentAdjust: (feedback: string) => invoke<AgentOutcome>("agent_adjust", { feedback }),
  agentPublish: (request: PublishRequest) =>
    invoke<PublishResult>("agent_publish", { request }),
  agentAbort: () => invoke<AbortResult>("agent_abort"),
  agentReset: () => invoke<string>("agent_reset"),
  agentCheckPr: () => invoke<PrCheck>("agent_check_pr"),
  agentReplanFromPr: () => invoke<AgentOutcome>("agent_replan_from_pr"),
  agentHistory: () => invoke<AttemptRecord[]>("agent_history"),

  // Configuracion del proveedor de IA
  configGet: () => invoke<AppConfig>("config_get"),
  configSet: (config: AppConfig) => invoke<AppConfig>("config_set", { config }),
  providersList: () => invoke<ProviderPreset[]>("providers_list"),
  aiTest: () => invoke<AiTest>("ai_test"),
  aiModels: () => invoke<string[]>("ai_models"),
  secretStatus: () => invoke<SecretStatus>("secret_status"),
  secretSet: (key: string) => invoke<SecretStatus>("secret_set", { key }),
  secretClear: () => invoke<SecretStatus>("secret_clear"),
  openExternal: (url: string) => invoke<void>("open_external", { url }),

  // Actualizaciones automaticas
  appVersion: () => invoke<VersionInfo>("app_version"),
  updateCheck: () => invoke<UpdateInfo>("update_check"),
  updateApply: () => invoke<void>("update_apply"),
};

/** Normaliza los errores del backend (string) a un mensaje legible. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
}
