import "./styles.css";

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";

import {
  getAudioCapabilities,
  getConfig,
  getConfigPath,
  exportConfig,
  importConfig,
  isDesktop,
  listAudioSessions,
  listRunningProcesses,
  saveConfig,
  setApplicationVolume,
} from "./api";
import { bindShortcutCapture } from "./shortcut";
import type {
  AppConfig,
  AudioCapabilities,
  AudioDirection,
  AudioSession,
  Preset,
} from "./types";

type Tab = "presets" | "mixer" | "settings";
type ToastTone = "neutral" | "success" | "error";
type RunningApplication = {
  key: string;
  label: string;
};

const INTERFACE_SCALE = 1.25;

const rootElement = document.querySelector<HTMLElement>("#app");
if (!rootElement) throw new Error("Missing #app root");
const root: HTMLElement = rootElement;

document.documentElement.dataset.runtime = isDesktop ? "desktop" : "browser";

const escapeHtml = (value: unknown): string =>
  String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");

const errorMessage = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

const BACKEND_INITIALIZING_MESSAGE = "duckd backend is still initializing";
const BACKEND_STARTUP_ATTEMPTS = 50;
const BACKEND_STARTUP_RETRY_MS = 50;

const loadInitialBackendState = async (): Promise<
  [AppConfig, AudioCapabilities, string]
> => {
  let lastError: unknown;
  for (let attempt = 0; attempt < BACKEND_STARTUP_ATTEMPTS; attempt += 1) {
    try {
      return await Promise.all([
        getConfig(),
        getAudioCapabilities(),
        getConfigPath(),
      ]);
    } catch (error) {
      if (!errorMessage(error).includes(BACKEND_INITIALIZING_MESSAGE)) throw error;
      lastError = error;
      await new Promise<void>((resolve) =>
        window.setTimeout(resolve, BACKEND_STARTUP_RETRY_MS),
      );
    }
  }
  throw lastError ?? new Error("duckd backend did not finish initializing");
};

const cloneConfig = (config: AppConfig): AppConfig =>
  JSON.parse(JSON.stringify(config)) as AppConfig;

const sessionAppKey = (session: AudioSession): string =>
  session.process_name?.trim() || session.app_name;

const normalizedAppKey = (value: string): string => {
  const filename = value.trim().split(/[\\/]/).pop()?.trim().toLocaleLowerCase() ?? "";
  return filename.endsWith(".exe") ? filename.slice(0, -4) : filename;
};

const runningApplications = (
  processNames: string[],
  sessions: AudioSession[],
): RunningApplication[] => {
  const unique = new Map<string, RunningApplication>();
  processNames.forEach((key) => {
    const normalized = normalizedAppKey(key);
    if (normalized && !unique.has(normalized)) unique.set(normalized, { key, label: key });
  });
  sessions.forEach((session) => {
    const key = sessionAppKey(session);
    const normalized = normalizedAppKey(key);
    if (!normalized) return;
    const existing = unique.get(normalized);
    unique.set(normalized, {
      key: existing?.key ?? key,
      label: session.app_name === key ? key : `${session.app_name} · ${existing?.key ?? key}`,
    });
  });
  return [...unique.values()].sort((left, right) => left.label.localeCompare(right.label));
};

const applyInterfaceScale = async (): Promise<void> => {
  if (isDesktop) await getCurrentWebview().setZoom(INTERFACE_SCALE);
};

const clampedVolume = (volume: number): number =>
  Math.max(0, Math.min(100, Math.round(volume)));

function volumeRows(sessions: AudioSession[], compact = false): string {
  if (sessions.length === 0) {
    return `<div class="empty-panel ${compact ? "empty-panel--compact" : ""}">
      <span class="empty-panel__mark">∅</span>
      <p>No active ${compact ? "output" : "audio"} streams.</p>
    </div>`;
  }

  return `<div class="mixer-list ${compact ? "mixer-list--compact" : ""}">
    ${sessions
      .map((session) => {
        const volume = clampedVolume(session.volume_percent);
        const process = session.process_name
          ? `<span class="stream-process">${escapeHtml(session.process_name)}</span>`
          : "";
        return `<article class="mixer-row" data-session-id="${escapeHtml(session.id)}">
          <div class="stream-meta">
            <span class="stream-icon">${escapeHtml(session.app_name.slice(0, 1).toUpperCase() || "?")}</span>
            <span class="stream-copy">
              <strong>${escapeHtml(session.app_name)}</strong>
              ${process}
            </span>
            ${session.muted ? '<span class="muted-badge">muted</span>' : ""}
          </div>
          <div class="volume-control">
            <input
              class="volume-slider"
              type="range"
              min="0"
              max="100"
              value="${volume}"
              style="--volume: ${volume}%"
              data-app="${escapeHtml(sessionAppKey(session))}"
              data-direction="${session.direction}"
              aria-label="${escapeHtml(session.app_name)} volume"
              ${session.volume_writable ? "" : "disabled"}
            />
            <output class="volume-value">${volume}%</output>
          </div>
        </article>`;
      })
      .join("")}
  </div>`;
}

function wireVolumeSliders(container: ParentNode, onError: (message: string) => void): void {
  container.querySelectorAll<HTMLInputElement>(".volume-slider").forEach((slider) => {
    const output = slider.parentElement?.querySelector<HTMLOutputElement>(".volume-value");
    slider.addEventListener("input", () => {
      slider.style.setProperty("--volume", `${slider.value}%`);
      if (output) output.textContent = `${slider.value}%`;
    });
    slider.addEventListener("change", () => {
      const app = slider.dataset.app;
      const direction = slider.dataset.direction as AudioDirection | undefined;
      if (!app || !direction) return;
      slider.disabled = true;
      void setApplicationVolume(app, direction, Number(slider.value))
        .catch((error: unknown) => onError(errorMessage(error)))
        .finally(() => {
          slider.disabled = false;
        });
    });
  });
}

class MainApplication {
  private config: AppConfig | null = null;
  private capabilities: AudioCapabilities | null = null;
  private configPath = "";
  private activeTab: Tab = "presets";
  private mixerDirection: AudioDirection = "output";
  private mixerSessions: AudioSession[] = [];
  private mixerPending = false;

  async mount(): Promise<void> {
    this.renderShell();
    if (!isDesktop) {
      this.showToast("Open this interface through the duckd desktop app.", "neutral");
      return;
    }

    try {
      await applyInterfaceScale().catch((error: unknown) => {
        console.warn("Could not apply interface scale", error);
      });
      [this.config, this.capabilities, this.configPath] =
        await loadInitialBackendState();
      this.renderActiveTab();
      this.updateResidentStatus();
    } catch (error) {
      this.showToast(errorMessage(error), "error");
    }

    window.setInterval(() => {
      if (this.activeTab === "mixer") void this.refreshMixer();
    }, 1800);
  }

  private renderShell(): void {
    root.innerHTML = `<div class="app-shell">
      <aside class="sidebar" aria-label="Primary navigation">
        <div class="brand"><span class="brand-mark">d:</span><span>duckd</span></div>
        <nav class="nav-list">
          ${this.navButton("presets", "01", "Presets")}
          ${this.navButton("mixer", "02", "Mixer")}
          ${this.navButton("settings", "03", "Settings")}
        </nav>
        <div class="runtime-status"><span class="status-dot"></span><span id="engine-status">connecting</span></div>
      </aside>
      <main class="workspace">
        <div id="workspace-content" class="workspace-content">
          <div class="loading-state"><span></span><span></span><span></span></div>
        </div>
        <footer class="status-bar">
          <span>duckd v0.1.0</span>
          <span id="footer-status">audio engine starting…</span>
        </footer>
      </main>
      <div id="toast" class="toast" role="status" aria-live="polite"></div>
      <div id="modal-root"></div>
    </div>`;

    root.querySelectorAll<HTMLButtonElement>("[data-tab]").forEach((button) => {
      button.addEventListener("click", () => {
        this.activeTab = button.dataset.tab as Tab;
        this.renderActiveTab();
        if (this.activeTab === "mixer") void this.refreshMixer(true);
      });
    });
  }

  private navButton(tab: Tab, index: string, label: string): string {
    return `<button class="nav-item ${this.activeTab === tab ? "nav-item--active" : ""}" type="button" data-tab="${tab}">
      <span class="nav-index">${index}</span><span>${label}</span>
    </button>`;
  }

  private updateNavigation(): void {
    root.querySelectorAll<HTMLButtonElement>("[data-tab]").forEach((button) => {
      const active = button.dataset.tab === this.activeTab;
      button.classList.toggle("nav-item--active", active);
      if (active) button.setAttribute("aria-current", "page");
      else button.removeAttribute("aria-current");
    });
  }

  private updateResidentStatus(): void {
    const engine = root.querySelector<HTMLElement>("#engine-status");
    const footer = root.querySelector<HTMLElement>("#footer-status");
    if (engine) engine.textContent = "resident / listening";
    if (footer && this.config) {
      footer.textContent = `${this.config.presets.length} preset${this.config.presets.length === 1 ? "" : "s"} registered`;
    }
  }

  private renderActiveTab(): void {
    this.updateNavigation();
    if (!this.config || !this.capabilities) return;
    if (this.activeTab === "presets") this.renderPresets();
    if (this.activeTab === "mixer") this.renderMixer();
    if (this.activeTab === "settings") this.renderSettings();
  }

  private content(): HTMLElement {
    const content = root.querySelector<HTMLElement>("#workspace-content");
    if (!content) throw new Error("Missing workspace content");
    return content;
  }

  private pageHeader(eyebrow: string, title: string, actions = ""): string {
    return `<header class="workspace-header">
      <div><p class="eyebrow">${eyebrow}</p><h1>${title}</h1></div>
      <div class="header-actions">${actions}</div>
    </header>`;
  }

  private renderPresets(): void {
    if (!this.config) return;
    const cards = this.config.presets.length
      ? this.config.presets
          .map(
            (preset, index) => `<article class="preset-card">
              <header class="preset-card__header">
                <div><span class="preset-index">preset ${String(index + 1).padStart(2, "0")}</span><h2>${escapeHtml(preset.name)}</h2></div>
                <kbd>${escapeHtml(preset.hotkey)}</kbd>
              </header>
              <div class="target-list">
                ${preset.default_volume == null ? "" : `<div class="target-row target-row--default"><span>all other active streams</span><strong>${preset.default_volume}%</strong></div>`}
                ${
                  preset.targets.length
                    ? preset.targets
                        .map(
                          (target) => `<div class="target-row"><span>${escapeHtml(target.app)}</span><strong>${target.volume}%</strong></div>`,
                        )
                        .join("")
                    : '<div class="target-row target-row--empty">No named overrides configured</div>'
                }
              </div>
              <footer class="preset-actions">
                <button class="button button--quiet" type="button" data-edit-preset="${index}">edit</button>
                <button class="button button--danger" type="button" data-delete-preset="${index}">delete</button>
              </footer>
            </article>`,
          )
          .join("")
      : `<div class="empty-panel"><span class="empty-panel__mark">+</span><h2>No presets yet</h2><p>Add one to bind application volumes to a global shortcut.</p></div>`;

    const content = this.content();
    content.innerHTML = `${this.pageHeader(
      "global volume profiles",
      "Presets",
      '<button class="button button--primary" id="add-preset" type="button">+ new preset</button>',
    )}<section class="page-body"><div class="preset-grid">${cards}</div></section>`;

    content.querySelector<HTMLButtonElement>("#add-preset")?.addEventListener("click", () =>
      void this.openPresetEditor(),
    );
    content.querySelectorAll<HTMLButtonElement>("[data-edit-preset]").forEach((button) => {
      button.addEventListener("click", () => void this.openPresetEditor(Number(button.dataset.editPreset)));
    });
    content.querySelectorAll<HTMLButtonElement>("[data-delete-preset]").forEach((button) => {
      button.addEventListener("click", () => void this.deletePreset(Number(button.dataset.deletePreset)));
    });
  }

  private async openPresetEditor(index?: number): Promise<void> {
    if (!this.config) return;
    const existing = index === undefined ? null : this.config.presets[index];
    const draft: Preset = existing
      ? JSON.parse(JSON.stringify(existing))
      : { name: "", hotkey: "", default_volume: null, targets: [{ app: "", volume: 100 }] };
    const modalRoot = root.querySelector<HTMLElement>("#modal-root");
    if (!modalRoot) return;

    modalRoot.innerHTML = `<div class="modal-backdrop" data-close-modal>
      <section class="modal" role="dialog" aria-modal="true" aria-labelledby="preset-editor-title">
        <header class="modal__header">
          <div><p class="eyebrow">preset editor</p><h2 id="preset-editor-title">${existing ? "Edit preset" : "New preset"}</h2></div>
          <button class="icon-button" type="button" data-close-modal aria-label="Close">×</button>
        </header>
        <form id="preset-form" class="form-stack">
          <label class="field"><span>Name</span><input name="name" maxlength="64" value="${escapeHtml(draft.name)}" placeholder="e.g. Focus" required /></label>
          <label class="field"><span>Global hotkey</span>
            <button class="hotkey-capture" type="button" id="preset-hotkey"><span data-hotkey-value></span><small>click, then press keys</small></button>
          </label>
          <div class="field-group">
            <div class="preset-default">
              <label class="toggle-row toggle-row--compact">
                <span><strong>All other active streams</strong><small>Set a baseline first; named applications below override it.</small></span>
                <input id="default-volume-enabled" type="checkbox" ${draft.default_volume == null ? "" : "checked"}/><i></i>
              </label>
              <label class="field field--volume preset-default__volume"><span class="sr-only">Default stream volume</span><input id="default-volume" type="number" min="0" max="100" value="${draft.default_volume ?? 60}" ${draft.default_volume == null ? "disabled" : ""} required /><span>%</span></label>
            </div>
            <div class="field-group__header">
              <span>Application targets</span>
              <span class="field-group__actions"><button class="text-button" id="refresh-apps" type="button">↻ running apps</button><button class="text-button" id="add-target" type="button">+ add app</button></span>
            </div>
            <div id="target-editor"></div>
            <datalist id="running-app-options"></datalist>
            <p class="field-hint" id="running-app-status">Looking for desktop and audio applications…</p>
            <p class="field-hint">Missing an app? Play audio in it and refresh, or find its executable in System Monitor / Task Manager’s Details tab and type that name above.</p>
          </div>
          <p class="form-error" id="preset-form-error"></p>
          <footer class="modal__actions"><button class="button button--quiet" type="button" data-close-modal>cancel</button><button class="button button--primary" type="submit">save preset</button></footer>
        </form>
      </section>
    </div>`;

    let hotkey = draft.hotkey;
    let defaultVolume = draft.default_volume ?? 60;
    let availableApplications: RunningApplication[] = [];
    const capture = modalRoot.querySelector<HTMLButtonElement>("#preset-hotkey");
    if (capture) bindShortcutCapture(capture, hotkey, (value) => (hotkey = value));

    const defaultEnabled = modalRoot.querySelector<HTMLInputElement>("#default-volume-enabled");
    const defaultInput = modalRoot.querySelector<HTMLInputElement>("#default-volume");
    const syncDefaultVolume = (): void => {
      const enabled = defaultEnabled?.checked ?? false;
      if (defaultInput) defaultInput.disabled = !enabled;
      draft.default_volume = enabled ? defaultVolume : null;
    };
    defaultEnabled?.addEventListener("change", syncDefaultVolume);
    defaultInput?.addEventListener("input", () => {
      defaultVolume = Number(defaultInput.value);
      syncDefaultVolume();
    });

    const renderTargets = (): void => {
      const editor = modalRoot.querySelector<HTMLElement>("#target-editor");
      if (!editor) return;
      editor.innerHTML = draft.targets
        .map(
          (target, targetIndex) => `<div class="target-editor-row">
            <label class="field"><span class="sr-only">Application</span><input data-target-app="${targetIndex}" list="running-app-options" autocomplete="off" value="${escapeHtml(target.app)}" placeholder="select or type an executable" required /></label>
            <label class="field field--volume"><span class="sr-only">Volume</span><input data-target-volume="${targetIndex}" type="number" min="0" max="100" value="${target.volume}" required /><span>%</span></label>
            <button class="icon-button icon-button--danger" type="button" data-remove-target="${targetIndex}" aria-label="Remove target">×</button>
          </div>`,
        )
        .join("");
      editor.querySelectorAll<HTMLInputElement>("[data-target-app]").forEach((input) => {
        input.addEventListener("input", () => {
          draft.targets[Number(input.dataset.targetApp)].app = input.value;
        });
      });
      editor.querySelectorAll<HTMLInputElement>("[data-target-volume]").forEach((input) => {
        input.addEventListener("input", () => {
          draft.targets[Number(input.dataset.targetVolume)].volume = Number(input.value);
        });
      });
      editor.querySelectorAll<HTMLButtonElement>("[data-remove-target]").forEach((button) => {
        button.addEventListener("click", () => {
          draft.targets.splice(Number(button.dataset.removeTarget), 1);
          renderTargets();
        });
      });
    };
    renderTargets();

    const refreshApplications = async (): Promise<void> => {
      const status = modalRoot.querySelector<HTMLElement>("#running-app-status");
      const options = modalRoot.querySelector<HTMLDataListElement>("#running-app-options");
      const refresh = modalRoot.querySelector<HTMLButtonElement>("#refresh-apps");
      if (refresh) refresh.disabled = true;
      if (status) status.textContent = "Looking for desktop and audio applications…";
      try {
        const [processNames, sessions] = await Promise.all([
          listRunningProcesses(),
          listAudioSessions("output").catch(() => [] as AudioSession[]),
        ]);
        availableApplications = runningApplications(processNames, sessions);
        if (options) {
          options.innerHTML = availableApplications
            .map((application) => `<option value="${escapeHtml(application.key)}">${escapeHtml(application.label)}</option>`)
            .join("");
        }
        if (status) {
          status.textContent = availableApplications.length
            ? `${availableApplications.length} likely app${availableApplications.length === 1 ? "" : "s"} available. Audio apps include their friendly stream name when known.`
            : "No likely applications found. You can still type an executable name.";
        }
      } catch (error) {
        if (status) status.textContent = `Could not list running audio apps: ${errorMessage(error)}`;
      } finally {
        if (refresh) refresh.disabled = false;
      }
    };

    modalRoot.querySelector<HTMLButtonElement>("#refresh-apps")?.addEventListener("click", () =>
      void refreshApplications(),
    );
    void refreshApplications();

    modalRoot.querySelector<HTMLButtonElement>("#add-target")?.addEventListener("click", () => {
      draft.targets.push({ app: "", volume: 100 });
      renderTargets();
    });
    modalRoot.querySelectorAll<HTMLElement>("[data-close-modal]").forEach((element) => {
      element.addEventListener("click", (event) => {
        if (event.target === element) modalRoot.innerHTML = "";
      });
    });
    modalRoot.querySelector<HTMLFormElement>("#preset-form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      const form = event.currentTarget as HTMLFormElement;
      const name = new FormData(form).get("name")?.toString().trim() ?? "";
      const error = modalRoot.querySelector<HTMLElement>("#preset-form-error");
      if (!name || !hotkey || draft.targets.some((target) => !target.app.trim())) {
        if (error) error.textContent = "Name, hotkey, and every application are required.";
        return;
      }
      if (draft.targets.some((target) => target.volume < 0 || target.volume > 100)) {
        if (error) error.textContent = "Target volumes must be between 0 and 100.";
        return;
      }
      if (draft.default_volume != null && (!Number.isFinite(draft.default_volume) || draft.default_volume < 0 || draft.default_volume > 100)) {
        if (error) error.textContent = "The all-other-streams volume must be between 0 and 100.";
        return;
      }
      const next = cloneConfig(this.config!);
      const saved: Preset = { name, hotkey, default_volume: draft.default_volume, targets: draft.targets };
      if (index === undefined) next.presets.push(saved);
      else next.presets[index] = saved;
      void this.persistConfig(next, "Preset saved").then((success) => {
        if (success) modalRoot.innerHTML = "";
        else if (error) error.textContent = "Could not save. Check the message below.";
      });
    });

    modalRoot.querySelector<HTMLInputElement>('input[name="name"]')?.focus();
  }

  private async deletePreset(index: number): Promise<void> {
    if (!this.config) return;
    const preset = this.config.presets[index];
    if (!window.confirm(`Delete preset “${preset.name}”?`)) return;
    const next = cloneConfig(this.config);
    next.presets.splice(index, 1);
    await this.persistConfig(next, "Preset deleted");
  }

  private renderMixer(): void {
    if (!this.capabilities) return;
    const inputSupported = this.capabilities.application_input;
    const content = this.content();
    content.innerHTML = `${this.pageHeader(
      "currently running streams",
      "Mixer",
      '<button class="button button--quiet" id="refresh-mixer" type="button">↻ refresh</button>',
    )}<section class="page-body">
      <div class="mixer-toolbar" role="tablist" aria-label="Audio direction">
        <button class="segment ${this.mixerDirection === "output" ? "segment--active" : ""}" type="button" data-direction="output">output</button>
        <button class="segment ${this.mixerDirection === "input" ? "segment--active" : ""}" type="button" data-direction="input" ${inputSupported ? "" : "disabled"}>input${inputSupported ? "" : " · unavailable"}</button>
        <span class="live-indicator"><i></i> live</span>
      </div>
      <div id="mixer-content">${volumeRows(this.mixerSessions)}</div>
      ${!inputSupported ? '<p class="platform-note">Per-application input volume is unavailable on Windows. Output mixing remains fully supported.</p>' : ""}
    </section>`;
    content.querySelector<HTMLButtonElement>("#refresh-mixer")?.addEventListener("click", () => void this.refreshMixer(true));
    content.querySelectorAll<HTMLButtonElement>("[data-direction]").forEach((button) => {
      button.addEventListener("click", () => {
        this.mixerDirection = button.dataset.direction as AudioDirection;
        this.mixerSessions = [];
        this.renderMixer();
        void this.refreshMixer(true);
      });
    });
    wireVolumeSliders(content, (message) => this.showToast(message, "error"));
  }

  private async refreshMixer(force = false): Promise<void> {
    if (this.mixerPending || !this.capabilities) return;
    if (this.mixerDirection === "input" && !this.capabilities.application_input) return;
    this.mixerPending = true;
    try {
      this.mixerSessions = await listAudioSessions(this.mixerDirection);
      const adjusting = Boolean(document.querySelector(".volume-slider:active"));
      if (this.activeTab === "mixer" && (!adjusting || force)) this.renderMixer();
    } catch (error) {
      this.showToast(errorMessage(error), "error");
    } finally {
      this.mixerPending = false;
    }
  }

  private renderSettings(): void {
    if (!this.config || !this.capabilities) return;
    const content = this.content();
    content.innerHTML = `${this.pageHeader("runtime preferences", "Settings")}
      <section class="page-body settings-layout">
        <form class="settings-card" id="settings-form">
          <div class="settings-card__header"><div><span class="section-number">01</span><h2>General</h2></div></div>
          <label class="toggle-row"><span><strong>Run in tray</strong><small>Keep hotkeys active when the main window closes.</small></span><input type="checkbox" name="run-in-tray" ${this.config.general.run_in_tray ? "checked" : ""}/><i></i></label>
          <label class="field"><span>HUD hotkey</span><button class="hotkey-capture" id="hud-hotkey" type="button"><span data-hotkey-value></span><small>click, then press keys</small></button></label>
          <button class="button button--primary settings-save" type="submit">save settings</button>
        </form>
        <section class="settings-card">
          <div class="settings-card__header"><div><span class="section-number">02</span><h2>Backend</h2></div><span class="status-pill">online</span></div>
          <dl class="capability-list">
            <div><dt>application output</dt><dd>${this.capabilities.application_output ? "supported" : "unavailable"}</dd></div>
            <div><dt>application input</dt><dd>${this.capabilities.application_input ? "supported" : "unavailable on Windows"}</dd></div>
          </dl>
        </section>
        <section class="settings-card settings-card--wide">
          <div class="settings-card__header"><div><span class="section-number">03</span><h2>Config file</h2></div><div class="config-actions"><button class="button button--quiet" id="import-config" type="button">import</button><button class="button button--quiet" id="export-config" type="button">export</button></div></div>
          <code class="config-path">${escapeHtml(this.configPath)}</code>
          <p class="settings-help">Import replaces the active config after validating its schema and shortcuts. Export writes a portable TOML copy.</p>
        </section>
      </section>`;

    let hudHotkey = this.config.general.hud_hotkey;
    const capture = content.querySelector<HTMLButtonElement>("#hud-hotkey");
    if (capture) bindShortcutCapture(capture, hudHotkey, (value) => (hudHotkey = value));
    content.querySelector<HTMLFormElement>("#settings-form")?.addEventListener("submit", (event) => {
      event.preventDefault();
      const next = cloneConfig(this.config!);
      const checkbox = content.querySelector<HTMLInputElement>('[name="run-in-tray"]');
      next.general.run_in_tray = checkbox?.checked ?? true;
      next.general.hud_hotkey = hudHotkey;
      void this.persistConfig(next, "Settings saved");
    });
    content.querySelector<HTMLButtonElement>("#import-config")?.addEventListener("click", () =>
      void this.importConfiguration(),
    );
    content.querySelector<HTMLButtonElement>("#export-config")?.addEventListener("click", () =>
      void this.exportConfiguration(),
    );
  }

  private async importConfiguration(): Promise<void> {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "TOML configuration", extensions: ["toml"] }],
      });
      if (typeof selected !== "string") return;
      if (!window.confirm("Replace the active duckd configuration with this file?")) return;

      this.config = await importConfig(selected);
      this.updateResidentStatus();
      this.renderActiveTab();
      this.showToast("Configuration imported", "success");
    } catch (error) {
      this.showToast(errorMessage(error), "error");
    }
  }

  private async exportConfiguration(): Promise<void> {
    try {
      const selected = await save({
        defaultPath: "duckd-config.toml",
        filters: [{ name: "TOML configuration", extensions: ["toml"] }],
      });
      if (!selected) return;

      await exportConfig(selected);
      this.showToast("Configuration exported", "success");
    } catch (error) {
      this.showToast(errorMessage(error), "error");
    }
  }

  private async persistConfig(next: AppConfig, message: string): Promise<boolean> {
    try {
      await saveConfig(next);
      this.config = next;
      this.updateResidentStatus();
      this.renderActiveTab();
      this.showToast(message, "success");
      return true;
    } catch (error) {
      this.showToast(errorMessage(error), "error");
      return false;
    }
  }

  private showToast(message: string, tone: ToastTone): void {
    const toast = root.querySelector<HTMLElement>("#toast");
    if (!toast) return;
    toast.textContent = message;
    toast.dataset.tone = tone;
    toast.classList.add("toast--visible");
    window.setTimeout(() => toast.classList.remove("toast--visible"), 3600);
  }
}

class HudApplication {
  private pending = false;

  mount(): void {
    void applyInterfaceScale().catch((error: unknown) => {
      console.warn("Could not apply HUD scale", error);
    });
    document.documentElement.classList.add("hud-document");
    root.innerHTML = `<main class="hud-shell">
      <header class="hud-header" data-tauri-drag-region>
        <div><span class="brand-mark">d:</span> quick mixer</div>
        <div class="hud-header__actions"><span class="live-indicator"><i></i> live</span><button class="icon-button" id="close-hud" type="button" aria-label="Close HUD">×</button></div>
      </header>
      <section class="hud-content" id="hud-content"><div class="loading-state"><span></span><span></span><span></span></div></section>
      <footer class="hud-footer">esc to dismiss <span>output streams</span></footer>
    </main>`;

    root.querySelector<HTMLButtonElement>("#close-hud")?.addEventListener("click", () => void this.hide());
    document.addEventListener("keydown", (event) => {
      if (event.code === "Escape") void this.hide();
    });
    window.setInterval(() => void this.refresh(), 1200);
    void this.refresh();
  }

  private async hide(): Promise<void> {
    if (isDesktop) await getCurrentWindow().hide();
  }

  private async refresh(): Promise<void> {
    if (this.pending || !isDesktop) return;
    this.pending = true;
    try {
      const sessions = await listAudioSessions("output");
      const content = root.querySelector<HTMLElement>("#hud-content");
      if (content) {
        const adjusting = Boolean(content.querySelector(".volume-slider:active"));
        if (!adjusting) {
          content.innerHTML = volumeRows(sessions, true);
          wireVolumeSliders(content, (message) => this.showError(message));
        }
      }
    } catch (error) {
      this.showError(errorMessage(error));
    } finally {
      this.pending = false;
    }
  }

  private showError(message: string): void {
    const content = root.querySelector<HTMLElement>("#hud-content");
    if (content) content.innerHTML = `<div class="hud-error">${escapeHtml(message)}</div>`;
  }
}

const isHud = new URLSearchParams(window.location.search).get("view") === "hud";
if (isHud) new HudApplication().mount();
else void new MainApplication().mount();
