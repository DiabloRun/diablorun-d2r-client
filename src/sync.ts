import * as chokidar from "chokidar";
import { getPayload, getPayloadHeader, Payload } from "./DI";
import fetch from "electron-fetch";
import { getLiveSplitClient } from "./livesplit";
import { ipcSend } from "./ipc";

const apiUrl = "https://api.diablo.run/sync";

export function syncSaveFileDir(savesDir: string) {
  console.log("Watching", savesDir);

  const watcher = chokidar.watch(`${savesDir}/*.d2s`, { persistent: true });
  let ready = false;

  watcher.on("ready", () => {
    ready = true;
  });

  watcher.on("add", async (file) => {
    // Start timer if new character created
    if (ready) {
      try {
        const client = await getLiveSplitClient();

        client.reset();
        client.pauseGameTime();
        client.startTimer();

        setTimeout(() => {
          ipcSend({ forceUnpauseGameTime: 1 });
        }, 500);
      } catch (err) {
        console.log(err);
      }
    }

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

  watcher.on("unlink", async (file) => {});

  watcher.on("change", async (file) => {
    console.log("change", file);
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
  try {
    const payloadHeader = await getPayloadHeader("ProcessFound");
    await postJSON(payloadHeader);
  } catch (err) {
    console.log("Sync process failed");
    console.log(err);
  }
}
