// Panel modal de estado del PR y del ciclo de auto-correccion (Seccion 10).
// Todo el contenido dinamico se inserta con textContent (nunca innerHTML) para
// no interpretar codigo del repositorio o de GitHub.

import { clear, create } from "../dom";
import type { PrCheck, PrReport, PublishResult } from "../types";

interface ModalAction {
  label: string;
  variant?: "primary" | "danger" | "ghost";
  onClick?: () => void;
}

const VERDICT_LABELS: Record<string, { label: string; tone: "ok" | "warn" | "err" | "info" }> = {
  MERGED: { label: "Integrado (merged)", tone: "ok" },
  CHANGES_REQUESTED: { label: "Cambios solicitados", tone: "warn" },
  CI_FAILURE: { label: "CI/CD fallido", tone: "err" },
  CONCEPTUAL_REJECTION: { label: "Rechazo conceptual", tone: "err" },
  TECHNICAL_REJECTION: { label: "Rechazo tecnico", tone: "warn" },
  PENDING: { label: "Pendiente de revision", tone: "info" },
  NONE: { label: "Sin Pull Request", tone: "info" },
};

function verdictMeta(verdict: string): { label: string; tone: "ok" | "warn" | "err" | "info" } {
  return VERDICT_LABELS[verdict] ?? { label: verdict, tone: "info" };
}

function bannerClass(tone: "ok" | "warn" | "err" | "info"): string {
  if (tone === "ok") return "status-banner ok";
  if (tone === "warn") return "status-banner warn";
  if (tone === "err") return "status-banner err";
  return "status-banner";
}

export class FeedbackModal {
  private static root(): HTMLElement {
    const node = document.getElementById("modal-root");
    if (!node) throw new Error("No se encontro #modal-root");
    return node;
  }

  static close(): void {
    clear(FeedbackModal.root());
  }

  private static mount(
    title: string,
    body: HTMLElement,
    actions: ModalAction[],
    dismissible = true,
  ): HTMLElement {
    FeedbackModal.close();
    const root = FeedbackModal.root();

    const backdrop = create("div", "modal-backdrop");
    const modal = create("div", "modal");

    const head = create("div", "modal-head");
    head.appendChild(create("h3", undefined, title));

    const closeBtn = create("button", "icon-btn", "×");
    closeBtn.type = "button";
    closeBtn.title = "Cerrar";
    closeBtn.addEventListener("click", () => FeedbackModal.close());
    head.appendChild(closeBtn);
    modal.appendChild(head);

    modal.appendChild(body);

    const foot = create("div", "modal-foot");
    for (const action of actions) {
      const button = create("button", `btn${action.variant ? ` btn-${action.variant}` : ""}`, action.label);
      button.type = "button";
      button.addEventListener("click", () => {
        action.onClick?.();
      });
      foot.appendChild(button);
    }
    modal.appendChild(foot);

    backdrop.appendChild(modal);
    if (dismissible) {
      backdrop.addEventListener("click", (event) => {
        if (event.target === backdrop) FeedbackModal.close();
      });
    }
    root.appendChild(backdrop);

    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && dismissible) {
        window.removeEventListener("keydown", onKey);
        FeedbackModal.close();
      }
    };
    window.addEventListener("keydown", onKey);

    return modal;
  }

  static error(title: string, message: string): void {
    const body = create("div", "modal-body");
    const banner = create("div", "status-banner err");
    banner.textContent = message;
    body.appendChild(banner);

    FeedbackModal.mount(title, body, [
      { label: "Entendido", variant: "primary", onClick: () => FeedbackModal.close() },
    ]);
  }

  static info(title: string, message: string, extra?: HTMLElement): void {
    const body = create("div", "modal-body");
    const banner = create("div", "status-banner");
    banner.textContent = message;
    body.appendChild(banner);
    if (extra) body.appendChild(extra);

    FeedbackModal.mount(title, body, [
      { label: "Cerrar", variant: "primary", onClick: () => FeedbackModal.close() },
    ]);
  }

  /** Solicita texto libre (feedback / ajuste). Devuelve null si se cancela. */
  static prompt(
    title: string,
    label: string,
    placeholder: string,
    initial = "",
  ): Promise<string | null> {
    return new Promise((resolve) => {
      let settled = false;
      const settle = (value: string | null) => {
        if (settled) return;
        settled = true;
        FeedbackModal.close();
        resolve(value);
      };

      const body = create("div", "modal-body");
      const labelNode = create("div", "attempt-label", label);
      const textarea = create("textarea");
      textarea.value = initial;
      textarea.rows = 6;
      textarea.placeholder = placeholder;
      body.appendChild(labelNode);
      body.appendChild(textarea);

      const modal = FeedbackModal.mount(
        title,
        body,
        [
          { label: "Cancelar", variant: "ghost", onClick: () => settle(null) },
          {
            label: "Enviar",
            variant: "primary",
            onClick: () => settle(textarea.value.trim() ? textarea.value.trim() : null),
          },
        ],
        false,
      );

      // Cierre por la X: se interpreta como cancelacion.
      const closeBtn = modal.querySelector(".icon-btn");
      closeBtn?.addEventListener("click", () => settle(null), { capture: true });

      textarea.addEventListener("keydown", (event) => {
        if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
          event.preventDefault();
          settle(textarea.value.trim() ? textarea.value.trim() : null);
        }
      });

      window.setTimeout(() => textarea.focus(), 30);
    });
  }

  static confirm(title: string, message: string, confirmLabel: string): Promise<boolean> {
    return new Promise((resolve) => {
      let settled = false;
      const settle = (value: boolean) => {
        if (settled) return;
        settled = true;
        FeedbackModal.close();
        resolve(value);
      };

      const body = create("div", "modal-body");
      const banner = create("div", "status-banner");
      banner.textContent = message;
      body.appendChild(banner);

      FeedbackModal.mount(
        title,
        body,
        [
          { label: "Cancelar", variant: "ghost", onClick: () => settle(false) },
          { label: confirmLabel, variant: "primary", onClick: () => settle(true) },
        ],
        false,
      );
    });
  }

  static publishResult(result: PublishResult, onOpen: (url: string) => void): void {
    const body = create("div", "modal-body");

    const banner = create("div", "status-banner ok");
    banner.textContent = result.message;
    body.appendChild(banner);

    const list = create("dl", "kv");
    const addRow = (label: string, value: string) => {
      list.appendChild(create("dt", undefined, label));
      list.appendChild(create("dd", undefined, value));
    };
    addRow("Rama", result.branch);
    addRow("Base", result.baseBranch);
    if (result.commit) addRow("Commit", result.commit);
    body.appendChild(list);

    const link = create("div", "code-block", result.url);
    body.appendChild(link);

    FeedbackModal.mount("Pull Request publicado", body, [
      { label: "Cerrar", variant: "ghost", onClick: () => FeedbackModal.close() },
      {
        label: "Abrir en GitHub",
        variant: "primary",
        onClick: () => {
          onOpen(result.url);
          FeedbackModal.close();
        },
      },
    ]);
  }

  static report(
    report: PrReport,
    onOpen: (url: string) => void,
    onReplan?: () => void,
  ): void {
    const meta = verdictMeta(report.verdict);
    const body = create("div", "modal-body");

    const banner = create("div", bannerClass(meta.tone));
    banner.textContent = `${meta.label} — ${report.verdictReason}`;
    body.appendChild(banner);

    if (report.found) {
      const list = create("dl", "kv");
      const addRow = (label: string, value: string) => {
        list.appendChild(create("dt", undefined, label));
        list.appendChild(create("dd", undefined, value));
      };
      addRow("PR", `#${report.number} · ${report.state}${report.isDraft ? " (draft)" : ""}`);
      addRow("Titulo", report.title);
      addRow("Ramas", `${report.headRef} → ${report.baseRef}`);
      addRow("Cambios", `+${report.additions} / -${report.deletions}`);
      if (report.reviewDecision) addRow("Revision", report.reviewDecision);
      if (report.mergeable) addRow("Mergeable", report.mergeable);
      body.appendChild(list);
    }

    const failed = report.checks.filter((check) => check.failed);
    if (failed.length > 0) {
      body.appendChild(create("div", "attempt-label", "Checks fallidos"));
      const list = create("div", "comment-list");
      for (const check of failed) {
        const item = create("div", "comment");
        item.appendChild(create("div", "comment-author", `${check.name}`));
        item.appendChild(
          create("div", "comment-body", `${check.status} / ${check.conclusion}`),
        );
        list.appendChild(item);
      }
      body.appendChild(list);
    }

    const comments = report.comments.filter((comment) => comment.body.trim().length > 0).slice(-8);
    if (comments.length > 0) {
      body.appendChild(create("div", "attempt-label", "Comentarios y revisiones"));
      const list = create("div", "comment-list");
      for (const comment of comments) {
        const item = create("div", "comment");
        const header =
          comment.kind === "review"
            ? `${comment.author} · revision${comment.state ? ` (${comment.state})` : ""}`
            : `${comment.author} · comentario`;
        item.appendChild(create("div", "comment-author", header));
        item.appendChild(create("div", "comment-body", comment.body));
        if (comment.path) {
          item.appendChild(create("div", "hint", comment.path));
        }
        list.appendChild(item);
      }
      body.appendChild(list);
    }

    const actions: ModalAction[] = [
      { label: "Cerrar", variant: "ghost", onClick: () => FeedbackModal.close() },
    ];

    if (report.found && report.url) {
      actions.push({
        label: "Abrir en GitHub",
        onClick: () => {
          onOpen(report.url);
          FeedbackModal.close();
        },
      });
    }

    if (onReplan) {
      actions.push({
        label: "Re-planificar",
        variant: "primary",
        onClick: () => {
          FeedbackModal.close();
          onReplan();
        },
      });
    }

    FeedbackModal.mount("Estado del Pull Request", body, actions);
  }

  static prCheck(check: PrCheck, onOpen: (url: string) => void, onReplan: () => void): void {
    FeedbackModal.report(check.report, onOpen, check.canReplan ? onReplan : undefined);

    // Anade la recomendacion de accion al cuerpo ya montado.
    const body = document.querySelector("#modal-root .modal-body");
    if (body instanceof HTMLElement) {
      const action = create("div", "status-banner");
      action.textContent = check.action;
      body.appendChild(action);
    }
  }
}
