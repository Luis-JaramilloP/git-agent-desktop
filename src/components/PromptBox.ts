// Entrada de tarea, selector de modelo y disparo del agente (Seccion 9, paso 1).

import { el } from "../dom";
import { create } from "../dom";
import type { RunRequest } from "../types";

import type { LogSink } from "./AuthBadge";

export class PromptBox {
  private taskInput: HTMLTextAreaElement;
  private modelSelect: HTMLSelectElement;
  private issueInput: HTMLInputElement;
  private runBtn: HTMLButtonElement;
  private abortBtn: HTMLButtonElement;
  private spinner: HTMLElement;
  private log: LogSink;
  private onRun: (request: RunRequest) => void;
  private onAbort: () => void;
  private busy = false;
  private ready = false;

  constructor(log: LogSink, onRun: (request: RunRequest) => void, onAbort: () => void) {
    this.log = log;
    this.onRun = onRun;
    this.onAbort = onAbort;

    this.taskInput = el<HTMLTextAreaElement>("task-input");
    this.modelSelect = el<HTMLSelectElement>("model-select");
    this.issueInput = el<HTMLInputElement>("issue-input");
    this.runBtn = el<HTMLButtonElement>("btn-run");
    this.abortBtn = el<HTMLButtonElement>("btn-abort");

    const spinner = this.runBtn.querySelector(".spinner");
    this.spinner = spinner instanceof HTMLElement ? spinner : create("span", "spinner hidden");

    this.runBtn.addEventListener("click", () => this.submit());
    this.abortBtn.addEventListener("click", () => this.onAbort());

    this.taskInput.addEventListener("keydown", (event) => {
      if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
        event.preventDefault();
        this.submit();
      }
    });

    this.syncButtons();
  }

  /** Rellena el selector con los modelos sugeridos del proveedor activo. */
  setModels(models: string[], selected: string): void {
    this.modelSelect.replaceChildren();

    const unique = [...new Set([...models, selected].filter((id) => id.trim().length > 0))];

    for (const id of unique) {
      const option = create("option", undefined, id);
      option.value = id;
      this.modelSelect.appendChild(option);
    }

    if (unique.length === 0) {
      const option = create("option", undefined, "(configura un modelo en Ajustes)");
      option.value = "";
      this.modelSelect.appendChild(option);
    }

    this.modelSelect.value = selected || unique[0] || "";
  }

  onChange(handler: (model: string) => void): void {
    this.modelSelect.addEventListener("change", () => handler(this.modelSelect.value));
  }

  get model(): string {
    return this.modelSelect.value;
  }

  setBusy(busy: boolean): void {
    this.busy = busy;
    this.spinner.classList.toggle("hidden", !busy);
    this.syncButtons();
  }

  setReady(ready: boolean): void {
    this.ready = ready;
    this.syncButtons();
  }

  setAbortEnabled(enabled: boolean): void {
    this.abortBtn.disabled = !enabled;
  }

  private syncButtons(): void {
    this.runBtn.disabled = this.busy || !this.ready;
    this.taskInput.disabled = this.busy;
    this.modelSelect.disabled = this.busy;
    this.issueInput.disabled = this.busy;
  }

  private submit(): void {
    if (this.busy) return;

    if (!this.ready) {
      this.log("warn", "Selecciona primero un repositorio Git local.");
      return;
    }

    const task = this.taskInput.value.trim();
    if (!task) {
      this.log("warn", "Describe la tarea que debe resolver el agente.");
      this.taskInput.focus();
      return;
    }

    const issueRaw = this.issueInput.value.trim();
    const issue = issueRaw ? Number.parseInt(issueRaw, 10) : null;

    this.onRun({
      task,
      issue: issue !== null && Number.isFinite(issue) ? issue : null,
      model: this.model || null,
    });
  }

  setTask(text: string): void {
    this.taskInput.value = text;
  }
}
