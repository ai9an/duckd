# AGENTS.md — duckd

Working slug for this project is `duckd` ("ducking" is the actual audio engineering term for auto-lowering one source when another plays). This file is the source of truth for any agent (Codex, Claude Code, etc.) working on the repo. Read it fully before writing code.

## What this is

A lightweight, cross-platform (Windows + Linux/Bazzite) desktop app that lets the user bind global hotkeys to volume presets — e.g. `Shift+F1` sets Discord to 25% and the current game to 100%, `Shift+F2` quietens the game and raises Spotify. On top of that it's a normal audio manager: manual per-app mixer, input/output device control, all fully configurable.

Main window has config/preset editing, the full mixer, and settings. Closing the window doesn't quit — it drops to the system tray and keeps listening for hotkeys in the background. There's also a quick toggleable HUD/popup mixer (small, borderless, drops down/pops up on a hotkey — think a terminal quake-console, not a full window) for fast adjustments without opening the main window.

## Tech stack

- **[Tauri v2](https://tauri.app/)** — Rust backend, web frontend, small binaries, native tray + global shortcuts support. This was chosen specifically over Electron for binary size and native OS integration.
- Frontend: vanilla TypeScript + Vite, no framework. Keep it lightweight — the UI is a mixer and some forms, it doesn't need React/Svelte/etc. Scaffold with `npm create tauri-app@latest`.
- Global hotkeys: [`tauri-plugin-global-shortcut`](https://v2.tauri.app/plugin/global-shortcut/) ([crate](https://crates.io/crates/tauri-plugin-global-shortcut)). Registers OS-level shortcuts that fire even when the window is hidden — required for the tray-resident behavior.
- Tray icon + menu: Tauri's built-in [tray-icon](https://v2.tauri.app/learn/system-tray/) API (part of `tauri` core in v2, no separate plugin needed).
- Config format: **TOML**, via the [`toml`](https://crates.io/crates/toml) crate + `serde`. Human-editable, fits a terminal-tool vibe better than JSON, diffs cleanly if the user versions their config.

## Platform audio backends (the hard part)

Per-app volume control is fundamentally different on each OS. Don't try to unify it behind one clever abstraction until both sides actually work — build them as two separate backend modules behind a shared trait/interface.

**Windows** — per-app (per-session) output volume via WASAPI/Core Audio's `ISimpleAudioVolume`. Don't hand-roll the COM bindings; use the [`windows-volume-control`](https://crates.io/crates/windows-volume-control) crate ([docs](https://docs.rs/windows-volume-control)), which wraps exactly this. Fall back to the raw [`windows`](https://crates.io/crates/windows) crate only if that wrapper is missing something.
- **Known limitation**: Windows does not expose per-app *input* (mic) volume — input control is device-level only (pick a mic, set its level). Don't promise per-app mic presets on Windows; the UI should reflect this instead of silently failing.

**Linux (Bazzite / PipeWire)** — Bazzite ships PipeWire, but it runs a PulseAudio-compatible layer (`pipewire-pulse`) by default, so targeting the Pulse protocol works out of the box without writing native PipeWire client code. Use the [`pulsectl`](https://github.com/krruzic/pulsectl) crate (wraps `libpulse-binding`) for `SinkController`/`SourceController` — it gives per-application sink-input/source-output volume directly. If you need lower-level access later, [`libpulse-binding`](https://crates.io/crates/libpulse-binding) is the underlying crate.
- Linux advantage over Windows: per-app *input* volume actually works here (PulseAudio/PipeWire track source-outputs per app), so the input side of presets can be fuller on Linux. Reflect that asymmetry in the UI/config schema rather than lying about parity.

**Both platforms**: match running apps to preset entries by process/executable name (e.g. `discord.exe` / `Discord`, `spotify` / `Spotify`). If an app named in a preset isn't currently running, skip it silently — don't error the whole preset.

## Known risk: Wayland global shortcuts

Bazzite runs KDE Plasma, which on Wayland uses the `xdg-desktop-portal` GlobalShortcuts portal rather than X11-style raw key grabbing. `tauri-plugin-global-shortcut` has portal support but it's newer and less battle-tested than the X11 path. Flag this early — test hotkey registration on the actual Bazzite/Wayland session before building the rest of the app on top of it, since it's the one thing that could force a design change (e.g. falling back to a "press to bind" capture UI if silent registration fails).

## Config schema (draft — adjust as needed)

```toml
[general]
run_in_tray = true
hud_hotkey = "Ctrl+Shift+Space"

[[presets]]
name = "Focus"
hotkey = "Shift+F1"
[[presets.targets]]
app = "Discord"
volume = 25
[[presets.targets]]
app = "MyGame.exe"
volume = 100

[[presets]]
name = "Chill"
hotkey = "Shift+F2"
[[presets.targets]]
app = "MyGame.exe"
volume = 40
[[presets.targets]]
app = "Spotify"
volume = 80
```

Config file location:
- Windows: `C:\Users\user\AppData\Roaming\audiomgr\config.toml`
- Linux: `/home/aidan/.config/audiomgr/config.toml`

Use Tauri's `app_config_dir()` API rather than hardcoding these — the paths above are what it'll resolve to on each OS, not literal strings to bake in.

## UI / design direction

Terminal-inspired, not literally a terminal emulator:
- Background: very dark, near-black (`#0a0a0c` – `#121214` range), not pure `#000`.
- Font: monospace throughout — [JetBrains Mono](https://www.jetbrains.com/lp/mono/) or [Fira Code](https://github.com/tonsky/FiraCode) are good defaults, both free/open.
- One accent color for active/selected state (sliders, active preset), everything else muted grays. Avoid a "gamer RGB" look — this should read as a tool, not a peripheral app.
- Full window: sidebar or tab layout for Presets / Mixer / Settings.
- HUD popup: small, borderless, always-on-top, shows just the mixer sliders for currently-running apps, dismiss on click-away or hotkey again.
- Customizability: Nearly all of this should be customizable to an extent in settings of the app

## Non-goals (for now)

- macOS support — out of scope, don't add cfg branches for it.
- Cloud sync / accounts — presets are local config files only.
- Audio effects/EQ — this is a volume/routing tool, not a DSP tool.

## Build & dev

- Install [Rust](https://www.rust-lang.org/tools/install) and [Node.js](https://nodejs.org/) first.
- `npm run tauri dev` — dev mode.
- `npm run tauri build` — produces platform installers via the Tauri bundler.
- Linux packaging note: Bazzite is an immutable/atomic Fedora base (`rpm-ostree`), so a `.rpm` won't layer cleanly without disrupting the base image. Prefer **[AppImage](https://appimage.org/)** as the primary Linux artifact (portable, no package manager involved) alongside whatever Tauri's bundler produces for other distros.
