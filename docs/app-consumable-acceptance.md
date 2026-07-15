# Traverse v0.1 App-Consumable Acceptance

This is the authoritative end-to-end acceptance path for the first app-consumable Traverse flow.

For the first `youaskm3` release-facing HTTP/JSON path, use [docs/youaskm3-canonical-app-http-path.md](youaskm3-canonical-app-http-path.md).

Use it when you want to prove, in one deterministic run, that:

- the runtime can execute the approved expedition request
- the local browser adapter exposes the governed subscription transport
- the browser app can render the live stream through the local proxy path
- the terminal trace is visible at the end of the streamed execution

## Canonical Command

Run the acceptance path with:

```bash
bash scripts/ci/app_consumable_acceptance.sh
```

That command delegates to the live browser-adapter demo smoke path, which already exercises the canonical app-consumable flow end to end.

## What It Validates

- the runtime starts and completes the approved expedition request path
- the local browser adapter serves the governed subscription stream
- the React browser demo renders the live ordered lifecycle, state, trace, and terminal evidence
- the final result reports the expected completed planning outcome

## Expected Failure Modes

The acceptance path fails deterministically when:

- the browser adapter cannot start or bind to the expected local port
- the browser app cannot proxy to the adapter
- the approved runtime request does not complete successfully
- the stream omits ordered lifecycle, state, trace, or terminal messages
- the adapter returns invalid setup or missing-stream responses

## Related Docs

- [quickstart.md](../quickstart.md)
- [https://github.com/traverse-framework/reference-apps/tree/main/apps/react-demo/README.md](../https://github.com/traverse-framework/reference-apps/tree/main/apps/react-demo/README.md)
- [docs/expedition-example-smoke.md](expedition-example-smoke.md)
