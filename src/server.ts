import * as fs from "node:fs";
import * as http from "node:http";
import * as path from "node:path";
import { WebSocket, WebSocketServer } from "ws";
import { CaptureState, D2RCapture } from "./capture";
import { Config } from "./config";
import { Timer, TimerSnapshot } from "./timer";

export interface AppSnapshot extends TimerSnapshot {
  serverTime: number;
  capture: CaptureState;
  config: {
    decimals: number;
    scale: number;
    showStatus: boolean;
  };
  configPath: string;
}

export interface AppServerOptions {
  config: Config;
  timer: Timer;
  capture: D2RCapture;
  configPath: string;
  publicDir?: string;
}

export const ACTIONS = [
  "start",
  "stop",
  "start-stop",
  "reset",
  "pause-resume",
] as const;

type Action = (typeof ACTIONS)[number];

const MIME_TYPES: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".txt": "text/plain; charset=utf-8",
};

const STATIC_ALIASES: Record<string, string> = {
  "/": "control.html",
  "/control": "control.html",
  "/control/": "control.html",
  "/overlay": "overlay.html",
  "/overlay/": "overlay.html",
};

type AliveWebSocket = WebSocket & { isAlive?: boolean };

export class AppServer {
  private readonly server: http.Server;
  private readonly wss: WebSocketServer;
  private readonly clients = new Set<WebSocket>();
  private readonly config: Config;
  private readonly timer: Timer;
  private readonly capture: D2RCapture;
  private readonly configPath: string;
  private readonly publicDir: string;
  private tickTimer: NodeJS.Timeout | null = null;
  private heartbeatTimer: NodeJS.Timeout | null = null;

  constructor(options: AppServerOptions) {
    this.config = options.config;
    this.timer = options.timer;
    this.capture = options.capture;
    this.configPath = options.configPath;
    this.publicDir =
      options.publicDir || path.resolve(__dirname, "..", "public");

    this.server = http.createServer((request, response) => {
      void this.handleRequest(request, response);
    });

    this.wss = new WebSocketServer({ noServer: true });

    this.server.on("upgrade", (request, socket, head) => {
      let pathname: string;

      try {
        pathname = new URL(
          request.url || "/",
          `http://${request.headers.host || "localhost"}`
        ).pathname;
      } catch {
        socket.destroy();
        return;
      }

      if (pathname !== "/ws") {
        socket.destroy();
        return;
      }

      this.wss.handleUpgrade(request, socket, head, (webSocket) => {
        this.wss.emit("connection", webSocket, request);
      });
    });

    this.wss.on("connection", (webSocket) => {
      const aliveSocket = webSocket as AliveWebSocket;
      aliveSocket.isAlive = true;
      this.clients.add(webSocket);

      webSocket.on("pong", () => {
        aliveSocket.isAlive = true;
      });

      webSocket.on("close", () => {
        this.clients.delete(webSocket);
      });

      webSocket.on("error", () => {
        this.clients.delete(webSocket);
      });

      this.sendState(webSocket);
    });
  }

  async start(): Promise<void> {
    await new Promise<void>((resolve, reject) => {
      const onError = (error: Error) => {
        this.server.off("listening", onListening);
        reject(error);
      };

      const onListening = () => {
        this.server.off("error", onError);
        resolve();
      };

      this.server.once("error", onError);
      this.server.once("listening", onListening);
      this.server.listen(this.config.port, this.config.host);
    });

    this.tickTimer = setInterval(() => {
      this.timer.setCaptureAvailable(this.capture.isAvailable());

      if (this.clients.size > 0) {
        this.broadcast();
      }
    }, 250);
    this.tickTimer.unref();

    this.heartbeatTimer = setInterval(() => {
      for (const client of this.wss.clients) {
        const aliveSocket = client as AliveWebSocket;

        if (aliveSocket.isAlive === false) {
          aliveSocket.terminate();
          continue;
        }

        aliveSocket.isAlive = false;
        aliveSocket.ping();
      }
    }, 30000);
    this.heartbeatTimer.unref();
  }

  async close(): Promise<void> {
    if (this.tickTimer) {
      clearInterval(this.tickTimer);
      this.tickTimer = null;
    }

    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }

    for (const client of this.clients) {
      try {
        client.terminate();
      } catch {
        // Client already closed.
      }
    }
    this.clients.clear();

    await new Promise<void>((resolve) => {
      this.wss.close(() => resolve());
    });

    await new Promise<void>((resolve) => {
      this.server.close(() => resolve());
      this.server.closeAllConnections?.();
    });
  }

  snapshot(): AppSnapshot {
    this.timer.setCaptureAvailable(this.capture.isAvailable());
    const timerState = this.timer.snapshot();

    return {
      ...timerState,
      serverTime: Date.now(),
      capture: this.capture.getState(),
      config: {
        decimals: this.config.timer.decimals,
        scale: this.config.overlay.scale,
        showStatus: this.config.overlay.showStatus,
      },
      configPath: this.configPath,
    };
  }

  broadcast() {
    if (this.clients.size === 0) {
      return;
    }

    const payload = JSON.stringify({
      type: "state",
      state: this.snapshot(),
    });

    for (const client of this.clients) {
      if (client.readyState === WebSocket.OPEN) {
        client.send(payload);
      }
    }
  }

  private sendState(webSocket: WebSocket) {
    if (webSocket.readyState !== WebSocket.OPEN) {
      return;
    }

    webSocket.send(
      JSON.stringify({
        type: "state",
        state: this.snapshot(),
      })
    );
  }

  private async handleRequest(
    request: http.IncomingMessage,
    response: http.ServerResponse
  ) {
    try {
      const url = new URL(
        request.url || "/",
        `http://${request.headers.host || "localhost"}`
      );
      const pathname = decodeURIComponent(url.pathname);

      if (
        pathname === "/api/state" &&
        (request.method === "GET" || request.method === "HEAD")
      ) {
        this.sendJson(response, 200, this.snapshot());
        return;
      }

      if (pathname === "/healthz" && request.method === "GET") {
        this.sendJson(response, 200, { ok: true });
        return;
      }

      if (pathname === "/api/frame" && request.method === "GET") {
        this.handleFrame(response);
        return;
      }

      if (pathname.startsWith("/api/action/")) {
        if (request.method !== "POST") {
          this.sendJson(response, 405, {
            error: "Use POST for timer actions.",
          });
          return;
        }

        this.handleAction(pathname.slice("/api/action/".length), response);
        return;
      }

      if (pathname.startsWith("/api/")) {
        this.sendJson(response, 404, { error: "Not found" });
        return;
      }

      if (request.method !== "GET" && request.method !== "HEAD") {
        this.sendText(response, 405, "Method Not Allowed");
        return;
      }

      await this.handleStatic(
        pathname,
        request.method === "HEAD",
        response
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      console.error(`[server] Request failed: ${message}`);

      if (!response.headersSent) {
        this.sendJson(response, 500, { error: "Internal server error" });
      } else {
        response.end();
      }
    }
  }

  private handleAction(action: string, response: http.ServerResponse) {
    const handlers: Record<Action, () => void> = {
      start: () => this.timer.start(),
      stop: () => this.timer.stop(),
      "start-stop": () => this.timer.startStop(),
      reset: () => this.timer.reset(),
      "pause-resume": () => this.timer.togglePause(),
    };

    const handler = handlers[action as Action];

    if (!handler) {
      this.sendJson(response, 400, {
        error: `Unknown action: ${action}`,
        actions: ACTIONS,
      });
      return;
    }

    handler();
    this.broadcast();
    this.sendJson(response, 200, this.snapshot());
  }

  private handleFrame(response: http.ServerResponse) {
    this.capture.requestPreview();
    const preview = this.capture.getPreview();

    if (!preview) {
      this.sendJson(response, 404, { error: "No captured frame yet" });
      return;
    }

    response.writeHead(200, {
      "Content-Type": "application/octet-stream",
      "Content-Length": String(preview.data.length),
      "Cache-Control": "no-store",
      "X-Frame-Width": String(preview.width),
      "X-Frame-Height": String(preview.height),
    });
    response.end(preview.data);
  }

  private async handleStatic(
    pathname: string,
    headOnly: boolean,
    response: http.ServerResponse
  ) {
    const filePath = this.resolveStaticPath(pathname);

    if (!filePath) {
      this.sendText(response, 404, "Not found");
      return;
    }

    try {
      const data = await fs.promises.readFile(filePath);
      const extension = path.extname(filePath).toLowerCase();
      const contentType =
        MIME_TYPES[extension] || "application/octet-stream";

      response.writeHead(200, {
        "Content-Type": contentType,
        "Content-Length": String(data.length),
        "Cache-Control": "no-store",
      });
      response.end(headOnly ? undefined : data);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") {
        this.sendText(response, 404, "Not found");
        return;
      }

      throw error;
    }
  }

  private resolveStaticPath(pathname: string): string | null {
    const alias = STATIC_ALIASES[pathname];

    if (alias) {
      return path.resolve(this.publicDir, alias);
    }

    const relative = pathname.replace(/^\/+/, "");
    const resolved = path.resolve(this.publicDir, relative);

    if (
      resolved !== this.publicDir &&
      !resolved.startsWith(`${this.publicDir}${path.sep}`)
    ) {
      return null;
    }

    return resolved;
  }

  private sendJson(
    response: http.ServerResponse,
    statusCode: number,
    body: unknown
  ) {
    const payload = JSON.stringify(body);
    response.writeHead(statusCode, {
      "Content-Type": "application/json; charset=utf-8",
      "Content-Length": String(Buffer.byteLength(payload)),
      "Cache-Control": "no-store",
    });
    response.end(payload);
  }

  private sendText(
    response: http.ServerResponse,
    statusCode: number,
    body: string
  ) {
    response.writeHead(statusCode, {
      "Content-Type": "text/plain; charset=utf-8",
      "Content-Length": String(Buffer.byteLength(body)),
      "Cache-Control": "no-store",
    });
    response.end(body);
  }
}
