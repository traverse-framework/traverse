import { BundleEmbedder, NodeFsBundleLoader } from "traverse-embedder-web";
import { fileURLToPath } from "node:url";

const manifestPath = fileURLToPath(
  new URL("../../applications/traverse-starter/app.manifest.json", import.meta.url),
);

const embedder = await BundleEmbedder.init({
  manifestPath,
  loader: new NodeFsBundleLoader(),
  platform: "web",
});

embedder.subscribe((event) => {
  console.log(`[${event.event_type}]`, JSON.stringify(event.data));
});

const outcome = embedder.submit("traverse-starter.process", { note: "hello" });

console.log("\nsubmit status:", outcome.status);
