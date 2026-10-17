// Panel de Ajustes: proveedor de IA, URL base, modelo y clave cifrada.
// La clave se envia al backend, que la cifra con DPAPI; nunca se guarda en
// texto plano ni se vuelve a mostrar completa.

import { api, errorMessage } from "../api";
import { clear, create } from "../dom";
import type { AppConfig, ProviderPreset, SecretStatus } from "../types";

import type { LogSink } from "./AuthBadge";

interface SettingsOptions {
  log: LogSink;
  onSaved: (config: AppConfig) => void;
  onCheckUpdates?: () => void;
}

function modalRoot(): HTMLElement {
  const node = document.getElementById("modal-root");
  if (!node) throw new Error("No se encontro #modal-root");
  return node;
}

export class SettingsModal {
  static close(): void {
    clear(modalRoot());
  }

  static async open(options: SettingsOptions): Promise<void> {
    let config: AppConfig;
    let presets: ProviderPreset[];
    let secret: SecretStatus;

    try {
      [config, presets, secret] = await Promise.all([
        api.configGet(),
        api.providersList(),
        api.secretStatus(),
      ]);
    } catch (error) {
      options.log("error", `No se pudieron cargar los ajustes: ${errorMessage(error)}`);
      return;
    }

    // ------------------------------- Estructura -------------------------------
    SettingsModal.close();
    const root = modalRoot();

    const backdrop = create("div", "modal-backdrop");
    const modal = create("div", "modal");

    const head = create("div", "modal-head");
    head.appendChild(create("h3", undefined, "Proveedor de IA"));
    const closeBtn = create("button", "icon-btn", "×");
    closeBtn.type = "button";
    closeBtn.title = "Cerrar";
    closeBtn.addEventListener("click", () => SettingsModal.close());
    head.appendChild(closeBtn);
    modal.appendChild(head);

    const body = create("div", "modal-body");

    // Proveedor
    const providerField = create("label", "field");
    providerField.appendChild(create("span", "field-label", "Proveedor"));
    const providerSelect = create("select");
    for (const preset of presets) {
      const option = create("option", undefined, preset.label);
      option.value = preset.id;
      providerSelect.appendChild(option);
    }
    providerSelect.value = config.provider;
    providerField.appendChild(providerSelect);
    body.appendChild(providerField);

    const notes = create("div", "status-banner");
    body.appendChild(notes);

    const docsLink = create("button", "link-btn", "obtener clave / documentacion");
    docsLink.type = "button";
    const docsRow = create("div", "hint");
    docsRow.appendChild(docsLink);
    body.appendChild(docsRow);

    // URL base + modelo
    const grid = create("div", "settings-grid");

    const baseField = create("label", "field");
    baseField.appendChild(create("span", "field-label", "URL base (termina en /v1)"));
    const baseInput = create("input");
    baseInput.type = "text";
    baseInput.spellcheck = false;
    baseInput.value = config.baseUrl;
    baseInput.placeholder = "http://127.0.0.1:11434/v1";
    baseField.appendChild(baseInput);
    grid.appendChild(baseField);

    const modelField = create("label", "field");
    modelField.appendChild(create("span", "field-label", "Modelo"));
    const modelInput = create("input");
    modelInput.type = "text";
    modelInput.spellcheck = false;
    modelInput.value = config.model;
    modelField.appendChild(modelInput);
    grid.appendChild(modelField);

    body.appendChild(grid);

    // Lista real de modelos del proveedor (evita ids obsoletos en los presets).
    const modelDatalist = create("datalist");
    modelDatalist.id = "provider-models";
    modelInput.setAttribute("list", "provider-models");
    body.appendChild(modelDatalist);

    const modelRow = create("div", "btn-row");
    const listBtn = create("button", "btn btn-mini", "Ver modelos disponibles");
    listBtn.type = "button";
    modelRow.appendChild(listBtn);
    body.appendChild(modelRow);

    const modelHint = create("div", "hint", "");
    body.appendChild(modelHint);

    listBtn.addEventListener("click", () => {
      void (async () => {
        listBtn.disabled = true;
        modelHint.textContent = "Consultando los modelos del proveedor...";
        try {
          const ids = await api.aiModels();
          clear(modelDatalist);
          for (const id of ids) {
            const option = create("option");
            option.value = id;
            modelDatalist.appendChild(option);
          }
          if (ids.length === 0) {
            modelHint.textContent = "El proveedor no devolvio ningun modelo.";
          } else {
            modelHint.textContent = `${ids.length} modelos disponibles en tu cuenta. Escribe o elige uno en el campo Modelo.`;
            if (!ids.includes(modelInput.value)) {
              modelInput.value = ids[0];
            }
          }
        } catch (error) {
          modelHint.textContent = errorMessage(error);
        } finally {
          listBtn.disabled = false;
        }
      })();
    });

    const suggestions = create("div", "suggestion-row");
    body.appendChild(suggestions);

    // Clave
    const keyField = create("label", "field");
    const keyLabel = create("span", "field-label", "Clave API (se guarda cifrada con DPAPI)");
    keyField.appendChild(keyLabel);

    const keyRow = create("div", "password-row");
    const keyInput = create("input");
    keyInput.type = "password";
    keyInput.placeholder = "pega la clave y pulsa Guardar";
    keyInput.autocomplete = "off";
    keyInput.spellcheck = false;
    keyRow.appendChild(keyInput);

    const revealBtn = create("button", "btn btn-ghost btn-mini", "ver");
    revealBtn.type = "button";
    revealBtn.addEventListener("click", () => {
      const showing = keyInput.type === "text";
      keyInput.type = showing ? "password" : "text";
      revealBtn.textContent = showing ? "ver" : "ocultar";
    });
    keyRow.appendChild(revealBtn);
    keyField.appendChild(keyRow);
    body.appendChild(keyField);

    const secretState = create("div", "hint");
    body.appendChild(secretState);

    // --------------------------- Actualizaciones ---------------------------
    body.appendChild(create("div", "attempt-label", "Actualizaciones"));
    const versionLine = create("div", "hint", "Version...");
    body.appendChild(versionLine);

    const updateField = create("label", "field");
    updateField.appendChild(
      create("span", "field-label", "Canal (URL de latest.json o carpeta)"),
    );
    const updateInput = create("input");
    updateInput.type = "text";
    updateInput.spellcheck = false;
    updateInput.value = config.updateUrl;
    updateInput.placeholder = "vacio = canal local %APPDATA%\\GitAgentDesktop\\updates";
    updateField.appendChild(updateInput);
    body.appendChild(updateField);

    const updateRow = create("div", "btn-row");
    const checkBtn = create("button", "btn btn-mini", "Buscar actualizaciones");
    checkBtn.type = "button";
    checkBtn.addEventListener("click", () => options.onCheckUpdates?.());
    updateRow.appendChild(checkBtn);
    body.appendChild(updateRow);

    void api
      .appVersion()
      .then((info) => {
        versionLine.textContent = `Version instalada v${info.currentVersion} · ${
          info.requiresElevation ? "requiere permisos de administrador para actualizar" : "actualizacion sin permisos"
        }`;
        versionLine.title = `Canal: ${info.channel}\nInstalada en: ${info.installDir}`;
        if (!info.configured && !updateInput.value) {
          versionLine.textContent += " · canal local no publicado";
        }
      })
      .catch(() => {
        versionLine.textContent = "No se pudo leer la version instalada.";
      });

    const result = create("div", "status-banner hidden");
    body.appendChild(result);

    // ------------------------------- Estado ---------------------------------
    const currentPreset = (): ProviderPreset | undefined =>
      presets.find((preset) => preset.id === providerSelect.value);

    let lastDefaultBase = config.baseUrl;

    const refreshProviderUi = () => {
      const preset = currentPreset();
      const requiresKey = preset?.requiresKey ?? true;

      notes.textContent = preset
        ? `${preset.notes}${preset.retired ? "  ⚠ Proveedor fuera de servicio." : ""}`
        : "";
      notes.className = `status-banner${preset?.retired ? " err" : ""}`;

      const docsUrl = preset?.docsUrl ?? "";
      docsLink.classList.toggle("hidden", !docsUrl);
      docsLink.onclick = () => {
        if (docsUrl) void api.openExternal(docsUrl).catch(() => undefined);
      };

      // Al cambiar de proveedor se propone su URL por defecto, salvo que el
      // usuario haya escrito una propia.
      const untouched = !baseInput.value.trim() || baseInput.value.trim() === lastDefaultBase.trim();
      if (untouched && preset) {
        baseInput.value = preset.baseUrl;
        lastDefaultBase = preset.baseUrl;
      }

      keyInput.disabled = !requiresKey;
      keyInput.placeholder = requiresKey
        ? "pega la clave y pulsa Guardar"
        : "no necesaria para proveedores locales";

      if (preset && preset.suggestedModels.length > 0 && !preset.suggestedModels.includes(modelInput.value)) {
        modelInput.value = preset.suggestedModels[0];
      }

      clear(suggestions);
      for (const suggestion of preset?.suggestedModels ?? []) {
        const chip = create("button", "chip", suggestion);
        chip.type = "button";
        chip.addEventListener("click", () => {
          modelInput.value = suggestion;
        });
        suggestions.appendChild(chip);
      }

      const secretOk = secret.stored || secret.fromEnv;
      secretState.textContent = secretOk
        ? secret.fromEnv
          ? "Clave tomada de la variable de entorno GAD_API_KEY."
          : `Clave guardada cifrada en ${secret.path}`
        : requiresKey
          ? "Sin clave guardada todavia."
          : "Este proveedor no requiere clave.";
    };

    providerSelect.addEventListener("change", refreshProviderUi);
    refreshProviderUi();

    // ------------------------------- Guardar --------------------------------
    const showMessage = (tone: "ok" | "err" | "warn", text: string) => {
      result.className = `status-banner ${tone}`;
      result.textContent = text;
      result.classList.remove("hidden");
    };

    const persist = async (thenTest: boolean): Promise<void> => {
      const updated: AppConfig = {
        ...config,
        provider: providerSelect.value,
        baseUrl: baseInput.value.trim(),
        model: modelInput.value.trim(),
        updateUrl: updateInput.value.trim(),
      };

      try {
        const saved = await api.configSet(updated);
        config = saved;

        if (keyInput.value.trim()) {
          secret = await api.secretSet(keyInput.value.trim());
          keyInput.value = "";
          options.log("ok", "Clave del proveedor guardada cifrada con DPAPI.");
        }

        options.onSaved(saved);
        refreshProviderUi();

        if (!thenTest) {
          SettingsModal.close();
          return;
        }

        showMessage("warn", "Probando conexion con el proveedor...");
        try {
          const test = await api.aiTest();
          showMessage(
            "ok",
            `Conexion correcta con ${test.model} en ${test.latencyMs} ms. ` +
              `Respuesta: "${test.reply}"${test.totalTokens ? ` (${test.totalTokens} tokens)` : ""}`,
          );
          options.log("ok", `Proveedor OK: ${test.provider} / ${test.model} (${test.latencyMs} ms)`);
        } catch (error) {
          showMessage("err", errorMessage(error));
          options.log("error", errorMessage(error));
        }
      } catch (error) {
        showMessage("err", errorMessage(error));
      }
    };

    const clearKey = async (): Promise<void> => {
      try {
        secret = await api.secretClear();
        refreshProviderUi();
        showMessage("warn", "Clave borrada del almacen cifrado.");
      } catch (error) {
        showMessage("err", errorMessage(error));
      }
    };

    // ------------------------------- Pie ------------------------------------
    const foot = create("div", "modal-foot");

    const clearBtn = create("button", "btn btn-ghost", "Borrar clave");
    clearBtn.type = "button";
    clearBtn.addEventListener("click", () => void clearKey());
    foot.appendChild(clearBtn);

    const cancel = create("button", "btn btn-ghost", "Cerrar");
    cancel.type = "button";
    cancel.addEventListener("click", () => SettingsModal.close());
    foot.appendChild(cancel);

    const testBtn = create("button", "btn", "Guardar y probar");
    testBtn.type = "button";
    testBtn.addEventListener("click", () => void persist(true));
    foot.appendChild(testBtn);

    const saveBtn = create("button", "btn btn-primary", "Guardar");
    saveBtn.type = "button";
    saveBtn.addEventListener("click", () => void persist(false));
    foot.appendChild(saveBtn);

    modal.appendChild(body);
    modal.appendChild(foot);
    backdrop.appendChild(modal);
    backdrop.addEventListener("click", (event) => {
      if (event.target === backdrop) SettingsModal.close();
    });
    root.appendChild(backdrop);

    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        window.removeEventListener("keydown", onKey);
        SettingsModal.close();
      }
    };
    window.addEventListener("keydown", onKey);

    window.setTimeout(() => keyInput.focus(), 40);
  }
}
