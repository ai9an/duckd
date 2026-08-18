export type AudioDirection = "output" | "input";

export type GeneralConfig = {
  run_in_tray: boolean;
  hud_hotkey: string;
};

export type PresetTarget = {
  app: string;
  volume: number;
};

export type Preset = {
  name: string;
  hotkey: string;
  default_volume?: number | null;
  targets: PresetTarget[];
};

export type AppConfig = {
  general: GeneralConfig;
  presets: Preset[];
};

export type AudioCapabilities = {
  application_output: boolean;
  application_input: boolean;
};

export type AudioSession = {
  id: string;
  app_name: string;
  process_name: string | null;
  direction: AudioDirection;
  volume_percent: number;
  muted: boolean;
  volume_writable: boolean;
};
