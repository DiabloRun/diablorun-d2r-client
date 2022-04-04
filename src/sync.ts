import * as chokidar from "chokidar";
import { getPayload } from "./utils";
import fetch from "electron-fetch";
import settings from "electron-settings";

export function syncSaveFileDir(savesDir: string) {
  console.log("Watching", savesDir);

  const watcher = chokidar.watch(`${savesDir}/*.d2s`, { persistent: true });

  watcher.on("add", async (file) => await syncSaveFile(file));
  watcher.on("change", async (file) => await syncSaveFile(file));

  return watcher;
}

async function syncSaveFile(file: string) {
  console.log("Sync", file);

  try {
    const { apiKey } = await settings.get();
    const payload = await getPayload(file, apiKey as string);

    await fetch("https://api.diablo.run/sync", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
    });
  } catch (err) {
    console.log("Sync failed");
    console.log(err);
  }
}
