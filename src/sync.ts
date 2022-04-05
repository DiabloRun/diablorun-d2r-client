import * as chokidar from "chokidar";
import { getPayload, getPayloadHeader, Payload } from "./DI";
import fetch from "electron-fetch";
import { ipcMain } from "electron";

const apiUrl = "https://api.diablo.run/sync";

export function syncSaveFileDir(savesDir: string) {
  console.log("Watching", savesDir);

  const files: string[] = [];
  const watcher = chokidar.watch(`${savesDir}/*.d2s`, { persistent: true });

  watcher.on("add", async (file) => {
    files.push(file);
    ipcMain.emit("files", [...files]);

    try {
      const payload = await getPayload(file);
      await postJSON(payload);
    } catch (err) {
      console.log("Sync failed", file, err);

      const header = await getPayloadHeader("DataRead");

      await postJSON({
        ...header,
        Seed: 0,
        SeedIsArg: false,
        Name: file.match(/(\w+)\.d2s$/)[1],
        Guid: "",
      } as Payload);
    }
  });

  watcher.on("unlink", async (file) => {
    const index = files.indexOf(file);

    if (index !== -1) {
      files.splice(index, 1);
    }

    ipcMain.emit("files", [...files]);
  });

  watcher.on("change", async (file) => {
    const payload = await getPayload(file);
    await postJSON(payload);
  });

  return watcher;
}

async function postJSON(body: any) {
  return await fetch(apiUrl, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });
}

export async function syncProcess() {
  console.log("Sync process");

  try {
    const payloadHeader = await getPayloadHeader("ProcessFound");
    await postJSON(payloadHeader);
  } catch (err) {
    console.log("Sync process failed");
    console.log(err);
  }
}
