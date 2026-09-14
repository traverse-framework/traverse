import { BundleEmbedder, FetchBundleLoader } from "traverse-embedder-web";

const status = document.querySelector("#status");
const events = document.querySelector("#events");
const manifestPath = "/examples/applications/traverse-starter/app.manifest.json";

function showEvent(event) {
  const line = `[${event.event_type}] ${JSON.stringify(event.data)}`;
  events.textContent = events.textContent === "No events yet." ? line : `${events.textContent}\n${line}`;
}

try {
  const embedder = await BundleEmbedder.init({
    manifestPath,
    loader: new FetchBundleLoader(),
    platform: "web",
  });

  embedder.subscribe(showEvent);
  const outcome = embedder.submit("traverse-starter.process", { note: "hello from the browser" });
  status.textContent = `submit status: ${outcome.status}`;
} catch (error) {
  status.textContent = `init failed: ${String(error)}`;
}
