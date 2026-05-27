// Settings overlay. Park-local knobs that affect operator capture and
// inspection, including the SDK contents root used by stream
// materialization experiments.

import { fade, fadeDownOut, fadeUpIn } from "../anim";
import { fetchSettings, saveSdkContentsRoot } from "../data/settings";
import { iconSettings } from "../icons";
import { escapeHtml } from "../util/escape";
import {
  shouldBurnMetadata,
  setBurnMetadata,
} from "../views/robot/screenshot";
import { toast } from "./toast";

let openOverlay: HTMLElement | null = null;

export function showSettingsOverlay(): void {
  if (openOverlay) return;

  const root = document.createElement("div");
  root.className =
    "fixed inset-0 bg-ink/70 backdrop-blur-sm z-[60] flex items-center justify-center";

  root.innerHTML = `
    <div class="w-full max-w-[520px] mx-4 bg-ink-alt border border-paper/10 rounded-md shadow-2xl overflow-hidden flex flex-col max-h-[80vh]" data-region="panel">
      <div class="px-5 pt-4 pb-3 border-b border-paper/10 flex items-center gap-3">
        <span class="text-paper/60">${iconSettings(16)}</span>
        <h2 class="text-paper text-base font-medium flex-1" style="font-family: var(--font-display)">Settings</h2>
        <button class="text-rule hover:text-paper text-base leading-none w-6 h-6 flex items-center justify-center" data-region="close" title="Close (Esc)">×</button>
      </div>
      <div class="overflow-y-auto" data-region="body">
        <div class="px-5 py-4 border-b border-paper/10">
          <h3 class="text-rule text-[11px] uppercase tracking-[0.2em] mb-3" style="font-family: var(--font-display)">SDK Contents</h3>
          <form class="space-y-2" data-region="sdk-form">
            <label class="block text-paper text-[13px]" for="sdk-contents-root">Root directory</label>
            <div class="flex gap-2">
              <input id="sdk-contents-root" data-region="sdk-root" type="text" autocomplete="off" autocapitalize="none" autocorrect="off" spellcheck="false" class="min-w-0 flex-1 rounded-sm border border-paper/10 bg-ink px-3 py-2 text-[12px] text-paper font-mono outline-none focus:border-accent/70" placeholder="~/.auki/park" />
              <button data-region="sdk-save" type="submit" class="shrink-0 rounded-sm border border-accent/60 px-3 py-2 text-[11px] uppercase tracking-[0.14em] text-paper hover:bg-accent/15 disabled:opacity-50 disabled:cursor-wait">Save</button>
              <button data-region="sdk-reset" type="button" class="shrink-0 rounded-sm border border-paper/10 px-3 py-2 text-[11px] uppercase tracking-[0.14em] text-rule hover:text-paper hover:border-paper/25 disabled:opacity-50">Reset</button>
            </div>
            <div class="text-[11px] leading-snug text-rule/75 font-mono break-all" data-region="sdk-status">Loading…</div>
          </form>
        </div>
        <div class="px-5 py-4 border-b border-paper/10">
          <h3 class="text-rule text-[11px] uppercase tracking-[0.2em] mb-3" style="font-family: var(--font-display)">Screenshots</h3>
          <label class="flex items-start gap-3 cursor-pointer select-none" data-region="burn-toggle">
            <input type="checkbox" class="mt-0.5 w-4 h-4 accent-accent shrink-0" data-region="burn-input" />
            <span class="flex-1 min-w-0">
              <span class="block text-paper text-[13px]">Stamp metadata onto saved frames</span>
              <span class="block text-rule/70 text-[11px] mt-0.5 leading-snug">Adds a small banner along the bottom of each PNG with sensor, timestamp, and session id. Disable for clean frames suitable for mocks or external sharing.</span>
            </span>
          </label>
        </div>
        <div class="px-5 py-4">
          <h3 class="text-rule text-[11px] uppercase tracking-[0.2em] mb-2" style="font-family: var(--font-display)">Keyboard</h3>
          <p class="text-rule/80 text-xs leading-snug">
            Press <kbd class="px-1 py-0.5 border border-paper/10 rounded text-[11px] mx-0.5 font-mono">⌘K</kbd>
            anywhere for quick search, or
            <kbd class="px-1 py-0.5 border border-paper/10 rounded text-[11px] mx-0.5 font-mono">?</kbd>
            on a robot view for the full shortcut list.
          </p>
        </div>
      </div>
    </div>
  `;

  document.body.appendChild(root);
  openOverlay = root;

  const panel = root.querySelector('[data-region="panel"]') as HTMLElement;
  const closeBtn = root.querySelector('[data-region="close"]') as HTMLButtonElement;

  fade(root, 1);
  fadeUpIn(panel);

  const close = () => {
    if (openOverlay !== root) return;
    openOverlay = null;
    window.removeEventListener("keydown", onKey);
    fade(root, 0);
    fadeDownOut(panel).finished.then(() => root.remove());
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  };
  window.addEventListener("keydown", onKey);

  closeBtn.addEventListener("click", close);
  root.addEventListener("click", (e) => {
    if (e.target === root) close();
  });

  // ─── SDK contents root ─────────────────────────────────────────────
  const sdkForm = root.querySelector(
    '[data-region="sdk-form"]',
  ) as HTMLFormElement;
  const sdkRoot = root.querySelector(
    '[data-region="sdk-root"]',
  ) as HTMLInputElement;
  const sdkSave = root.querySelector(
    '[data-region="sdk-save"]',
  ) as HTMLButtonElement;
  const sdkReset = root.querySelector(
    '[data-region="sdk-reset"]',
  ) as HTMLButtonElement;
  const sdkStatus = root.querySelector(
    '[data-region="sdk-status"]',
  ) as HTMLElement;

  let defaultRoot = "";

  const setSdkBusy = (busy: boolean) => {
    sdkRoot.disabled = busy;
    sdkSave.disabled = busy;
    sdkReset.disabled = busy || defaultRoot.length === 0;
  };

  const renderSdkStatus = (message: string, kind: "muted" | "ok" | "error" = "muted") => {
    sdkStatus.className =
      "text-[11px] leading-snug font-mono break-all " +
      (kind === "error"
        ? "text-accent"
        : kind === "ok"
          ? "text-paper/80"
          : "text-rule/75");
    sdkStatus.innerHTML = escapeHtml(message);
  };

  const loadSdkSettings = async () => {
    setSdkBusy(true);
    renderSdkStatus("Loading…");
    try {
      const settings = await fetchSettings();
      if (!root.isConnected) return;
      defaultRoot = settings.default_sdk_contents_root;
      sdkRoot.value = settings.sdk_contents_root;
      sdkRoot.placeholder = settings.default_sdk_contents_root;
      renderSdkStatus(`Saved in ${settings.settings_path}`, "muted");
    } catch (err) {
      if (!root.isConnected) return;
      const message = err instanceof Error ? err.message : String(err);
      renderSdkStatus(message, "error");
    } finally {
      if (root.isConnected) setSdkBusy(false);
    }
  };

  const saveSdkRoot = async (path: string) => {
    setSdkBusy(true);
    renderSdkStatus("Saving…");
    try {
      const settings = await saveSdkContentsRoot(path);
      if (!root.isConnected) return;
      defaultRoot = settings.default_sdk_contents_root;
      sdkRoot.value = settings.sdk_contents_root;
      sdkRoot.placeholder = settings.default_sdk_contents_root;
      renderSdkStatus(`Saved in ${settings.settings_path}`, "ok");
      toast.success("SDK contents root saved");
    } catch (err) {
      if (!root.isConnected) return;
      const message = err instanceof Error ? err.message : String(err);
      renderSdkStatus(message, "error");
      toast.error(`SDK contents root: ${message}`);
    } finally {
      if (root.isConnected) setSdkBusy(false);
    }
  };

  sdkForm.addEventListener("submit", (e) => {
    e.preventDefault();
    void saveSdkRoot(sdkRoot.value);
  });
  sdkReset.addEventListener("click", () => {
    if (!defaultRoot) return;
    void saveSdkRoot(defaultRoot);
  });
  void loadSdkSettings();

  // ─── screenshot burn-metadata toggle ───────────────────────────────
  const burnInput = root.querySelector(
    '[data-region="burn-input"]',
  ) as HTMLInputElement;
  burnInput.checked = shouldBurnMetadata();
  burnInput.addEventListener("change", () => {
    setBurnMetadata(burnInput.checked);
  });
}
