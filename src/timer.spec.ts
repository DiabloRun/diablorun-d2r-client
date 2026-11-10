import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { Timer } from "./timer";

function createClock(start = 0) {
  let now = start;

  return {
    now: () => now,
    advance(milliseconds: number) {
      now += milliseconds;
      return now;
    },
  };
}

describe("Timer", () => {
  it("counts RTA and IGT while running with capture available", () => {
    const clock = createClock();
    const timer = new Timer(clock.now);

    timer.setCaptureAvailable(true, clock.now());
    timer.start(clock.now());
    clock.advance(1000);

    const snapshot = timer.snapshot(clock.now());

    assert.equal(snapshot.status, "running");
    assert.equal(snapshot.rtaMs, 1000);
    assert.equal(snapshot.igtMs, 1000);
    assert.equal(snapshot.rtaActive, true);
    assert.equal(snapshot.igtActive, true);
  });

  it("freezes IGT while loading and keeps RTA running", () => {
    const clock = createClock();
    const timer = new Timer(clock.now);

    timer.setCaptureAvailable(true, clock.now());
    timer.start(clock.now());
    clock.advance(1000);
    timer.setLoading(true, clock.now());
    clock.advance(500);

    let snapshot = timer.snapshot(clock.now());
    assert.equal(snapshot.rtaMs, 1500);
    assert.equal(snapshot.igtMs, 1000);
    assert.equal(snapshot.igtActive, false);

    timer.setLoading(false, clock.now());
    clock.advance(500);

    snapshot = timer.snapshot(clock.now());
    assert.equal(snapshot.rtaMs, 2000);
    assert.equal(snapshot.igtMs, 1500);
    assert.equal(snapshot.igtActive, true);
  });

  it("freezes both timers during manual pause", () => {
    const clock = createClock();
    const timer = new Timer(clock.now);

    timer.setCaptureAvailable(true, clock.now());
    timer.start(clock.now());
    clock.advance(1000);
    timer.togglePause(clock.now());
    clock.advance(700);

    let snapshot = timer.snapshot(clock.now());
    assert.equal(snapshot.status, "paused");
    assert.equal(snapshot.rtaMs, 1000);
    assert.equal(snapshot.igtMs, 1000);
    assert.equal(snapshot.rtaActive, false);
    assert.equal(snapshot.igtActive, false);

    timer.togglePause(clock.now());
    clock.advance(300);

    snapshot = timer.snapshot(clock.now());
    assert.equal(snapshot.status, "running");
    assert.equal(snapshot.rtaMs, 1300);
    assert.equal(snapshot.igtMs, 1300);
  });

  it("freezes IGT when capture is lost", () => {
    const clock = createClock();
    const timer = new Timer(clock.now);

    timer.setCaptureAvailable(true, clock.now());
    timer.start(clock.now());
    clock.advance(1000);
    timer.setCaptureAvailable(false, clock.now());
    clock.advance(500);

    const snapshot = timer.snapshot(clock.now());

    assert.equal(snapshot.rtaMs, 1500);
    assert.equal(snapshot.igtMs, 1000);
    assert.equal(snapshot.igtActive, false);
  });

  it("keeps IGT paused when a run starts during a loading screen", () => {
    const clock = createClock();
    const timer = new Timer(clock.now);

    timer.setCaptureAvailable(true, clock.now());
    timer.setLoading(true, clock.now());
    timer.start(clock.now());
    clock.advance(1000);

    let snapshot = timer.snapshot(clock.now());
    assert.equal(snapshot.rtaMs, 1000);
    assert.equal(snapshot.igtMs, 0);

    timer.setLoading(false, clock.now());
    clock.advance(500);

    snapshot = timer.snapshot(clock.now());
    assert.equal(snapshot.rtaMs, 1500);
    assert.equal(snapshot.igtMs, 500);
  });

  it("toggles start and stop and starts a fresh run from stopped", () => {
    const clock = createClock();
    const timer = new Timer(clock.now);

    timer.setCaptureAvailable(true, clock.now());
    timer.startStop(clock.now());
    clock.advance(250);

    let snapshot = timer.snapshot(clock.now());
    assert.equal(snapshot.status, "running");
    assert.equal(snapshot.rtaMs, 250);

    timer.startStop(clock.now());
    clock.advance(500);

    snapshot = timer.snapshot(clock.now());
    assert.equal(snapshot.status, "stopped");
    assert.equal(snapshot.rtaMs, 250);
    assert.equal(snapshot.igtMs, 250);

    timer.startStop(clock.now());
    clock.advance(100);

    snapshot = timer.snapshot(clock.now());
    assert.equal(snapshot.status, "running");
    assert.equal(snapshot.rtaMs, 100);
    assert.equal(snapshot.igtMs, 100);
  });

  it("does not reset an already running timer when start is called again", () => {
    const clock = createClock();
    const timer = new Timer(clock.now);

    timer.setCaptureAvailable(true, clock.now());
    timer.start(clock.now());
    clock.advance(400);
    timer.start(clock.now());
    clock.advance(100);

    const snapshot = timer.snapshot(clock.now());
    assert.equal(snapshot.rtaMs, 500);
  });

  it("resets both timers to idle", () => {
    const clock = createClock();
    const timer = new Timer(clock.now);

    timer.setCaptureAvailable(true, clock.now());
    timer.start(clock.now());
    clock.advance(1000);
    timer.reset(clock.now());
    clock.advance(500);

    const snapshot = timer.snapshot(clock.now());
    assert.equal(snapshot.status, "idle");
    assert.equal(snapshot.rtaMs, 0);
    assert.equal(snapshot.igtMs, 0);
    assert.equal(snapshot.rtaActive, false);
    assert.equal(snapshot.igtActive, false);
  });
});
