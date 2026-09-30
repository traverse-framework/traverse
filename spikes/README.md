# Spikes

Time-boxed feasibility experiments. **Not product code:** spikes are not
workspace members, are never published or shipped, and sit outside the
audited `unsafe` boundary (`scripts/ci/scoped_unsafe_boundary_check.sh`
scans `crates/` only). Anything a spike proves is re-implemented properly
under `crates/` with the normal specs, tests, and audits.

| Spike | Report |
|---|---|
| `birdnet-onnx/` (#1590) | `docs/spikes/1590-birdnet-onnx-wasm.md` |

Third-party model weights used by a spike (for example BirdNET, CC BY-NC-SA
4.0) are read from a local path at build time and **must never be
committed**.
