import { BundleEmbedder, FetchBundleLoader } from "/pkg/index.js";

const { createElement: h, useEffect, useState } = React;

const MANIFEST_PATH = "/repo/apps/traverse-starter/app.manifest.json";
const CAPABILITY_ID = "traverse-starter.process";

function eventList(events) {
  if (events.length === 0) {
    return h("p", null, "No events yet.");
  }
  return h(
    "ol",
    null,
    events.map((event) =>
      h(
        "li",
        { key: event.event_id },
        h("strong", null, event.event_type),
        " — ",
        h("code", null, JSON.stringify(event.data)),
      ),
    ),
  );
}

function App() {
  const [embedder, setEmbedder] = useState(null);
  const [initError, setInitError] = useState(null);
  const [events, setEvents] = useState([]);
  const [note, setNote] = useState("hello from React, no sidecar");
  const [evidence, setEvidence] = useState(null);

  useEffect(() => {
    let cancelled = false;
    BundleEmbedder.init({
      manifestPath: MANIFEST_PATH,
      loader: new FetchBundleLoader(),
      platform: "web",
    })
      .then((instance) => {
        if (cancelled) {
          return;
        }
        instance.subscribe((event) => {
          setEvents((previous) => [...previous, event]);
        });
        setEmbedder(instance);
        setEvidence(instance.releaseEvidence());
      })
      .catch((error) => {
        if (!cancelled) {
          setInitError(String(error));
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const submit = () => {
    if (embedder === null) {
      return;
    }
    embedder.submit(CAPABILITY_ID, { note });
  };

  return h(
    "div",
    null,
    h("h1", null, "traverse-embedder-web — traverse-starter, no sidecar"),
    h(
      "p",
      null,
      "This page loads the checked-in traverse-starter application bundle directly " +
        "from the repository over static HTTP and executes its bundled WASM capability " +
        "in this browser tab via BundleEmbedder — there is no traverse-cli serve process " +
        "involved.",
    ),
    initError !== null &&
      h("div", { className: "card status-error" }, h("strong", null, "init failed: "), initError),
    embedder !== null &&
      h(
        "div",
        { className: "card" },
        h("p", { className: "status-ok" }, "Bundle initialized without a sidecar."),
        h("input", {
          value: note,
          onChange: (event) => setNote(event.target.value),
        }),
        h("button", { onClick: submit }, "Submit " + CAPABILITY_ID),
      ),
    h(
      "div",
      { className: "card" },
      h("h2", null, "Events"),
      eventList(events),
    ),
    evidence !== null &&
      h(
        "div",
        { className: "card" },
        h("h2", null, "Release evidence"),
        h("pre", null, JSON.stringify(evidence, null, 2)),
      ),
  );
}

ReactDOM.createRoot(document.getElementById("root")).render(h(App));
