import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { RuntimeWasmHost, RuntimeWasmHostError } from "../dist/index.js";

// Cross-host ordered-event conformance (Spec 139 / Spec 140 FR-013, #1502).
//
// Runs fixtures/cross-host/app-state-machine-events-v1 against the real runtime.wasm through the
// production RuntimeWasmHost and compares each step with golden.json, which the Rust reference
// (crates/traverse-runtime/tests/app_state_machine_conformance.rs) generated. Skipped unless
// TRAVERSE_NATIVE_ARTIFACT_ROOT points at a directory containing runtime/runtime.wasm, exactly
// like the Swift, Kotlin and .NET real-artifact tests. The checked-in tests/fixtures/runtime.wasm
// predates the app state machine and cannot run this fixture.

const artifactRoot = process.env.TRAVERSE_NATIVE_ARTIFACT_ROOT;
const fixtureDir = join(
  fileURLToPath(new URL(".", import.meta.url)),
  "../../../../fixtures/cross-host/app-state-machine-events-v1",
);

/** Normalizes runtime-assigned ids to $S<n> / $C<n> by first appearance. */
class Placeholders {
  sessions = [];
  commands = [];

  static name(list, prefix, id) {
    let index = list.indexOf(id);
    if (index < 0) {
      list.push(id);
      index = list.length - 1;
    }
    return `${prefix}${index + 1}`;
  }

  normalize(value) {
    if (Array.isArray(value)) return value.map((item) => this.normalize(item));
    if (value !== null && typeof value === "object") {
      const out = {};
      for (const [key, inner] of Object.entries(value)) {
        if (key === "session_id" && typeof inner === "string") {
          out[key] = Placeholders.name(this.sessions, "$S", inner);
        } else if (key === "command_id" && typeof inner === "string") {
          out[key] = Placeholders.name(this.commands, "$C", inner);
        } else {
          out[key] = this.normalize(inner);
        }
      }
      return out;
    }
    return value;
  }

  static resolve(list, placeholder, prefix) {
    return list[Number(placeholder.slice(prefix.length)) - 1];
  }
}

const encode = (value) => new TextEncoder().encode(JSON.stringify(value));

/** A rejected submit surfaces as a RuntimeWasmHostError whose message carries the response JSON. */
function rejectedResponse(error) {
  if (!(error instanceof RuntimeWasmHostError)) throw error;
  const marker = "traverse_submit rejected: ";
  const start = error.message.indexOf(marker);
  assert.notEqual(start, -1, `unexpected host error: ${error.message}`);
  return JSON.parse(error.message.slice(start + marker.length));
}

async function runScenario(runtimeBytes, header, scenario) {
  const host = await RuntimeWasmHost.instantiate(runtimeBytes);
  const init = host.init(
    {
      capabilityId: header.app_id,
      capabilityVersion: "1.0.0",
      serviceType: "stateless",
      emits: [],
      hostPlacementTarget: header.host_placement_target,
      permittedTargets: header.permitted_targets,
      stateMachine: header.state_machine,
    },
    new Uint8Array(0),
  );
  assert.equal(init.status, "ready");

  const names = new Placeholders();
  const pending = []; // { session, command } in arrival order
  const transcript = [];

  for (const [index, step] of scenario.steps.entries()) {
    const [kind, spec] = Object.entries(step)[0];
    const targetWait = () =>
      spec.command
        ? pending.find((wait) => wait.command === Placeholders.resolve(names.commands, spec.command, "$C"))
        : pending[pending.length - 1];
    let request;
    if (kind === "submit") {
      request = { kind: "app_command", command: spec.command, payload: spec.payload };
      if (spec.session) request.session_id = Placeholders.resolve(names.sessions, spec.session, "$S");
    } else if (kind === "complete") {
      const wait = targetWait();
      request = {
        kind: "host_connector_result",
        command_id: wait.command,
        session_id: wait.session,
        result_class: spec.result_class,
        payload: spec.payload,
      };
    } else if (kind === "fire_deadline") {
      const wait = targetWait();
      request = { kind: "deadline_fired", command_id: wait.command, session_id: wait.session };
    } else {
      assert.fail(`unknown step kind ${kind}`);
    }

    let response;
    let guestStatus = 0;
    try {
      response = host.submit(encode(request));
    } catch (error) {
      // The host reports only "non-zero"; the guest's rejected status is -1.
      response = rejectedResponse(error);
      guestStatus = -1;
    }
    for (const wait of response.pending_host_connector ?? []) {
      pending.push({ session: wait.session_id, command: wait.command_id });
    }
    transcript.push({
      step: index,
      kind,
      guest_status: guestStatus,
      response: names.normalize(response),
      events: names.normalize(host.drainEvents()),
    });
  }
  return transcript;
}

test(
  "web RuntimeWasmHost reproduces the golden ordered event log",
  { skip: artifactRoot ? false : "TRAVERSE_NATIVE_ARTIFACT_ROOT is not set" },
  async () => {
    const runtimeBytes = new Uint8Array(await readFile(join(artifactRoot, "runtime/runtime.wasm")));
    const fixture = JSON.parse(await readFile(join(fixtureDir, "fixture.json"), "utf8"));
    const golden = JSON.parse(await readFile(join(fixtureDir, "golden.json"), "utf8")).scenarios;

    const divergences = [];
    for (const scenario of fixture.scenarios) {
      const actual = await runScenario(runtimeBytes, fixture.init_header, scenario);
      const want = golden[scenario.id];
      if (actual.length !== want.length) {
        divergences.push(`scenario ${scenario.id}: ${actual.length} steps, expected ${want.length}`);
        continue;
      }
      for (const [index, expected] of want.entries()) {
        const got = actual[index];
        for (const field of ["kind", "guest_status", "response"]) {
          if (JSON.stringify(got[field]) !== JSON.stringify(expected[field])) {
            divergences.push(
              `scenario ${scenario.id} step ${index} ${field}: expected ${JSON.stringify(expected[field])}, got ${JSON.stringify(got[field])}`,
            );
          }
        }
        for (let eventIndex = 0; eventIndex < Math.max(got.events.length, expected.events.length); eventIndex += 1) {
          const g = got.events[eventIndex];
          const w = expected.events[eventIndex];
          if (JSON.stringify(g) !== JSON.stringify(w)) {
            divergences.push(
              `scenario ${scenario.id} step ${index} event ${eventIndex}: expected ${JSON.stringify(w)}, got ${JSON.stringify(g)}`,
            );
          }
        }
      }
    }
    assert.deepEqual(divergences, []);
  },
);
