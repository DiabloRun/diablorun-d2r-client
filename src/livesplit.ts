import LiveSplitClient from "livesplit-client";
import { ipcSend } from "./ipc";
import { getSettings } from "./settings";

let client: LiveSplitClient;

export async function updateLiveSplitClient() {
  const { liveSplitEnabled, liveSplitServer } = await getSettings();

  if (client && !liveSplitEnabled) {
    client.disconnect();
  }

  if (!liveSplitEnabled) {
    ipcSend({ liveSplitStatus: "disconnected" });
    return;
  }

  ipcSend({ liveSplitStatus: "connecting" });

  client = new LiveSplitClient(liveSplitServer);
  await client.connect();

  if (client.connected) {
    ipcSend({ liveSplitStatus: "connected" });
  }
}

export function getLiveSplitClient() {
  return client;
}

/*
process.on("uncaughtException", (error) => {
  if (error.message.match(/^connect ECONNREFUSED/)) {
    ipcSend({ liveSplitStatus: "connecting" });
    setTimeout(() => updateLiveSplitClient(), 3000);
  }
});
*/

setInterval(() => {
  if (client && !client.connected) {
    updateLiveSplitClient();
  }
}, 3000);
