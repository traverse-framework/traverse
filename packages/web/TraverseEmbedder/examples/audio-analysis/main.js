import {
  BundleEmbedder,
  FetchBundleLoader,
  createBrowserAudioInputAdapters,
} from "/pkg/index.js";

const manifestPath = "/repo/examples/applications/audio-analysis/app.manifest.json";
const status = document.querySelector("#status");
const eventPanel = document.querySelector("#events");
const buttons = Object.fromEntries(
  ["permission", "capture", "analyze", "reset"].map((name) => [name, document.querySelector(`#${name}`)]),
);
const events = [];

function renderEvents() {
  eventPanel.textContent = JSON.stringify(events, null, 2);
}

function setEnabled(enabled) {
  for (const button of Object.values(buttons)) button.disabled = !enabled;
}

try {
  const embedder = await BundleEmbedder.init({
    manifestPath,
    loader: new FetchBundleLoader(),
    platform: "browser",
  });
  const audio = createBrowserAudioInputAdapters({
    stageArtifact: (bytes, maxBytes) => crypto.randomUUID() + `-${Math.min(bytes.byteLength, maxBytes)}`,
  });
  embedder.registerHostConnectorAdapter("audio.permission.request", audio.requestPermission);
  embedder.registerHostConnectorAdapter("audio.capture", audio.captureAudio);
  embedder.subscribe((event) => {
    events.push(event);
    renderEvents();
  });

  const submit = (command, payload = {}) => {
    const last = [...events].reverse().find((event) => typeof event.session_id === "string");
    embedder.submit({ kind: "app_command", command, payload, sessionId: last?.session_id });
  };
  buttons.permission.addEventListener("click", () => submit("request_permission"));
  buttons.capture.addEventListener("click", () => submit("capture_audio", { max_duration_ms: 5000, max_bytes: 1048576 }));
  buttons.analyze.addEventListener("click", () => submit("analyze"));
  buttons.reset.addEventListener("click", () => submit("reset"));
  setEnabled(true);
  status.textContent = "Verified bundle loaded. Runtime owns the session state.";
} catch (error) {
  status.textContent = `Bundle initialization failed: ${String(error)}`;
}
