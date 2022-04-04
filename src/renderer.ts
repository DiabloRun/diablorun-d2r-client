import "./index.css";

const windowApi = (window as any).api;
const apiKeyInput = document.getElementById("apiKey") as HTMLInputElement;
const savesDir = document.getElementById("savesDir");

apiKeyInput.addEventListener("input", () => {
  windowApi.send({ settings: { apiKey: apiKeyInput.value } });
});

windowApi.receive((message: any) => {
  if (message.settings) {
    apiKeyInput.value = message.settings.apiKey || "";
    savesDir.textContent = message.settings.savesDir || "";
  }

  if (message.log) {
    console.log(message.log);
  }
});

document
  .getElementById("selectSavesDirButton")
  .addEventListener("click", () => {
    windowApi.send({ selectSavesDirButton: true });
  });
