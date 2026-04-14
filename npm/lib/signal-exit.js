import os from "node:os";

export function exitCodeFromSignal(signal) {
  const signalNumber = os.constants.signals[signal];
  return typeof signalNumber === "number" ? 128 + signalNumber : 1;
}

export function reemitSignalOrExit(processLike, signal) {
  try {
    processLike.kill(processLike.pid, signal);
  } catch {
    processLike.exit(exitCodeFromSignal(signal));
  }
}
