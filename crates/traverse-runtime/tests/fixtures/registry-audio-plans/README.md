# Released registry audio-plan WASI fixtures

Pinned copies of the released `artifacts/core.*-1.0.0` GitHub release assets from
[`traverse-framework/registry`](https://github.com/traverse-framework/registry)
used by `registry_audio_plan_artifact_router.rs`.

| Asset | Digest (`sha256:`) |
| --- | --- |
| `core-create-audio-capture-request-plan.wasm` | `10fcd7c6e53d0f16cf396b2601864c85436d686e79dee4ae8ffec50b083a6baf` |
| `core-create-audio-transform-plan.wasm` | `b706c69e1c945efa12e968479a6959dc551b7dba2b376bb48f5bb0f4fddacb60` |
| `core-create-audio-window-plan.wasm` | `ecc7aba24eee918106999ab89cbcacda212502aae32f8692d35b00565a95dca4` |
| `core-validate-audio-source-profile.wasm` | `ad440e4782bc9418c59d422e4a34d3524130978a4bef049212a2ad346cd8775d` |

Set `TRAVERSE_FETCH_REGISTRY_ARTIFACTS=1` to re-download these assets from the
release URLs and re-verify digests before executing through `ArtifactRouter`.
