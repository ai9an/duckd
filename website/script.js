const DOWNLOADS = {
  windows: {
    label: "Download for Windows",
    url: "https://github.com/ai9an/duckd/releases/download/v1.1/duckd_1.1.0_x64-setup.exe",
  },
  linux: {
    label: "Download for Linux (AppImage)",
    url: "https://github.com/ai9an/duckd/releases/download/v1.1/duckd_1.1.0_amd64.AppImage",
  },
};

const TERMINAL_LINES = [
  { text: '$ Alt+3 pressed → preset "lockin"', type: "command" },
  { text: "  discord     100% → 20%", type: "change" },
  { text: "  spotify      80% →  0%", type: "change" },
  { text: "  firefox-bin  60% →  0%", type: "change" },
  { text: "✓ preset applied", type: "result" },
];

function configureDownloads() {
  const primaryDownload = document.querySelector("#primaryDownload");
  const primaryDownloadLabel = primaryDownload?.querySelector("span");
  const secondaryDownload = document.querySelector("#secondaryDownload");

  // Keep the terminal demo independent from download-link markup. A stale
  // cached page should not be able to stop the animation from initializing.
  if (!primaryDownload || !primaryDownloadLabel || !secondaryDownload) return;

  const isWindows = /Windows/i.test(navigator.userAgent);
  const primaryPlatform = isWindows ? "windows" : "linux";
  const secondaryPlatform = isWindows ? "linux" : "windows";

  primaryDownload.href = DOWNLOADS[primaryPlatform].url;
  primaryDownloadLabel.textContent = DOWNLOADS[primaryPlatform].label;
  secondaryDownload.href = DOWNLOADS[secondaryPlatform].url;
  secondaryDownload.textContent = DOWNLOADS[secondaryPlatform].label;
}

const terminalOutput = document.querySelector("#terminalOutput");
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
let animationRun = 0;

function wait(duration) {
  return new Promise((resolve) => window.setTimeout(resolve, duration));
}

function createTerminalLine(line) {
  const element = document.createElement("div");
  element.className = `terminal-line terminal-line--${line.type}`;
  terminalOutput.append(element);
  return element;
}

function showStaticTerminal() {
  terminalOutput.replaceChildren();

  TERMINAL_LINES.forEach((line) => {
    const element = createTerminalLine(line);
    element.textContent = line.text;
  });
}

async function playTerminal(run) {
  while (run === animationRun && !reducedMotion.matches && !document.hidden) {
    terminalOutput.replaceChildren();

    for (const line of TERMINAL_LINES) {
      if (run !== animationRun || reducedMotion.matches || document.hidden) return;

      const element = createTerminalLine(line);
      element.classList.add("is-active");
      const characterDelay = line.type === "command" ? 20 : 9;

      for (const character of line.text) {
        if (run !== animationRun || reducedMotion.matches || document.hidden) return;
        element.textContent += character;
        await wait(characterDelay);
      }

      element.classList.remove("is-active");
      await wait(line.type === "command" ? 120 : 45);
    }

    const finalLine = terminalOutput.lastElementChild;
    finalLine?.classList.add("is-active");
    await wait(1050);
    finalLine?.classList.remove("is-active");
    await wait(180);
  }
}

function updateTerminalMotion() {
  animationRun += 1;

  if (!terminalOutput) return;

  if (reducedMotion.matches) {
    showStaticTerminal();
    return;
  }

  void playTerminal(animationRun);
}

configureDownloads();

if (terminalOutput) {
  reducedMotion.addEventListener?.("change", updateTerminalMotion);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) updateTerminalMotion();
  });
  window.addEventListener("pageshow", (event) => {
    if (event.persisted) updateTerminalMotion();
  });
  updateTerminalMotion();
}
