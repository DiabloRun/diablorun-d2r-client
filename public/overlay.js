import { formatTime, readNumberParam } from "/time.js";

const params = new URLSearchParams(window.location.search);

const decimalsOverride = params.has("decimals")
  ? readNumberParam(params, "decimals", 2, 0, 3)
  : null;
const scaleOverride = params.has("scale")
  ? readNumberParam(params, "scale", 1, 0.25, 5)
  : null;
const statusParam = params.get("status");

const rtaElement = document.getElementById("rta");
const igtElement = document.getElementById("igt");
const statusElement = document.getElementById("status");
const statusTextElement = document.getElementById("statusText");

if (params.get("labels") === "0") {
  document.body.classList.add("no-labels");
}

const state = {
  snapshot: null,
  connected: false,
  decimals: 2,
  scale: 1,
  showStatus: false,
};

let lastRta = "";
let lastIgt = "";

function setText(element, value) {
  if (element.textContent !== value) {
    element.textContent = value;
  }
}

function applyConfig() {
  const snapshot = state.snapshot;

  state.decimals =
    decimalsOverride ?? snapshot?.config?.decimals ?? 2;
  state.scale = scaleOverride ?? snapshot?.config?.scale ?? 1;

  if (statusParam === "1") {
    state.showStatus = true;
  } else if (statusParam === "0") {
    state.showStatus = false;
  } else {
    state.showStatus = snapshot?.config?.showStatus ?? false;
  }

  document.documentElement.style.setProperty(
    "--scale",
    String(state.scale)
  );
}

function statusFor(snapshot) {
  if (!state.connected) {
    return { text: "DISCONNECTED", className: "bad" };
  }

  if (!snapshot) {
    return { text: "CONNECTING", className: "loading" };
  }

  if (snapshot.capture.status === "error") {
    return { text: "CAPTURE ERROR", className: "bad" };
  }

  if (snapshot.capture.status === "lost") {
    return { text: "CAPTURE LOST", className: "bad" };
  }

  if (snapshot.capture.status === "searching") {
    return { text: "WAITING FOR D2R", className: "loading" };
  }

  if (snapshot.capture.status === "starting") {
    return { text: "STARTING CAPTURE", className: "loading" };
  }

  if (!snapshot.captureAvailable) {
    return { text: "CAPTURE LOST", className: "bad" };
  }

  if (snapshot.loading) {
    return { text: "LOADING", className: "loading" };
  }

  return { text: "PLAYING", className: "ok" };
}

function updateStatus() {
  if (!state.showStatus) {
    if (!statusElement.hidden) {
      statusElement.hidden = true;
    }
    return;
  }

  const next = statusFor(state.snapshot);

  if (statusElement.hidden) {
    statusElement.hidden = false;
  }

  const className = `status ${next.className}`;

  if (statusElement.className !== className) {
    statusElement.className = className;
  }

  setText(statusTextElement, next.text);
}

function render() {
  const snapshot = state.snapshot;

  if (snapshot) {
    const now = Date.now();
    const connected = state.connected;

    const rta =
      snapshot.rtaMs +
      (connected && snapshot.rtaActive ? now - snapshot.serverTime : 0);
    const igt =
      snapshot.igtMs +
      (connected && snapshot.igtActive ? now - snapshot.serverTime : 0);

    const rtaText = formatTime(rta, state.decimals);
    const igtText = formatTime(igt, state.decimals);

    if (rtaText !== lastRta) {
      lastRta = rtaText;
      setText(rtaElement, rtaText);
    }

    if (igtText !== lastIgt) {
      lastIgt = igtText;
      setText(igtElement, igtText);
    }

    updateStatus();
  }

  requestAnimationFrame(render);
}

let reconnectDelay = 1000;

function connect() {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const socket = new WebSocket(`${protocol}//${window.location.host}/ws`);

  socket.addEventListener("open", () => {
    state.connected = true;
    reconnectDelay = 1000;
  });

  socket.addEventListener("message", (event) => {
    try {
      const message = JSON.parse(event.data);

      if (message.type === "state" && message.state) {
        state.snapshot = message.state;
        state.connected = true;
        applyConfig();
      }
    } catch {
      // Ignore malformed messages.
    }
  });

  socket.addEventListener("close", () => {
    state.connected = false;
    applyConfig();
    window.setTimeout(connect, reconnectDelay);
    reconnectDelay = Math.min(5000, reconnectDelay * 1.5);
  });

  socket.addEventListener("error", () => {
    socket.close();
  });
}

applyConfig();
connect();
requestAnimationFrame(render);
