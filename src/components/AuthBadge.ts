// Indicador de sesion activa de GitHub (Seccion 3).

import { api, errorMessage } from "../api";
import { clear, create } from "../dom";
import type { AuthStatus } from "../types";

export type LogSink = (level: string, message: string) => void;

export class AuthBadge {
  private root: HTMLElement;
  private log: LogSink;
  private onStatus?: (status: AuthStatus) => void;
  private status: AuthStatus | null = null;
  private busy = false;

  constructor(root: HTMLElement, log: LogSink, onStatus?: (status: AuthStatus) => void) {
    this.root = root;
    this.log = log;
    this.onStatus = onStatus;
  }

  async refresh(): Promise<AuthStatus | null> {
    try {
      this.status = await api.authStatus();
    } catch (error) {
      this.status = null;
      this.log("error", `No se pudo consultar 'gh auth status': ${errorMessage(error)}`);
    }
    this.render();
    if (this.status) this.onStatus?.(this.status);
    return this.status;
  }

  current(): AuthStatus | null {
    return this.status;
  }

  private render(): void {
    clear(this.root);
    const status = this.status;

    if (!status) {
      this.root.className = "auth-badge is-pending";
      this.root.title = "Comprobando la sesion de GitHub CLI";
      this.root.appendChild(create("span", "dot"));
      this.root.appendChild(create("span", undefined, "Comprobando gh..."));
      return;
    }

    let modifier = "is-offline";
    let label = "GitHub desconectado";

    if (!status.ghInstalled) {
      label = "gh no instalado";
    } else if (status.authenticated) {
      modifier = "is-online";
      label = status.user ? `@${status.user}` : "Sesion activa";
    } else if (this.busy) {
      modifier = "is-pending";
      label = "Conectando...";
    }

    this.root.className = `auth-badge ${modifier}`;
    this.root.title = status.message;
    this.root.appendChild(create("span", "dot"));
    this.root.appendChild(create("span", undefined, label));

    if (status.authenticated) {
      const exit = create("button", "link-btn", "salir");
      exit.type = "button";
      exit.addEventListener("click", () => void this.disconnect());
      this.root.appendChild(exit);
      return;
    }

    if (status.ghInstalled) {
      const connect = create("button", "link-btn", this.busy ? "esperando..." : "conectar");
      connect.type = "button";
      connect.disabled = this.busy;
      connect.addEventListener("click", () => void this.connect());
      this.root.appendChild(connect);
    }
  }

  private async connect(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    this.render();
    this.log("info", "Iniciando el flujo OAuth de GitHub CLI (se abrira el navegador)...");

    try {
      this.status = await api.authLogin();
      this.log("ok", this.status.message);
      this.onStatus?.(this.status);
    } catch (error) {
      this.log("error", `No se pudo conectar la cuenta: ${errorMessage(error)}`);
      try {
        this.status = await api.authStatus();
      } catch {
        /* se mantiene el estado previo */
      }
    } finally {
      this.busy = false;
      this.render();
    }
  }

  private async disconnect(): Promise<void> {
    try {
      this.status = await api.authLogout();
      this.log("warn", "Sesion de GitHub CLI cerrada.");
      this.onStatus?.(this.status);
    } catch (error) {
      this.log("error", `No se pudo cerrar la sesion: ${errorMessage(error)}`);
    }
    this.render();
  }
}
