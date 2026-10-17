// Ventana pequeña de actualizacion: muestra la version nueva, permite aplicarla
// y refleja el progreso real (descarga, verificacion, aplicacion de cambios).

import { listen } from "@tauri-apps/api/event";

import { api, errorMessage } from "../api";
import { clear, create } from "../dom";
import type { UpdateInfo, UpdateProgress } from "../types";

import type { LogSink } from "./AuthBadge";

function modalRoot(): HTMLElement {
  const node = document.getElementById("modal-root");
  if (!node) throw new Error("No se encontro #modal-root");
  return node;
}

function humanSize(bytes: number): string {
  if (!bytes) return "tamano desconocido";
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export class UpdateModal {
  static close(): void {
    clear(modalRoot());
  }

  static async open(info: UpdateInfo, log: LogSink): Promise<void> {
    const version = info.version ?? "nueva";

    // Se evita duplicar el modal si ya esta abierto.
    const existing = document.querySelector("#modal-root .modal");
    if (existing) {
      const title = existing.querySelector("h3")?.textContent ?? "";
      if (title.includes("Actualizacion")) return;
    }

    UpdateModal.close();

    const root = modalRoot();
    const backdrop = create("div", "modal-backdrop");
    const modal = create("div", "modal");

    const head = create("div", "modal-head");
    head.appendChild(create("h3", undefined, `Actualizacion disponible · v${version}`));
    const closeBtn = create("button", "icon-btn", "×");
    closeBtn.type = "button";
    closeBtn.title = "Cerrar";
    closeBtn.addEventListener("click", () => UpdateModal.close());
    head.appendChild(closeBtn);
    modal.appendChild(head);

    const body = create("div", "modal-body");
    modal.appendChild(body);

    const foot = create("div", "modal-foot");
    modal.appendChild(foot);

    const hint = create("div", "hint");
    body.appendChild(hint);

    const showAvailable = (): void => {
      clear(body);
      clear(foot);

      const banner = create("div", "status-banner ok");
      banner.textContent = `Tienes la v${info.currentVersion} y esta disponible la v${version} (${humanSize(
        info.size,
      )}).`;
      body.appendChild(banner);

      if (info.notes) {
        body.appendChild(create("div", "attempt-label", "Novedades"));
        body.appendChild(create("p", "attempt-text", info.notes));
      }

      body.appendChild(
        create(
          "p",
          "hint",
          "La actualizacion se aplica sobre la app ya instalada: se descarga, se verifica su firma y se " +
            "reinicia sola. No hace falta volver al instalador.",
        ),
      );

      if (info.mandatory) {
        body.appendChild(create("p", "hint", "Esta actualizacion es obligatoria para seguir usando la app."));
      }

      if (!info.mandatory) {
        const later = create("button", "btn btn-ghost", "Mas tarde");
        later.type = "button";
        later.addEventListener("click", () => UpdateModal.close());
        foot.appendChild(later);
      }

      const apply = create("button", "btn btn-primary", `Actualizar a v${version}`);
      apply.type = "button";
      apply.addEventListener("click", () => void runUpdate());
      foot.appendChild(apply);
    };

    const runUpdate = async (): Promise<void> => {
      clear(body);
      clear(foot);

      const title = create("div", "attempt-label", "Aplicando cambios a la aplicacion instalada");
      body.appendChild(title);

      const track = create("div", "progress-track");
      const fill = create("div", "progress-fill");
      fill.style.width = "2%";
      track.appendChild(fill);
      body.appendChild(track);

      const message = create("p", "attempt-text", "Preparando la actualizacion...");
      body.appendChild(message);

      const note = create(
        "p",
        "hint",
        "La aplicacion se cerrara y se volvera a abrir al terminar. No cierres la ventana del instalador.",
      );
      body.appendChild(note);

      const stopListening = await listen<UpdateProgress>("update://progress", (event) => {
        const progress = event.payload;
        const percent = Math.max(2, Math.min(100, progress.percent));
        fill.style.width = `${percent}%`;
        message.textContent = progress.message;
        log("agent", `[actualizacion] ${progress.message}`);
      });

      try {
        const versionInfo = await api.appVersion();
        if (versionInfo.requiresElevation) {
          note.textContent =
            "Windows pedira permiso de administrador porque la app esta instalada en una carpeta " +
            "protegida. Acepta el aviso para aplicar los cambios.";
        }

        await api.updateApply();
        // Si el proceso no termino, la actualizacion no llego a aplicarse.
        stopListening();
        message.textContent = "El instalador no llego a iniciarse.";
      } catch (error) {
        stopListening();
        const text = errorMessage(error);
        log("error", `Actualizacion fallida: ${text}`);
        clear(body);
        const banner = create("div", "status-banner err");
        banner.textContent = text;
        body.appendChild(banner);
        clear(foot);
        const back = create("button", "btn", "Reintentar");
        back.type = "button";
        back.addEventListener("click", () => void runUpdate());
        foot.appendChild(back);
        const close = create("button", "btn btn-ghost", "Cerrar");
        close.type = "button";
        close.addEventListener("click", () => UpdateModal.close());
        foot.appendChild(close);
      }
    };

    backdrop.appendChild(modal);
    backdrop.addEventListener("click", (event) => {
      if (event.target === backdrop) UpdateModal.close();
    });
    root.appendChild(backdrop);

    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        window.removeEventListener("keydown", onKey);
        UpdateModal.close();
      }
    };
    window.addEventListener("keydown", onKey);

    showAvailable();

    // Recordatorio del canal si no esta configurado.
    if (!info.configured) {
      hint.textContent = "Canal de actualizaciones no configurado.";
    }

    log("info", `Actualizacion ${version} disponible (actual v${info.currentVersion}).`);
  }
}
