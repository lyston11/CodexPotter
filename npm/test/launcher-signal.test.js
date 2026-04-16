import assert from "node:assert/strict";
import test from "node:test";

import { exitCodeFromSignal, reemitSignalOrExit } from "../bin/codex-potter.js";

test("exitCodeFromSignal uses conventional signal exit code", () => {
  assert.equal(exitCodeFromSignal("SIGHUP"), 129);
});

test("reemitSignalOrExit reemits supported signals", () => {
  const calls = [];
  const fakeProcess = {
    pid: 42,
    kill(pid, signal) {
      calls.push(["kill", pid, signal]);
    },
    exit(code) {
      calls.push(["exit", code]);
    },
  };

  reemitSignalOrExit(fakeProcess, "SIGTERM");

  assert.deepEqual(calls, [["kill", 42, "SIGTERM"]]);
});

test("reemitSignalOrExit falls back to exit code when reemit fails", () => {
  const calls = [];
  const fakeProcess = {
    pid: 42,
    kill() {
      throw new Error("unsupported signal");
    },
    exit(code) {
      calls.push(["exit", code]);
    },
  };

  reemitSignalOrExit(fakeProcess, "SIGHUP");

  assert.deepEqual(calls, [["exit", 129]]);
});
