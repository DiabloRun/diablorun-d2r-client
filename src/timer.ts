export type TimerStatus = "idle" | "running" | "paused" | "stopped";

export interface TimerSnapshot {
  status: TimerStatus;
  rtaMs: number;
  igtMs: number;
  rtaActive: boolean;
  igtActive: boolean;
  loading: boolean;
  captureAvailable: boolean;
}

export type Clock = () => number;

export class Timer {
  private status: TimerStatus = "idle";
  private rtaMs = 0;
  private igtMs = 0;
  private lastTick: number;
  private loading = false;
  private captureAvailable = false;

  constructor(private readonly clock: Clock = () => performance.now()) {
    this.lastTick = this.clock();
  }

  private update(now = this.clock()) {
    // Ignore negative deltas in case the clock jumps backwards.
    const delta = Math.max(0, now - this.lastTick);
    this.lastTick = now;

    if (this.status !== "running") {
      return;
    }

    this.rtaMs += delta;

    if (!this.loading && this.captureAvailable) {
      this.igtMs += delta;
    }
  }

  start(now = this.clock()) {
    this.update(now);

    if (this.status === "running" || this.status === "paused") {
      return;
    }

    this.rtaMs = 0;
    this.igtMs = 0;
    this.lastTick = now;
    this.status = "running";
  }

  stop(now = this.clock()) {
    this.update(now);

    if (this.status === "running" || this.status === "paused") {
      this.status = "stopped";
    }

    this.lastTick = now;
  }

  startStop(now = this.clock()) {
    if (this.status === "running" || this.status === "paused") {
      this.stop(now);
    } else {
      this.start(now);
    }
  }

  togglePause(now = this.clock()) {
    this.update(now);

    if (this.status === "running") {
      this.status = "paused";
    } else if (this.status === "paused") {
      this.status = "running";
    }

    this.lastTick = now;
  }

  reset(now = this.clock()) {
    this.lastTick = now;
    this.status = "idle";
    this.rtaMs = 0;
    this.igtMs = 0;
  }

  setLoading(value: boolean, now = this.clock()) {
    if (this.loading === value) {
      return false;
    }

    this.update(now);
    this.loading = value;
    return true;
  }

  setCaptureAvailable(value: boolean, now = this.clock()) {
    if (this.captureAvailable === value) {
      return false;
    }

    this.update(now);
    this.captureAvailable = value;
    return true;
  }

  snapshot(now = this.clock()): TimerSnapshot {
    this.update(now);

    const active = this.status === "running";

    return {
      status: this.status,
      rtaMs: Math.round(this.rtaMs),
      igtMs: Math.round(this.igtMs),
      rtaActive: active,
      igtActive: active && !this.loading && this.captureAvailable,
      loading: this.loading,
      captureAvailable: this.captureAvailable,
    };
  }
}
