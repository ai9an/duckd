import { invoke } from "@tauri-apps/api/core";

import type {
  AppConfig,
  AudioCapabilities,
  AudioDirection,
  AudioSession,
} from "./types";

export const isDesktop = "__TAURI_INTERNALS__" in window;

function desktopOnly(): never {
  throw new Error("This action is available in the duckd desktop app.");
}

export function getConfig(): Promise<AppConfig> {
  if (!isDesktop) desktopOnly();
  return invoke<AppConfig>("get_config");
}

export function getConfigPath(): Promise<string> {
  if (!isDesktop) desktopOnly();
  return invoke<string>("get_config_path");
}

export function saveConfig(config: AppConfig): Promise<void> {
  if (!isDesktop) desktopOnly();
  return invoke<void>("save_config", { config });
}

export function importConfig(path: string): Promise<AppConfig> {
  if (!isDesktop) desktopOnly();
  return invoke<AppConfig>("import_config", { path });
}

export function exportConfig(path: string): Promise<void> {
  if (!isDesktop) desktopOnly();
  return invoke<void>("export_config", { path });
}

export function getAudioCapabilities(): Promise<AudioCapabilities> {
  if (!isDesktop) desktopOnly();
  return invoke<AudioCapabilities>("get_audio_capabilities");
}

export function listAudioSessions(
  direction: AudioDirection,
): Promise<AudioSession[]> {
  if (!isDesktop) desktopOnly();
  return invoke<AudioSession[]>("list_audio_sessions", { direction });
}

export function listRunningProcesses(): Promise<string[]> {
  if (!isDesktop) desktopOnly();
  return invoke<string[]>("list_running_processes");
}

export function setApplicationVolume(
  app: string,
  direction: AudioDirection,
  volumePercent: number,
): Promise<number> {
  if (!isDesktop) desktopOnly();
  return invoke<number>("set_application_volume", {
    app,
    direction,
    volumePercent,
  });
}
