import { app, BrowserWindow, ipcMain, dialog } from "electron";
import * as path from "path";
import settings from "electron-settings";
import { syncProcess, syncSaveFileDir } from "./sync";
import { FSWatcher } from "chokidar";

// This allows TypeScript to pick up the magic constant that's auto-generated by Forge's Webpack
// plugin that tells the Electron app where to look for the Webpack-bundled app code (depending on
// whether you're running in development or production).
declare const MAIN_WINDOW_WEBPACK_ENTRY: string;

// Handle creating/removing shortcuts on Windows when installing/uninstalling.
if (require("electron-squirrel-startup")) {
  // eslint-disable-line global-require
  app.quit();
}

export let mainWindow: BrowserWindow;
let watcher: FSWatcher;

const createWindow = (): void => {
  // Create the browser window.
  mainWindow = new BrowserWindow({
    width: 800,
    height: 600,
    autoHideMenuBar: true,
    webPreferences: {
      nodeIntegration: true,
      preload: path.join(__dirname, "..", "preload", "index.js"),
    },
  });

  // and load the index.html of the app.
  mainWindow.loadURL(MAIN_WINDOW_WEBPACK_ENTRY);

  // Open the DevTools.
  // mainWindow.webContents.openDevTools();

  // Set up watcher when window is ready
  mainWindow.webContents.on("dom-ready", async () => {
    const allSettings = await settings.get();

    if (!watcher && allSettings.savesDir) {
      watcher = syncSaveFileDir(allSettings.savesDir as string);
    }

    mainWindow.webContents.send("message", {
      settings: allSettings,
    });
  });
};

// This method will be called when Electron has finished
// initialization and is ready to create browser windows.
// Some APIs can only be used after this event occurs.
app.on("ready", createWindow);

// Quit when all windows are closed, except on macOS. There, it's common
// for applications and their menu bar to stay active until the user quits
// explicitly with Cmd + Q.
app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    app.quit();
  }
});

app.on("activate", () => {
  // On OS X it's common to re-create a window in the app when the
  // dock icon is clicked and there are no other windows open.
  if (BrowserWindow.getAllWindows().length === 0) {
    createWindow();
  }
});

// In this file you can include the rest of your app's specific main process
// code. You can also put them in separate files and import them here.
ipcMain.on("message", async (_event, message) => {
  if (message.settings) {
    for (const keyPath in message.settings) {
      await settings.set(keyPath, message.settings[keyPath]);
    }

    if (message.settings.raceId) {
      await syncProcess();
    }
  }

  if (message.selectSavesDirButton) {
    const value = await dialog.showOpenDialog(mainWindow, {
      properties: ["openDirectory"],
    });

    if (!value.canceled && value.filePaths.length > 0) {
      // Stop previous watcher
      if (watcher) {
        watcher.removeAllListeners();
      }

      await settings.set("savesDir", value.filePaths[0]);

      // Set up new watcher
      watcher = syncSaveFileDir(value.filePaths[0]);
    }

    mainWindow.webContents.send("message", {
      settings: await settings.get(),
    });
  }
});

ipcMain.on("files", (files) => {
  console.log("FILES", files);
});
