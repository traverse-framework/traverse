import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const packageRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = join(packageRoot, "../../..");
const exampleRoot = join(packageRoot, "examples/audio-analysis");

test("audio-analysis browser example serves a runtime-owned bundle", async (t) => {
  const server = createServer(async (request, response) => {
    const path = request.url?.split("?")[0] ?? "/";
    const source = path === "/" ? join(exampleRoot, "index.html")
      : path === "/app.manifest.json" ? join(repositoryRoot, "examples/applications/audio-analysis/app.manifest.json")
        : path === "/runtime.wasm" ? join(repositoryRoot, "examples/applications/audio-analysis/runtime/runtime.wasm")
          : null;
    if (source === null) { response.statusCode = 404; response.end(); return; }
    response.end(await readFile(source));
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => server.close());
  const address = server.address();
  assert.ok(address && typeof address !== "string");
  const base = `http://127.0.0.1:${address.port}`;

  const [page, manifest, runtime] = await Promise.all([
    fetch(`${base}/`).then((response) => response.text()),
    fetch(`${base}/app.manifest.json`).then((response) => response.json()),
    fetch(`${base}/runtime.wasm`).then((response) => response.arrayBuffer()),
  ]);
  assert.match(page, /Runtime events/);
  assert.match(page, /runtime\.wasm/);
  assert.equal(manifest.app_id, "audio-analysis");
  assert.equal(manifest.state_machine.initial_state, "idle");
  assert.equal(manifest.state_machine.states.some((state) => state.invoke?.host_connector === "audio.capture"), true);
  assert.equal(manifest.state_machine.states.some((state) => state.invoke?.capability_id === "doc-approval.analyze"), true);
  assert.ok(runtime.byteLength > 1024, "bundle must include a real runtime.wasm");
});
