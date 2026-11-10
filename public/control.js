import { formatTime } from "/time.js";

const elements = {
  connection: document.getElementById("connection"),
  rta: document.getElementById("rta"),
  igt: document.getElementById("igt"),
  startStop: document.getElementById("startStop"),
  pauseResume: document.getElementById("pauseResume"),
  reset: document.getElementById("reset"),
  timerStatus: document.getElementById("timerStatus"),
  loadingStatus: document.getElementById("loadingStatus"),
  captureStatus: document.getElementById("captureStatus"),
  windowInfo: document.getElementById("windowInfo"),
  resolution: document.getElementById("resolution"),
  frameRate: document.getElementById("frameRate"),
  errorBox: document.getElementById("errorBox"),
  preview: document.getElementById("preview"),
  previewToggle: document.getElementById("previewToggle"),
  previewHint: document.getElementById("previewHint"),
  overlayUrl: document.getElementById("overlayUrl"),
  copyOverlay: document.getElementById("copyOverlay"),
  hotkeyCommands: document.getElementById("hotkeyCommands"),
  configPath: document.getElementById("configPath"),
};

const state = {
  snapshot: null,
  connected: false,
  decimals: 2,
};

let actionError = "";
let actionInFlight = false;
let lastRta = "";
let lastIgt = "";
let previewBusy = false;
let previewHint = "";

function setText(element, value) {
  if (element && element.textContent !== value) {
    element.textContent = value;
  }
}

function setStatusClass(element, className) {
  if (element && element.className !== className) {
    element.className = className;
  }
}

function updateConnectionBadge() {
  elements.connection.textContent = state.connected
    ? "connected"
    : "reconnecting\u2026";
  elements.connection.className = `badge ${
    state.connected ? "connected" : "disconnected"
  }`;
}

function captureState() {
  const capture = state.snapshot?.capture;

  if (!capture) {
    return { text: "unknown", className: "" };
  }

  switch (capture.status) {
    case "running":
      if (!state.snapshot?.captureAvailable) {
        return { text: "capture stalled", className: "warn" };
      }
      return { text: "capturing", className: "ok" };
    case "starting":
      return { text: "starting", className: "warn" };
    case "searching":
      return { text: "searching for D2R", className: "warn" };
    case "lost":
      return { text: "D2R window lost", className: "bad" };
    case "error":
      return { text: "error", className: "bad" };
    default:
      return { text: capture.status, className: "" };
  }
}

function updateErrorBox() {
  const messages = [];

  if (!state.connected) {
    messages.push("Lost connection to the timer server. Reconnecting\u2026");
  }

  if (state.snapshot?.capture?.error) {
    messages.push(state.snapshot.capture.error);
  }

  if (actionError) {
    messages.push(actionError);
  }

  if (messages.length === 0) {
    elements.errorBox.hidden = true;
    elements.errorBox.textContent = "";
    return;
  }

  elements.errorBox.hidden = false;
  setText(elements.errorBox, messages.join("\n"));
}

function updateButtons() {
  if (actionInFlight) {
    elements.startStop.disabled = true;
    elements.pauseResume.disabled = true;
    elements.reset.disabled = true;
    return;
  }

  const status = state.snapshot?.status || "idle";
  const active = status === "running" || status === "paused";

  elements.startStop.disabled = false;
  elements.reset.disabled = false;
  setText(elements.startStop, active ? "Stop" : "Start");
  setText(elements.pauseResume, status === "paused" ? "Resume" : "Pause");
  elements.pauseResume.disabled = !active;
}

function applySnapshot(snapshot) {
  state.snapshot = snapshot;
  state.decimals = snapshot.config?.decimals ?? 2;

  const status = snapshot.status || "idle";
  setText(elements.timerStatus, status);
  setStatusClass(
    elements.timerStatus,
    status === "running"
      ? "ok"
      : status === "paused"
        ? "warn"
        : status === "stopped"
          ? "warn"
          : ""
  );

  setText(elements.loadingStatus, snapshot.loading ? "yes" : "no");
  setStatusClass(
    elements.loadingStatus,
    snapshot.loading ? "warn" : "ok"
  );

  const capture = captureState();
  setText(elements.captureStatus, capture.text);
  setStatusClass(elements.captureStatus, capture.className);

  const windowTitle = snapshot.capture?.windowTitle;
  const windowId = snapshot.capture?.windowId;
  setText(
    elements.windowInfo,
    windowTitle
      ? `${windowTitle}${windowId ? ` (#${windowId})` : ""}`
      : "\u2014"
  );

  const width = snapshot.capture?.width || 0;
  const height = snapshot.capture?.height || 0;
  const outputWidth = snapshot.capture?.outputWidth || 0;
  const outputHeight = snapshot.capture?.outputHeight || 0;

  setText(
    elements.resolution,
    width && height
      ? `${width}\u00d7${height} \u2192 ${outputWidth}\u00d7${outputHeight}`
      : "\u2014"
  );

  setText(
    elements.frameRate,
    snapshot.capture?.fps ? `${snapshot.capture.fps} fps` : "\u2014"
  );

  if (snapshot.configPath) {
    setText(elements.configPath, snapshot.configPath);
  }

  updateButtons();
  updateErrorBox();
}

function renderTimers() {
  const snapshot = state.snapshot;

  if (snapshot) {
    const now = Date.now();
    const rta =
      snapshot.rtaMs +
      (state.connected && snapshot.rtaActive ? now - snapshot.serverTime : 0);
    const igt =
      snapshot.igtMs +
      (state.connected && snapshot.igtActive ? now - snapshot.serverTime : 0);

    const rtaText = formatTime(rta, state.decimals);
    const igtText = formatTime(igt, state.decimals);

    if (rtaText !== lastRta) {
      lastRta = rtaText;
      setText(elements.rta, rtaText);
    }

    if (igtText !== lastIgt) {
      lastIgt = igtText;
      setText(elements.igt, igtText);
    }
  }

  requestAnimationFrame(renderTimers);
}

async function runAction(action) {
  if (actionInFlight) {
    return;
  }

  actionInFlight = true;
  actionError = "";
  updateButtons();

  try {
    const response = await fetch(`/api/action/${action}`, {
      method: "POST",
    });

    const body = await response.json().catch(() => null);

    if (!response.ok) {
      throw new Error(
        body?.error || `Request failed with ${response.status}`
      );
    }

    state.connected = true;
    updateConnectionBadge();
    applySnapshot(body);
    updateErrorBox();
  } catch (error) {
    actionError =
      error instanceof Error ? error.message : String(error);
    updateErrorBox();
  } finally {
    actionInFlight = false;
    updateButtons();
  }
}

elements.startStop.addEventListener("click", () => {
  void runAction("start-stop");
});

elements.pauseResume.addEventListener("click", () => {
  void runAction("pause-resume");
});

elements.reset.addEventListener("click", () => {
  void runAction("reset");
});

elements.copyOverlay.addEventListener("click", async () => {
  const text = elements.overlayUrl.textContent.trim();

  try {
    await navigator.clipboard.writeText(text);
    elements.copyOverlay.textContent = "Copied";
  } catch {
    const range = document.createRange();
    range.selectNodeContents(elements.overlayUrl);
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    elements.copyOverlay.textContent = "Select & copy";
  }

  window.setTimeout(() => {
    elements.copyOverlay.textContent = "Copy";
  }, 1500);
});

async function updatePreview() {
  if (
    previewBusy ||
    !elements.previewToggle.checked ||
    document.hidden
  ) {
    return;
  }

  previewBusy = true;

  try {
    const response = await fetch(`/api/frame?t=${Date.now()}`);

    if (!response.ok) {
      setPreviewHint("No captured frame yet.");
      return;
    }

    const width = Number(response.headers.get("X-Frame-Width"));
    const height = Number(response.headers.get("X-Frame-Height"));
    const buffer = await response.arrayBuffer();

    if (
      !Number.isFinite(width) ||
      !Number.isFinite(height) ||
      width <= 0 ||
      height <= 0 ||
      buffer.byteLength !== width * height * 4
    ) {
      setPreviewHint("Invalid frame received.");
      return;
    }

    const canvas = elements.preview;
    const context = canvas.getContext("2d");

    if (canvas.width !== width) {
      canvas.width = width;
    }
    if (canvas.height !== height) {
      canvas.height = height;
    }

    context.putImageData(
      new ImageData(new Uint8ClampedArray(buffer), width, height),
      0,
      0
    );

    setPreviewHint(`${width}\u00d7${height} detector frame`);
  } catch {
    setPreviewHint("Preview unavailable.");
  } finally {
    previewBusy = false;
  }
}

function setPreviewHint(text) {
  if (text !== previewHint) {
    previewHint = text;
    setText(elements.previewHint, text);
  }
}

elements.previewToggle.addEventListener("change", () => {
  if (!elements.previewToggle.checked) {
    setPreviewHint("Preview paused.");
  } else {
    setPreviewHint("Waiting for the D2R window\u2026");
    void updatePreview();
  }
});

const origin = window.location.origin;
const overlayUrl = `${origin}/overlay`;

elements.overlayUrl.textContent = overlayUrl;
elements.hotkeyCommands.textContent = [
  "Start / stop",
  `  curl -s -X POST ${origin}/api/action/start-stop`,
  "",
  "Reset",
  `  curl -s -X POST ${origin}/api/action/reset`,
  "",
  "Pause / resume",
  `  curl -s -X POST ${origin}/api/action/pause-resume`,
].join("\n");

let reconnectDelay = 1000;

function connect() {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const socket = new WebSocket(`${protocol}//${window.location.host}/ws`);

  socket.addEventListener("open", () => {
    state.connected = true;
    reconnectDelay = 1000;
    updateConnectionBadge();
    updateErrorBox();
  });

  socket.addEventListener("message", (event) => {
    try {
      const message = JSON.parse(event.data);

      if (message.type === "state" && message.state) {
        state.connected = true;
        updateConnectionBadge();
        applySnapshot(message.state);
      }
    } catch {
      // Ignore malformed messages.
    }
  });

  socket.addEventListener("close", () => {
    state.connected = false;
    updateConnectionBadge();
    updateErrorBox();
    window.setTimeout(connect, reconnectDelay);
    reconnectDelay = Math.min(5000, reconnectDelay * 1.5);
  });

  socket.addEventListener("error", () => {
    socket.close();
  });
}

updateConnectionBadge();
connect();
requestAnimationFrame(renderTimers);
window.setInterval(() => {
  void updatePreview();
}, 150);

void updatePreview();
