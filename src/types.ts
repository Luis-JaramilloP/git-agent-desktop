// Tipos compartidos con el backend de Rust (serde con rename_all = "camelCase").

export interface AuthStatus {
  ghInstalled: boolean;
  gitInstalled: boolean;
  ghVersion?: string | null;
  gitVersion?: string | null;
  authenticated: boolean;
  user?: string | null;
  host?: string | null;
  scopes?: string | null;
  tokenKind?: string | null;
  message: string;
}

export interface EnvDoctor {
  os: string;
  arch: string;
  gitVersion?: string | null;
  gitPath: string;
  ghVersion?: string | null;
  ghPath: string;
  ghAuthenticated: boolean;
  configDir: string;
  mockAi: boolean;
  webview2?: string | null;
}

export interface RepoInfo {
  path: string;
  name: string;
  isGitRepo: boolean;
  remoteUrl?: string | null;
  currentBranch?: string | null;
  defaultBranch?: string | null;
  headCommit?: string | null;
  headSubject?: string | null;
  dirty: boolean;
  changedFiles: number;
  gitVersion?: string | null;
}

export interface RepoStatus {
  branch: string;
  defaultBranch: string;
  dirty: boolean;
  changedFiles: string[];
  stagedFiles: number;
  ahead: number;
  behind: number;
}

export interface FileChange {
  filePath: string;
  created: boolean;
  additions: number;
  deletions: number;
  diff: string;
  bytesWritten: number;
}

export interface Usage {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
}

export interface AgentOutcome {
  repoPath: string;
  branch: string;
  baseBranch: string;
  summary: string;
  notes?: string | null;
  changes: FileChange[];
  diff: string;
  commitMessage: string;
  prTitle: string;
  prBody: string;
  attempt: number;
  maxAttempts: number;
  model: string;
  selectedFiles: string[];
  usage: Usage;
  warnings: string[];
  mock: boolean;
  phase: string;
}

export interface RunRequest {
  task: string;
  issue?: number | null;
  model?: string | null;
}

export interface PublishRequest {
  commitMessage?: string | null;
  prTitle?: string | null;
  prBody?: string | null;
  baseBranch?: string | null;
}

export interface PublishResult {
  url: string;
  branch: string;
  baseBranch: string;
  commit?: string | null;
  message: string;
}

export interface AbortResult {
  restored: string[];
  branch?: string | null;
  branchDeleted: boolean;
  message: string;
}

export interface CheckRun {
  name: string;
  status: string;
  conclusion: string;
  url?: string | null;
  failed: boolean;
}

export interface PrComment {
  author: string;
  body: string;
  createdAt: string;
  kind: string;
  state?: string | null;
  path?: string | null;
  line?: number | null;
}

export interface PrReport {
  found: boolean;
  number: number;
  url: string;
  title: string;
  state: string;
  isDraft: boolean;
  reviewDecision?: string | null;
  mergeable?: string | null;
  baseRef: string;
  headRef: string;
  additions: number;
  deletions: number;
  files: string[];
  checks: CheckRun[];
  comments: PrComment[];
  verdict: string;
  verdictReason: string;
  summaryText: string;
}

export interface PrCheck {
  report: PrReport;
  action: string;
  canReplan: boolean;
  branch?: string | null;
  baseBranch?: string | null;
  cleanupPerformed: boolean;
  remoteBranchDeleted: boolean;
}

export interface ProviderPreset {
  id: string;
  label: string;
  baseUrl: string;
  requiresKey: boolean;
  local: boolean;
  suggestedModels: string[];
  notes: string;
  docsUrl: string;
  retired: boolean;
}

export interface AiTest {
  ok: boolean;
  provider: string;
  model: string;
  baseUrl: string;
  latencyMs: number;
  reply: string;
  totalTokens: number;
}

export interface SecretStatus {
  stored: boolean;
  fromEnv: boolean;
  available: boolean;
  path: string;
}
export interface AppConfig {
  provider: string;
  model: string;
  baseUrl: string;
  temperature: number;
  maxTokens: number;
  maxFileBytes: number;
  maxContextChars: number;
  maxFilesInTree: number;
  maxAttempts: number;
  updateUrl: string;
  mockAi: boolean;
}

export interface UpdateInfo {
  currentVersion: string;
  available: boolean;
  version?: string | null;
  notes?: string | null;
  mandatory: boolean;
  size: number;
  source: string;
  configured: boolean;
}

export interface UpdateProgress {
  phase: string;
  percent: number;
  message: string;
}

export interface VersionInfo {
  currentVersion: string;
  channel: string;
  configured: boolean;
  installDir: string;
  requiresElevation: boolean;
}

export interface AttemptRecord {
  attempt: number;
  timestamp: string;
  model: string;
  task: string;
  branch: string;
  summary: string;
  diff: string;
  files: string[];
  outcome: string;
  reviewerFeedback?: string | null;
  prUrl?: string | null;
  usageTotalTokens?: number | null;
}

export interface LogLine {
  level: "info" | "ok" | "warn" | "error" | "agent" | string;
  message: string;
  ts: string;
}
