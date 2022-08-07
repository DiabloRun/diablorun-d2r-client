import "./index.css";
import { detectLoading, MIN_IMAGE_SIZE } from "./loading-detection";
import { Settings } from "./types";

const windowApi = (window as any).api;

const state: {
  settings: Settings;
  isLoading: boolean;
  stream?: MediaStream;
} = {
  settings: {
    apiKey: "",
    raceId: "",
    savesDir: "",
    liveSplitEnabled: false,
    liveSplitServer: "",
  },
  isLoading: false,
};

const apiKeyInput = document.getElementById("apiKey") as HTMLInputElement;
const raceIdInput = document.getElementById("raceId") as HTMLInputElement;
const savesDirSpan = document.getElementById("savesDir") as HTMLSpanElement;
const savesDirButton = document.getElementById(
  "selectSavesDirButton"
) as HTMLButtonElement;

const liveSplitEnabledCheckbox = document.getElementById(
  "liveSplitEnabled"
) as HTMLInputElement;
const liveSplitServerInput = document.getElementById(
  "liveSplitServer"
) as HTMLInputElement;
const liveSplitStatusSpan = document.getElementById(
  "liveSplitStatus"
) as HTMLSpanElement;

const isLoadingSpan = document.getElementById("isLoading") as HTMLSpanElement;

const video = document.querySelector("video");
const canvas = document.querySelector("canvas");

apiKeyInput.addEventListener("input", () => {
  state.settings.apiKey = apiKeyInput.value;
  windowApi.send({ settings: state.settings });
});

/*
raceIdInput.addEventListener("input", () => {
  state.settings.raceId = raceIdInput.value;
  windowApi.send({ settings: state.settings });
});
*/

savesDirButton.addEventListener("click", () => {
  windowApi.send({ selectSavesDirButton: true });
});

liveSplitEnabledCheckbox.addEventListener("input", () => {
  state.settings.liveSplitEnabled = liveSplitEnabledCheckbox.checked;

  if (state.settings.liveSplitEnabled) {
    windowApi.send({ settings: state.settings, getCaptureSourceId: 1 });
  } else {
    stopCapture();
    windowApi.send({ settings: state.settings });
  }
});

liveSplitServerInput.addEventListener("input", () => {
  state.settings.liveSplitServer = liveSplitServerInput.value;
  windowApi.send({ settings: state.settings });
});

function setSettings(settings: Settings) {
  state.settings = settings;

  apiKeyInput.value = settings.apiKey;
  // raceIdInput.value = settings.raceId;
  savesDirSpan.textContent = settings.savesDir;
  liveSplitEnabledCheckbox.checked = settings.liveSplitEnabled;
  liveSplitServerInput.value = settings.liveSplitServer;
}

windowApi.receive(
  async (message: {
    settings?: Settings;
    log?: any;
    captureSourceId?: string;
    forceUnpauseGameTime?: number;
    liveSplitStatus?: string;
  }) => {
    if (message.settings) {
      setSettings(message.settings);
    }

    if (message.log) {
      console.log(message.log);
    }

    if (message.captureSourceId) {
      startCapture(message.captureSourceId);
    }

    if (message.forceUnpauseGameTime) {
      state.isLoading = true;
    }

    if (message.liveSplitStatus) {
      liveSplitStatusSpan.textContent = message.liveSplitStatus;
    }
  }
);

async function startCapture(sourceId: string) {
  let captureInterval: NodeJS.Timer;
  state.stream = await navigator.mediaDevices.getUserMedia({
    audio: false,
    video: {
      // @ts-ignore
      mandatory: {
        chromeMediaSource: "desktop",
        chromeMediaSourceId: sourceId,
        maxWidth: MIN_IMAGE_SIZE,
        maxHeight: MIN_IMAGE_SIZE,
        maxFrameRate: 30,
      },
    },
  });

  video.onloadedmetadata = () => {
    video.play();

    const ctx = canvas.getContext("2d");

    canvas.width = video.videoWidth;
    canvas.height = video.videoHeight;

    captureInterval = setInterval(() => {
      ctx.drawImage(video, 0, 0, video.videoWidth, video.videoHeight);

      const isLoadingNow = detectLoading(
        ctx,
        video.videoWidth,
        video.videoHeight
      );

      if (isLoadingNow !== state.isLoading) {
        windowApi.send({
          liveSplit: isLoadingNow ? "pauseGameTime" : "unpauseGameTime",
        });
      }

      state.isLoading = isLoadingNow;
      isLoadingSpan.textContent = state.isLoading ? "loading" : "playing";
    }, 50);
  };

  video.srcObject = state.stream;

  // Watch for window capture end
  const track = state.stream.getVideoTracks()[0];

  /*
  console.log(track.getConstraints());
  console.log(track.getSettings());
  console.log(track.getCapabilities());
  */

  track.addEventListener("ended", () => {
    clearInterval(captureInterval);
    windowApi.send({ getCaptureSourceId: 1 });
  });
}

function stopCapture() {
  if (state.stream) {
    for (const track of state.stream.getTracks()) {
      track.stop();
    }
  }
}
