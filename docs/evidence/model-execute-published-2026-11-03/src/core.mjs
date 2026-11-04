// The Browser and Node hosts run this same file, importing only the published npm package.
import {
  BundleEmbedder, ExactModelBrowserHost, MODEL_RUNTIME_CONNECTOR, MODEL_RUNTIME_CONNECTOR_VERSION,
  exactModelMediatedConnector,
} from "traverse-embedder-web";

const hex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
const unhex = (text) => Uint8Array.from(text.match(/../g), (pair) => parseInt(pair, 16));

export async function runScenarios({ read, loader, manifestPath, platform }) {
  const json = async (path) => JSON.parse(new TextDecoder().decode(await read(path)));
  const scenarios = await json("src/scenarios.json");
  const vector = await json("published/package/vector.json");
  const input = await json("published/input.json");
  const results = [];
  for (const scenario of scenarios) {
    const pin = scenario.pin_digest ? { ...vector.pin, digest: scenario.pin_digest } : vector.pin;
    const host = new ExactModelBrowserHost([pin], {
      trustedPublicKeysHex: scenario.untrusted ? [] : [vector.trusted_public_key_hex],
      ...(scenario.no_usage ? {} : { modelUsage: "non_commercial" }),
    });
    const wasm = new Uint8Array(await read("published/package/model.wasm"));
    if (scenario.tamper_wasm) wasm[wasm.length - 1] ^= 1;
    const registration_reason = await host.registerPackage(
      await read("published/package/model.manifest.json"), wasm, await read("published/package/model.sig.json"),
      { policy_ref: "policy:default-wasm-cpu" },
    ).then(() => null, (error) => error.reason ?? String(error));
    if (scenario.revoke) host.setPackageStatus({ [vector.pin.digest]: { status: "revoked", reason: "demo" } });
    const embedder = await BundleEmbedder.init({ manifestPath, loader, platform });
    const events = [];
    embedder.subscribe((event) => events.push(event));
    if (!scenario.unbound) {
      embedder.activateMediatedConnector(MODEL_RUNTIME_CONNECTOR, MODEL_RUNTIME_CONNECTOR_VERSION,
        exactModelMediatedConnector(host, { allowedClassifications: ["public"] }));
    }
    host.io.stageModelInput(unhex(vector.cases[0].input_frame_hex), 4096);
    const outcome = await embedder.submitAsync("model.execute", input);
    const terminal = events.find((e) => e.event_type === "capability_result" || e.event_type === "error");
    const output = terminal?.data?.output ?? null;
    results.push({
      name: scenario.name, registration_reason, submit_status: outcome.status,
      terminal_event: terminal?.event_type ?? null, output,
      output_frame_hex: output?.output_ref ? hex(host.io.readModelOutput(output.output_ref, 56)) : null,
      expected_frame_hex: scenario.name === "success" ? vector.cases[0].output_frame_hex : undefined,
    });
  }
  return results;
}
