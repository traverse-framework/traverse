// Runs the shipped dist/modelWorker.js inside a Node worker_threads Worker:
// Node workers have no `self.onmessage`, so bridge it to `parentPort`.
import { parentPort } from "node:worker_threads";

globalThis.postMessage = (message) => parentPort.postMessage(message);
await import("../../dist/modelWorker.js");
parentPort.on("message", (data) => globalThis.onmessage({ data }));
