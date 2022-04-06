import { mainWindow } from "./";
import { IpcMessage } from "./types";

export function ipcSend(message: IpcMessage) {
  mainWindow.webContents.send("message", message);
}
