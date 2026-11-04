import { readFile, writeFile } from "node:fs/promises";
import { NodeFsBundleLoader } from "traverse-embedder-web";
import { runScenarios } from "./core.mjs";
const results = await runScenarios({
  read: async (path) => new Uint8Array(await readFile(path)),
  loader: new NodeFsBundleLoader(), manifestPath: "bundle/app.manifest.json", platform: "web",
});
await writeFile("proof/node.json", JSON.stringify({ host: `node ${process.version}`, results }, null, 2));
console.log(results.map((r) => `${r.name}: reg=${r.registration_reason} ${JSON.stringify(r.output).slice(0, 110)}`).join("\n"));
