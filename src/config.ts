import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

export interface CaptureConfig {
  fps: number;
  display: string;
  windowClass: string;
  windowTitle: string;
  minWidth: number;
  minHeight: number;
}

export interface TimerConfig {
  decimals: number;
}

export interface OverlayConfig {
  scale: number;
  showStatus: boolean;
}

export interface Config {
  host: string;
  port: number;
  capture: CaptureConfig;
  timer: TimerConfig;
  overlay: OverlayConfig;
}

export function createDefaultConfig(): Config {
  return {
    host: "127.0.0.1",
    port: 8787,
    capture: {
      fps: 25,
      display: process.env.DISPLAY || ":0",
      windowClass: "d2r.exe",
      windowTitle: "Diablo II: Resurrected",
      minWidth: 160,
      minHeight: 160,
    },
    timer: {
      decimals: 2,
    },
    overlay: {
      scale: 1,
      showStatus: false,
    },
  };
}

export function getConfigPath(): string {
  if (process.env.D2R_TIMER_CONFIG) {
    return path.resolve(process.env.D2R_TIMER_CONFIG);
  }

  const base =
    process.env.XDG_CONFIG_HOME || path.join(os.homedir(), ".config");

  return path.join(base, "diablorun-web", "config.json");
}

function mergeDefaults<T>(defaults: T, value: unknown): T {
  if (defaults !== null && typeof defaults === "object") {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      return defaults;
    }

    const result: Record<string, unknown> = {};
    for (const key of Object.keys(defaults as Record<string, unknown>)) {
      result[key] = mergeDefaults(
        (defaults as Record<string, unknown>)[key],
        (value as Record<string, unknown>)[key]
      );
    }
    return result as T;
  }

  if (typeof value === typeof defaults) {
    if (typeof value === "number" && !Number.isFinite(value)) {
      return defaults;
    }
    return value as T;
  }

  return defaults;
}

function clamp(value: number, min: number, max: number, fallback: number) {
  if (!Number.isFinite(value)) {
    return fallback;
  }
  return Math.min(max, Math.max(min, Math.round(value)));
}

function normalizeConfig(config: Config): Config {
  config.port = clamp(config.port, 1, 65535, 8787);
  config.capture.fps = clamp(config.capture.fps, 1, 60, 25);
  config.capture.minWidth = clamp(config.capture.minWidth, 1, 16384, 160);
  config.capture.minHeight = clamp(config.capture.minHeight, 1, 16384, 160);
  config.timer.decimals = clamp(config.timer.decimals, 0, 3, 2);
  config.overlay.scale = Math.min(
    5,
    Math.max(0.25, Number(config.overlay.scale) || 1)
  );

  if (!config.host || typeof config.host !== "string") {
    config.host = "127.0.0.1";
  }
  if (!config.capture.display || typeof config.capture.display !== "string") {
    config.capture.display = process.env.DISPLAY || ":0";
  }
  if (!config.capture.windowClass) {
    config.capture.windowClass = "d2r.exe";
  }
  if (!config.capture.windowTitle) {
    config.capture.windowTitle = "Diablo II: Resurrected";
  }

  return config;
}

export function loadConfig(): Config {
  const configPath = getConfigPath();
  const defaults = createDefaultConfig();

  try {
    const raw = fs.readFileSync(configPath, "utf8");
    const parsed = JSON.parse(raw) as unknown;
    return normalizeConfig(mergeDefaults(defaults, parsed));
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;

    if (code === "ENOENT") {
      try {
        fs.mkdirSync(path.dirname(configPath), { recursive: true });
        fs.writeFileSync(
          configPath,
          `${JSON.stringify(defaults, null, 2)}\n`,
          "utf8"
        );
      } catch (writeError) {
        console.warn(
          `[config] Could not create ${configPath}: ${(writeError as Error).message}`
        );
      }
    } else {
      console.warn(
        `[config] Could not read ${configPath}: ${(error as Error).message}. Using defaults.`
      );
    }

    return defaults;
  }
}
