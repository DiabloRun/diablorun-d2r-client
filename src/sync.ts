import * as chokidar from "chokidar";
import { getPayload, getPayloadHeader } from "./utils";
import fetch from "electron-fetch";
import settings from "electron-settings";

const apiUrl = "https://api.diablo.run/sync";

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
    const payload = await getPayload(file);

    await fetch(apiUrl, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
    });
  } catch (err) {
    console.log("Sync failed: ", file);
    console.log(err);
  }
}

export async function syncProcess() {
  console.log("Sync process");

  try {
    const payloadHeader = await getPayloadHeader();

    await fetch(apiUrl, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        ...payloadHeader,
        Event: "ProcessFound",
      }),
    });
  } catch (err) {
    console.log("Sync process failed");
    console.log(err);
  }
}
