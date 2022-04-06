import settings from "electron-settings";
import { userInfo } from "os";
import * as path from "path";
import { Settings } from "./types";

const homeDir = userInfo().homedir;
const defaultSavesDir = path.join(
  homeDir,
  "Saved Games",
  "Diablo II Resurrected"
);

const defaultSettings: Settings = {
  apiKey: "",
  savesDir: defaultSavesDir,
  raceId: "",
  liveSplitEnabled: false,
  liveSplitServer: "127.0.0.1:16834",
};

export async function getSettings(): Promise<Settings> {
  const savedSettings = await settings.get();
  return { ...defaultSettings, ...savedSettings };
}

export async function overwriteSettings(values: Partial<Settings>) {
  let keyPath: keyof Settings;

  for (keyPath in values) {
    await settings.set(keyPath, values[keyPath]);
  }
}
