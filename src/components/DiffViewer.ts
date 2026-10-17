// Visor de diferencias con aprobacion Human-in-the-Loop (Seccion 9, paso 3).
// Parsea diffs unificados (tanto `git diff` como los generados por patch.rs) y
// los renderiza con numeros de linea, colores y contadores.

import { clear, create, formatNumber } from "../dom";
import type { AgentOutcome } from "../types";

type LineKind = "add" | "del" | "ctx" | "note";

interface ParsedLine {
  kind: LineKind;
  text: string;
  oldNo: number | null;
  newNo: number | null;
}

interface ParsedHunk {
  range: string;
  context: string;
  lines: ParsedLine[];
}

interface ParsedFile {
  path: string;
  created: boolean;
  additions: number;
  deletions: number;
  hunks: ParsedHunk[];
}

const HUNK_RE = /^@@ -(\d+)(?:,(\d+))?\s+\+(\d+)(?:,(\d+))?\s+@@\s?(.*)$/;

function normalizePath(value: string): string {
  let path = value.trim();
  if (path.startsWith('"') && path.endsWith('"')) {
    path = path.slice(1, -1);
  }
  if (path === "/dev/null") return path;
  if (path.startsWith("a/") || path.startsWith("b/")) {
    path = path.slice(2);
  }
  return path;
}

/** Convierte un diff unificado en una estructura renderizable. */
export function parseUnifiedDiff(raw: string): ParsedFile[] {
  const files: ParsedFile[] = [];
  let current: ParsedFile | null = null;
  let hunk: ParsedHunk | null = null;
  let pendingOld = "";
  let oldNo = 0;
  let newNo = 0;

  const pushFile = (path: string): ParsedFile => {
    const file: ParsedFile = {
      path: path || "(archivo desconocido)",
      created: false,
      additions: 0,
      deletions: 0,
      hunks: [],
    };
    files.push(file);
    return file;
  };

  for (const line of raw.split(/\r?\n/)) {
    if (line.startsWith("diff --git ")) {
      const match = /^diff --git (.+?) (.+)$/.exec(line);
      current = pushFile(match ? normalizePath(match[2]) : "");
      hunk = null;
      continue;
    }

    if (line.startsWith("new file mode")) {
      if (current) current.created = true;
      continue;
    }

    if (
      line.startsWith("index ") ||
      line.startsWith("old mode") ||
      line.startsWith("new mode") ||
      line.startsWith("similarity index") ||
      line.startsWith("dissimilarity index") ||
      line.startsWith("rename ") ||
      line.startsWith("deleted file mode") ||
      line.startsWith("\\ No newline")
    ) {
      continue;
    }

    if (line.startsWith("--- ")) {
      pendingOld = normalizePath(line.slice(4));
      if (!current) {
        current = pushFile(pendingOld);
        hunk = null;
      }
      continue;
    }

    if (line.startsWith("+++ ")) {
      const next = normalizePath(line.slice(4));
      if (current) {
        current.path = next === "/dev/null" ? pendingOld : next;
      } else {
        current = pushFile(next === "/dev/null" ? pendingOld : next);
      }
      continue;
    }

    const hunkMatch = HUNK_RE.exec(line);
    if (hunkMatch) {
      if (!current) current = pushFile("");
      oldNo = Number(hunkMatch[1]);
      newNo = Number(hunkMatch[3]);
      hunk = {
        range: `-${hunkMatch[1]}${hunkMatch[2] ? `,${hunkMatch[2]}` : ""} +${hunkMatch[3]}${hunkMatch[4] ? `,${hunkMatch[4]}` : ""}`,
        context: (hunkMatch[5] ?? "").trim(),
        lines: [],
      };
      current.hunks.push(hunk);
      continue;
    }

    // Encabezados de modo binario
    if (line.startsWith("Binary files ")) {
      if (!current) current = pushFile("");
      current.hunks.push({
        range: "binario",
        context: line,
        lines: [{ kind: "note", text: "Archivo binario: sin diff textual.", oldNo: null, newNo: null }],
      });
      hunk = null;
      continue;
    }

    if (hunk) {
      if (line.startsWith("+")) {
        current!.additions += 1;
        hunk.lines.push({ kind: "add", text: line.slice(1), oldNo: null, newNo });
        newNo += 1;
      } else if (line.startsWith("-")) {
        current!.deletions += 1;
        hunk.lines.push({ kind: "del", text: line.slice(1), oldNo, newNo: null });
        oldNo += 1;
      } else if (line.startsWith(" ") || line === "") {
        hunk.lines.push({
          kind: "ctx",
          text: line.slice(1),
          oldNo,
          newNo,
        });
        oldNo += 1;
        newNo += 1;
      } else {
        hunk.lines.push({ kind: "note", text: line, oldNo: null, newNo: null });
      }
    }
  }

  return files.filter((file) => file.path !== "" || file.hunks.length > 0);
}

function splitPath(path: string): { dir: string; base: string } {
  const index = path.lastIndexOf("/");
  if (index < 0) return { dir: "", base: path };
  return { dir: path.slice(0, index + 1), base: path.slice(index + 1) };
}

const MAX_RENDER_LINES = 4000;

export class DiffViewer {
  private root: HTMLElement;
  private outcome: AgentOutcome | null = null;

  constructor(root: HTMLElement) {
    this.root = root;
  }

  clear(): void {
    this.outcome = null;
    this.renderEmpty();
  }

  current(): AgentOutcome | null {
    return this.outcome;
  }

  render(outcome: AgentOutcome | null): void {
    this.outcome = outcome;
    clear(this.root);

    if (!outcome) {
      this.renderEmpty();
      return;
    }

    const parsed = parseUnifiedDiff(outcome.diff);
    const totalAdd = outcome.changes.reduce((sum, change) => sum + change.additions, 0);
    const totalDel = outcome.changes.reduce((sum, change) => sum + change.deletions, 0);

    // ----------------------------- Barra superior -----------------------------
    const toolbar = create("div", "diff-toolbar");
    toolbar.appendChild(
      create("span", "diff-stat", `${formatNumber(outcome.changes.length)} archivo(s)`),
    );
    toolbar.appendChild(create("span", "diff-stat add", `+${formatNumber(totalAdd)}`));
    toolbar.appendChild(create("span", "diff-stat del", `-${formatNumber(totalDel)}`));

    const branchTag = create("span", "meta-tag accent", outcome.branch);
    toolbar.appendChild(branchTag);

    if (outcome.mock) {
      toolbar.appendChild(create("span", "meta-tag amber", "modo prueba"));
    }

    if (outcome.usage.totalTokens > 0) {
      toolbar.appendChild(
        create(
          "span",
          "meta-tag",
          `${formatNumber(outcome.usage.totalTokens)} tokens`,
        ),
      );
    }

    toolbar.appendChild(
      create(
        "span",
        "meta-tag",
        `ciclo ${outcome.attempt}/${outcome.maxAttempts}`,
      ),
    );

    this.root.appendChild(toolbar);

    // ------------------------------- Resumen ---------------------------------
    const summaryCard = create("div", "attempt-card");
    const summaryBody = create("div", "attempt-body");
    summaryBody.appendChild(create("div", "attempt-label", "Resumen del agente"));
    summaryBody.appendChild(create("p", "attempt-text", outcome.summary));
    if (outcome.notes) {
      summaryBody.appendChild(create("div", "attempt-label", "Notas"));
      summaryBody.appendChild(create("p", "attempt-text", outcome.notes));
    }
    if (outcome.warnings.length > 0) {
      const warnLabel = create("div", "attempt-label", "Avisos");
      summaryBody.appendChild(warnLabel);
      for (const warning of outcome.warnings) {
        summaryBody.appendChild(create("p", "attempt-text", `• ${warning}`));
      }
    }
    summaryCard.appendChild(summaryBody);
    this.root.appendChild(summaryCard);

    // ------------------------------ Archivos ---------------------------------
    if (parsed.length === 0) {
      const fallback = create("div", "placeholder", "El diff no contiene hunks textuales.");
      this.root.appendChild(fallback);
    }

    let renderedLines = 0;
    for (const file of parsed) {
      const card = this.buildFileCard(file, renderedLines);
      renderedLines += this.countLines(file);
      this.root.appendChild(card);
    }
  }

  private countLines(file: ParsedFile): number {
    return file.hunks.reduce((sum, hunk) => sum + hunk.lines.length, 0);
  }

  private buildFileCard(file: ParsedFile, alreadyRendered: number): HTMLElement {
    const card = create("div", "diff-file");

    const head = create("div", "diff-file-head");
    head.appendChild(
      create("span", `diff-file-icon${file.created ? " created" : ""}`, file.created ? "NEW" : "MOD"),
    );

    const nameWrap = create("span", "diff-file-name");
    const { dir, base } = splitPath(file.path);
    if (dir) nameWrap.appendChild(create("span", "dir", dir));
    nameWrap.appendChild(create("span", "base", base));
    head.appendChild(nameWrap);

    const counts = create("span", "diff-file-counts");
    counts.appendChild(create("span", "diff-count add", `+${file.additions}`));
    counts.appendChild(create("span", "diff-count del", `-${file.deletions}`));
    head.appendChild(counts);

    card.appendChild(head);

    const body = create("div", "diff-file-body");
    let budget = MAX_RENDER_LINES - alreadyRendered;

    for (const hunk of file.hunks) {
      if (budget <= 0) break;

      const hunkHead = create("div", "diff-hunk-head");
      hunkHead.appendChild(create("span", "hunk-range", hunk.range));
      if (hunk.context) hunkHead.appendChild(create("span", "hunk-context", hunk.context));
      body.appendChild(hunkHead);

      for (const line of hunk.lines) {
        if (budget <= 0) break;
        budget -= 1;
        body.appendChild(this.buildLine(line));
      }
    }

    if (budget <= 0) {
      body.appendChild(
        create(
          "div",
          "diff-line collapsed",
          "… diff truncado en el visor (se mantiene completo en el archivo y en el PR)",
        ),
      );
    }

    card.appendChild(body);
    return card;
  }

  private buildLine(line: ParsedLine): HTMLElement {
    const row = create("div", `diff-line ${line.kind === "add" ? "add" : line.kind === "del" ? "del" : line.kind === "ctx" ? "ctx" : "note"}`);

    row.appendChild(
      create("div", "ln", line.oldNo === null ? "" : String(line.oldNo)),
    );
    row.appendChild(
      create("div", "ln new", line.newNo === null ? "" : String(line.newNo)),
    );

    const sign = line.kind === "add" ? "+" : line.kind === "del" ? "-" : " ";
    row.appendChild(create("div", "sign", sign));
    row.appendChild(create("div", "code", line.text));

    return row;
  }

  private renderEmpty(): void {
    clear(this.root);
    const empty = create("div", "empty-state");
    empty.appendChild(create("div", "empty-icon", "◇"));
    empty.appendChild(create("h3", undefined, "Sin cambios pendientes"));
    empty.appendChild(
      create(
        "p",
        undefined,
        "Selecciona un repositorio, describe la tarea y ejecuta el agente para ver aqui el diff propuesto.",
      ),
    );
    this.root.appendChild(empty);
  }
}
