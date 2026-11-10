import { execFile, spawn, type ChildProcess } from "node:child_process";
import { Config } from "./config";
import {
  detectLoading,
  MIN_IMAGE_SIZE,
  RawFrameSource,
} from "./loading-detection";

export type CaptureStatus =
  | "searching"
  | "starting"
  | "running"
  | "lost"
  | "error";

export interface CaptureState {
  status: CaptureStatus;
  windowId: number | null;
  windowTitle: string | null;
  width: number;
  height: number;
  outputWidth: number;
  outputHeight: number;
  fps: number;
  error: string | null;
  lastFrameAt: number | null;
}

export interface CapturePreview {
  data: Buffer;
  width: number;
  height: number;
  at: number;
}

interface WindowInfo {
  id: number;
  title: string;
  width: number;
  height: number;
}

export interface D2RCaptureOptions {
  config: Config;
  onLoadingChanged: (loading: boolean) => void;
  onStateChanged: (state: CaptureState) => void;
}

const POLL_INTERVAL_MS = 2000;
const FRAME_TIMEOUT_MS = 1000;
const RESTART_AFTER_STALL_MS = 3000;
const START_TIMEOUT_MS = 5000;
const PREVIEW_INTERVAL_MS = 100;
const PREVIEW_REQUEST_TTL_MS = 1000;

function execFileText(
  file: string,
  args: string[],
  env: NodeJS.ProcessEnv,
  timeout = 3000
): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(
      file,
      args,
      { env, timeout, encoding: "utf8", maxBuffer: 1024 * 1024 },
      (error, stdout) => {
        if (error) {
          reject(error);
          return;
        }
        resolve(stdout);
      }
    );
  });
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function rgb24ToRgba(
  frame: Uint8Array,
  width: number,
  height: number
): Buffer {
  const pixelCount = width * height;
  const output = Buffer.alloc(pixelCount * 4);

  for (let pixel = 0, source = 0; pixel < pixelCount; ++pixel, source += 3) {
    const destination = pixel * 4;
    output[destination] = frame[source];
    output[destination + 1] = frame[source + 1];
    output[destination + 2] = frame[source + 2];
    output[destination + 3] = 255;
  }

  return output;
}

export class D2RCapture {
  private readonly config: Config;
  private readonly display: string;
  private readonly options: D2RCaptureOptions;
  private readonly intentionalStops = new WeakSet<ChildProcess>();
  private readonly state: CaptureState;
  private pollTimer: NodeJS.Timeout | null = null;
  private polling = false;
  private ffmpeg: ChildProcess | null = null;
  private ffmpegStartedAt = 0;
  private stderrTail = "";
  private frameBuffer: Buffer | null = null;
  private frameOffset = 0;
  private outputWidth = 0;
  private outputHeight = 0;
  private lastFrameAt: number | null = null;
  private lastPreviewAt = 0;
  private previewRequestedAt = 0;
  private preview: CapturePreview | null = null;
  private loading = false;
  private everCaptured = false;
  private wasRunning = false;
  private lastErrorLog: string | null = null;

  constructor(options: D2RCaptureOptions) {
    this.options = options;
    this.config = options.config;
    this.display =
      this.config.capture.display || process.env.DISPLAY || ":0";
    this.state = {
      status: "searching",
      windowId: null,
      windowTitle: null,
      width: 0,
      height: 0,
      outputWidth: 0,
      outputHeight: 0,
      fps: this.config.capture.fps,
      error: null,
      lastFrameAt: null,
    };
  }

  start() {
    if (this.pollTimer) {
      return;
    }

    this.pollTimer = setInterval(() => {
      void this.poll();
    }, POLL_INTERVAL_MS);
    this.pollTimer.unref();

    void this.poll();
  }

  stop() {
    if (this.pollTimer) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }

    this.stopFfmpeg();
  }

  getState(): CaptureState {
    return {
      ...this.state,
      lastFrameAt: this.lastFrameAt,
    };
  }

  isAvailable(now = Date.now()) {
    return (
      this.state.status === "running" &&
      this.lastFrameAt !== null &&
      now - this.lastFrameAt < FRAME_TIMEOUT_MS
    );
  }

  getPreview(): CapturePreview | null {
    return this.preview;
  }

  /**
   * Marks the preview as recently consumed. Frame conversion only happens while
   * a control page (or another client) is actively requesting preview frames.
   */
  requestPreview(now = Date.now()) {
    this.previewRequestedAt = now;
  }

  private childEnvironment(): NodeJS.ProcessEnv {
    return {
      ...process.env,
      DISPLAY: this.display,
    };
  }

  private updateState(patch: Partial<CaptureState>) {
    Object.assign(this.state, patch);
    this.options.onStateChanged(this.getState());
  }

  private logError(message: string) {
    if (this.lastErrorLog !== message) {
      this.lastErrorLog = message;
      console.warn(`[capture] ${message}`);
    }
  }

  private setLoading(value: boolean) {
    if (this.loading === value) {
      return;
    }

    this.loading = value;
    this.options.onLoadingChanged(value);
  }

  private async listWindowIds(
    selector: "class" | "name",
    pattern: string
  ): Promise<number[]> {
    try {
      const stdout = await execFileText(
        "xdotool",
        [
          "search",
          "--onlyvisible",
          `--${selector}`,
          escapeRegExp(pattern),
        ],
        this.childEnvironment()
      );

      return [
        ...new Set(
          stdout
            .trim()
            .split(/\s+/)
            .map((value) => Number.parseInt(value, 10))
            .filter((value) => Number.isInteger(value) && value > 0)
        ),
      ];
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") {
        throw new Error(
          "xdotool is not installed. Install the xdotool package for your distribution."
        );
      }

      const stderr = String(
        (error as { stderr?: string | Buffer }).stderr || ""
      );

      if (/can't open display|unable to open display/i.test(stderr)) {
        throw new Error(
          `Cannot open X display "${this.display}". Set capture.display in the config or run inside a graphical session.`
        );
      }

      // xdotool exits with status 1 when no window matches.
      return [];
    }
  }

  private async getWindowInfo(id: number): Promise<WindowInfo | null> {
    try {
      const [title, geometry] = await Promise.all([
        execFileText(
          "xdotool",
          ["getwindowname", String(id)],
          this.childEnvironment()
        ),
        execFileText(
          "xdotool",
          ["getwindowgeometry", "--shell", String(id)],
          this.childEnvironment()
        ),
      ]);

      const values = new Map<string, string>();
      for (const line of geometry.split("\n")) {
        const index = line.indexOf("=");
        if (index > 0) {
          values.set(line.slice(0, index), line.slice(index + 1).trim());
        }
      }

      const width = Number.parseInt(values.get("WIDTH") || "0", 10);
      const height = Number.parseInt(values.get("HEIGHT") || "0", 10);

      if (!Number.isFinite(width) || !Number.isFinite(height)) {
        return null;
      }

      return {
        id,
        title: title.trim(),
        width,
        height,
      };
    } catch {
      return null;
    }
  }

  private async findWindow(): Promise<WindowInfo | null> {
    const ids = [
      ...new Set([
        ...(await this.listWindowIds(
          "class",
          this.config.capture.windowClass
        )),
        ...(await this.listWindowIds(
          "name",
          this.config.capture.windowTitle
        )),
      ]),
    ];

    if (ids.length === 0) {
      return null;
    }

    const windows = await Promise.all(ids.map((id) => this.getWindowInfo(id)));
    const matches = windows
      .filter((window): window is WindowInfo => window !== null)
      .filter(
        (window) =>
          window.width >= this.config.capture.minWidth &&
          window.height >= this.config.capture.minHeight
      )
      .sort((a, b) => b.width * b.height - a.width * a.height);

    return matches[0] || null;
  }

  private computeOutputSize(width: number, height: number) {
    let outputWidth = MIN_IMAGE_SIZE;
    let outputHeight = Math.round((height * outputWidth) / width);

    if (outputHeight > MIN_IMAGE_SIZE) {
      outputHeight = MIN_IMAGE_SIZE;
      outputWidth = Math.round((width * outputHeight) / height);
    }

    // rawvideo frame sizes need consistent even dimensions
    outputWidth = Math.max(2, outputWidth - (outputWidth % 2));
    outputHeight = Math.max(2, outputHeight - (outputHeight % 2));

    return { outputWidth, outputHeight };
  }

  private startFfmpeg(window: WindowInfo) {
    this.stopFfmpeg();

    const { outputWidth, outputHeight } = this.computeOutputSize(
      window.width,
      window.height
    );

    this.outputWidth = outputWidth;
    this.outputHeight = outputHeight;
    this.frameBuffer = Buffer.alloc(outputWidth * outputHeight * 3);
    this.frameOffset = 0;
    this.stderrTail = "";
    this.ffmpegStartedAt = Date.now();

    console.log(
      `[capture] Found "${window.title}" ${window.width}x${window.height} (id ${window.id}), capturing at ${outputWidth}x${outputHeight} @ ${this.config.capture.fps} fps`
    );

    this.updateState({
      status: "starting",
      windowId: window.id,
      windowTitle: window.title,
      width: window.width,
      height: window.height,
      outputWidth,
      outputHeight,
      fps: this.config.capture.fps,
      error: null,
    });

    const args = [
      "-hide_banner",
      "-loglevel",
      "error",
      "-f",
      "x11grab",
      "-draw_mouse",
      "0",
      "-window_id",
      String(window.id),
      "-framerate",
      String(this.config.capture.fps),
      "-i",
      this.display,
      "-vf",
      `scale=${outputWidth}:${outputHeight}:flags=fast_bilinear`,
      "-pix_fmt",
      "rgb24",
      "-f",
      "rawvideo",
      "pipe:1",
    ];

    const child = spawn("ffmpeg", args, {
      env: this.childEnvironment(),
      stdio: ["ignore", "pipe", "pipe"],
    });

    this.ffmpeg = child;
    let failed = false;

    const fail = (message: string) => {
      if (failed || this.intentionalStops.has(child)) {
        return;
      }
      failed = true;

      if (this.ffmpeg === child) {
        this.ffmpeg = null;
      }

      this.frameBuffer = null;
      this.frameOffset = 0;
      this.lastFrameAt = null;
      this.wasRunning = false;
      this.setLoading(false);
      this.updateState({
        status: "error",
        error: message,
      });

      this.logError(message);
    };

    child.stdout?.on("data", (chunk: Buffer) => {
      this.handleChunk(chunk);
    });

    child.stderr?.on("data", (chunk: Buffer) => {
      this.stderrTail = `${this.stderrTail}${chunk.toString("utf8")}`.slice(
        -2048
      );
    });

    child.on("error", (error) => {
      const message =
        (error as NodeJS.ErrnoException).code === "ENOENT"
          ? "ffmpeg is not installed. Install the ffmpeg package for your distribution."
          : `ffmpeg failed: ${error.message}`;
      fail(message);
    });

    child.on("exit", (code, signal) => {
      if (this.intentionalStops.has(child)) {
        return;
      }

      if (this.ffmpeg === child) {
        this.ffmpeg = null;
      }

      const detail = this.stderrTail.trim();
      fail(
        detail ||
          `ffmpeg exited unexpectedly (code ${code ?? "none"}${
            signal ? `, signal ${signal}` : ""
          })`
      );
    });
  }

  private stopFfmpeg() {
    const child = this.ffmpeg;

    this.frameBuffer = null;
    this.frameOffset = 0;
    this.outputWidth = 0;
    this.outputHeight = 0;
    this.lastFrameAt = null;
    this.ffmpegStartedAt = 0;
    this.wasRunning = false;
    this.setLoading(false);

    if (!child) {
      return;
    }

    this.ffmpeg = null;
    this.intentionalStops.add(child);
    child.kill("SIGTERM");

    const killTimer = setTimeout(() => {
      try {
        child.kill("SIGKILL");
      } catch {
        // Process has already exited.
      }
    }, 1000);
    killTimer.unref();
  }

  private handleChunk(chunk: Buffer) {
    if (!this.frameBuffer || this.outputWidth === 0 || this.outputHeight === 0) {
      return;
    }

    const frameSize = this.outputWidth * this.outputHeight * 3;
    let offset = 0;

    while (offset < chunk.length) {
      const remaining = frameSize - this.frameOffset;
      const take = Math.min(remaining, chunk.length - offset);

      chunk.copy(this.frameBuffer, this.frameOffset, offset, offset + take);
      this.frameOffset += take;
      offset += take;

      if (this.frameOffset === frameSize) {
        this.handleFrame(this.frameBuffer);
        this.frameOffset = 0;
      }
    }
  }

  private handleFrame(frame: Buffer) {
    const now = Date.now();
    const wasRunning = this.wasRunning;

    this.lastFrameAt = now;
    this.wasRunning = true;

    if (this.state.status !== "running") {
      this.everCaptured = true;
      this.lastErrorLog = null;
      this.updateState({
        status: "running",
        error: null,
      });
    }

    if (!wasRunning) {
      console.log("[capture] Receiving frames");
    }

    const loading = detectLoading(
      new RawFrameSource(frame, this.outputWidth, this.outputHeight, 3),
      this.outputWidth,
      this.outputHeight
    );

    this.setLoading(loading);

    if (
      now - this.lastPreviewAt >= PREVIEW_INTERVAL_MS &&
      now - this.previewRequestedAt <= PREVIEW_REQUEST_TTL_MS
    ) {
      this.lastPreviewAt = now;
      this.preview = {
        data: rgb24ToRgba(frame, this.outputWidth, this.outputHeight),
        width: this.outputWidth,
        height: this.outputHeight,
        at: now,
      };
    }
  }

  private async poll() {
    if (this.polling) {
      return;
    }

    this.polling = true;

    try {
      const window = await this.findWindow();

      if (!window) {
        this.stopFfmpeg();

        const status = this.everCaptured ? "lost" : "searching";

        if (
          this.state.status !== status ||
          this.state.windowId !== null ||
          this.state.width !== 0
        ) {
          this.updateState({
            status,
            windowId: null,
            windowTitle: null,
            width: 0,
            height: 0,
            outputWidth: 0,
            outputHeight: 0,
            error:
              status === "lost" ? "D2R window lost. Waiting for it to return." : null,
          });
        }
        return;
      }

      if (!this.ffmpeg) {
        this.startFfmpeg(window);
        return;
      }

      if (window.id !== this.state.windowId) {
        console.log("[capture] D2R window changed, restarting capture");
        this.stopFfmpeg();
        this.startFfmpeg(window);
        return;
      }

      if (
        window.width !== this.state.width ||
        window.height !== this.state.height
      ) {
        console.log("[capture] D2R window size changed, restarting capture");
        this.stopFfmpeg();
        this.startFfmpeg(window);
        return;
      }

      if (this.state.windowTitle !== window.title) {
        this.updateState({ windowTitle: window.title });
      }

      const now = Date.now();

      if (
        this.lastFrameAt !== null &&
        now - this.lastFrameAt > RESTART_AFTER_STALL_MS
      ) {
        console.warn("[capture] No frames received, restarting capture");
        this.stopFfmpeg();
        this.startFfmpeg(window);
        return;
      }

      if (
        this.state.status === "starting" &&
        this.ffmpegStartedAt > 0 &&
        now - this.ffmpegStartedAt > START_TIMEOUT_MS
      ) {
        const detail = this.stderrTail.trim();
        const message =
          detail || "Timed out waiting for the first captured frame.";
        this.stopFfmpeg();
        this.updateState({
          status: "error",
          error: message,
        });
        this.logError(message);
      }
    } catch (error) {
      this.stopFfmpeg();
      const message = errorMessage(error);
      this.updateState({
        status: "error",
        error: message,
      });
      this.logError(message);
    } finally {
      this.polling = false;
    }
  }
}
