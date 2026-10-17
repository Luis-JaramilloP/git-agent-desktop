// Selector de la carpeta del repositorio local y ficha de estado.

import { open } from "@tauri-apps/plugin-dialog";

import { api, errorMessage } from "../api";
import { clear, create } from "../dom";
import type { RepoInfo, RepoStatus } from "../types";

import type { LogSink } from "./AuthBadge";

export class RepoSelector {
  private body: HTMLElement;
  private chip: HTMLElement;
  private log: LogSink;
  private onChange: (info: RepoInfo | null) => void;
  private info: RepoInfo | null = null;
  private status: RepoStatus | null = null;

  constructor(
    body: HTMLElement,
    chip: HTMLElement,
    log: LogSink,
    onChange: (info: RepoInfo | null) => void,
  ) {
    this.body = body;
    this.chip = chip;
    this.log = log;
    this.onChange = onChange;
  }

  current(): RepoInfo | null {
    return this.info;
  }

  async pick(): Promise<void> {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Selecciona la carpeta del repositorio local",
      });

      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path) return;

      const info = await api.repoOpen(path);
      this.info = info;
      this.log("ok", `Repositorio abierto: ${info.path}`);
      if (info.dirty) {
        this.log(
          "warn",
          `El repositorio tiene ${info.changedFiles} archivo(s) modificados sin confirmar.`,
        );
      }
      await this.refreshStatus();
      this.render();
      this.onChange(info);
    } catch (error) {
      this.log("error", `No se pudo abrir el repositorio: ${errorMessage(error)}`);
    }
  }

  async refreshStatus(): Promise<void> {
    if (!this.info) return;
    try {
      this.status = await api.repoStatus();
    } catch (error) {
      this.log("warn", `No se pudo leer 'git status': ${errorMessage(error)}`);
    }
  }

  setInfo(info: RepoInfo | null): void {
    this.info = info;
    this.render();
    this.onChange(info);
  }

  render(): void {
    clear(this.body);
    clear(this.chip);

    const info = this.info;

    if (!info) {
      this.chip.classList.remove("has-repo");
      this.chip.textContent = "Sin repositorio seleccionado";

      const placeholder = create(
        "div",
        "placeholder",
        "Elige la carpeta de un repositorio Git local.",
      );
      this.body.appendChild(placeholder);

      const button = create("button", "btn btn-primary", "Seleccionar carpeta");
      button.type = "button";
      button.addEventListener("click", () => void this.pick());
      this.body.appendChild(button);
      return;
    }

    this.chip.classList.add("has-repo");
    this.chip.textContent = `${info.name} · ${info.currentBranch ?? "sin rama"}`;

    const card = create("div", "repo-card");
    const nameRow = create("div", "repo-name");
    nameRow.appendChild(create("span", undefined, info.name));
    if (info.dirty) {
      nameRow.appendChild(create("span", "badge warn", `${info.changedFiles} sin confirmar`));
    }
    card.appendChild(nameRow);

    card.appendChild(create("div", "repo-path", info.path));

    const list = create("dl", "kv");
    const addRow = (label: string, value: string) => {
      list.appendChild(create("dt", undefined, label));
      list.appendChild(create("dd", undefined, value));
    };

    addRow("Rama", this.status?.branch ?? info.currentBranch ?? "—");
    addRow("Rama base", info.defaultBranch ?? "—");
    addRow("Remoto", info.remoteUrl ?? "(sin origin)");
    addRow("HEAD", info.headCommit ? `${info.headCommit}` : "—");
    if (this.status) {
      addRow("Sincronia", `↑${this.status.ahead} ↓${this.status.behind}`);
    }
    card.appendChild(list);

    if (info.headSubject) {
      card.appendChild(create("div", "hint", info.headSubject));
    }

    const actions = create("div", "btn-row");
    const change = create("button", "btn btn-ghost btn-mini", "Cambiar");
    change.type = "button";
    change.addEventListener("click", () => void this.pick());
    actions.appendChild(change);

    const refresh = create("button", "btn btn-ghost btn-mini", "Actualizar estado");
    refresh.type = "button";
    refresh.addEventListener("click", () => {
      void (async () => {
        await this.refreshStatus();
        this.render();
      })();
    });
    actions.appendChild(refresh);
    card.appendChild(actions);

    this.body.appendChild(card);
  }
}
