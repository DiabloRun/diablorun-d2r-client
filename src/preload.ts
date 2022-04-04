import { contextBridge, ipcRenderer } from "electron";

contextBridge.exposeInMainWorld("api", {
  send: (data: any) => {
    ipcRenderer.send("message", data);
  },
  receive: (func: any) => {
    ipcRenderer.on("message", (_event, ...args) => func(...args));
  },
});
