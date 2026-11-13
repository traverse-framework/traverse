# Model rights re-proof after the Spec 002 provenance patches (2026-11-12)

Evidence for `#1830`. It re-runs `#1786`'s checks ([`../model-rights-live-2026-11-05/`](../model-rights-live-2026-11-05/)) on the five patch versions the registry published under Decision 130 (registry#656, registry#657). It uses only published packages and the live registry:

- `traverse-cli-rs` 0.16.2
- `traverse-runtime` 0.16.2
- `traverse-contracts` 0.16.2
- `traverse-registry` 0.31.0
- registry index `index-v427` (257 records)

These are the same package versions `#1786` used, so the only change is the registry records.

Run `bash run.sh` to regenerate `proof/` from an empty `.work/` folder.

## What was checked

Each patch version is pinned exactly in `app/`:

| Agent | Patch version | Deprecated predecessor |
|---|---|---|
| `text.detect-entities` | 1.2.1 | 1.2.0 |
| `text.redact-entities` | 1.2.1 | 1.2.0 |
| `audio.detect-speech-segments` | 1.2.1 | 1.2.0 |
| `audio.transcribe-speech` | 1.3.1 | 1.3.0 |
| `report.translate-fr-semantic` | 1.2.1 | 1.2.0 |

## Results

| Check | Result | Proof |
|---|---|---|
| Published `traverse-contracts` parses the patch contracts | Pass. All five declare `provenance.source: ai-assisted` and parse as published. | `runtime-policy.json` (`contract_parse_as_published`) |
| `registry sync` keeps the Spec 026 fields (Spec 055 FR-014) | Pass. `ai`, `status`, `model_usage` and `revocation` are unchanged for every version of the five agents. | `sync.json`, `sync-fidelity.txt` |
| `registry inspect` shows the rights record | Pass. Each patch version shows SPDX, the three rights, `usage_class`, LICENSE and NOTICE files with sha256, `derivation` and `verification_status`. | `inspect-*.json`, `inspect-*.txt` |
| `--model-commercial-use` filter | Pass. `allowed` selects every rights-bearing version, including each patch, and `conditional`, `forbidden` and `unknown` select nothing. | `search/` |
| `--status` filter | Pass. `active` selects each patch version and no predecessor it replaces. `deprecated` selects each predecessor. `revoked` selects nothing. | `search/` |
| Runtime rights decision | Pass. The published `check_registry_record_rights` permits every version under the default policy and with `allow_conditional`. | `runtime-policy.json` |
| Record digests and signatures | Pass. `app prepare` verifies the contract digest, artifact digest and signature for all five, with `policy: permitted`. | `prepare.json` |
| LICENSE and NOTICE digests | Pass. All 13 referenced files match their declared sha256. | `license-notice-digests.tsv` |
| `app validate` on the exact pins | Pass. `status: validated`. | `validate.json` |
| `app register` on the exact pins | Pass. `status: registered`. | `register.json` |
| Each deprecated predecessor shows as deprecated | Pass. `registry inspect`, `--status deprecated` and the runtime's lifecycle status all show `deprecated`. | `inspect-*.txt`, `search/`, `runtime-policy.json` |
| Each deprecated predecessor names its successor | **Pass in the published index, missing after sync.** See finding 1. | `successors.txt` |

`#1786`'s finding 1 (registry#656) is resolved: the agents it blocked now validate and register.

## Findings

1. **Gap, filed as `#1848`.** Each published predecessor declares `superseded_by` with its active, `ai-assisted` patch version. `registry sync` drops `superseded_by` from every record, and `registry inspect` doesn't show it. So a Traverse user sees that the predecessor is deprecated, but not which version replaces it. No approved spec requires sync to keep `superseded_by`, so this isn't a spec violation (`successors.txt`, `sync-fidelity.txt`).
2. **Observation, unchanged from `#1786`.** Sync also drops `risk`, `risk_source` and `is_automatic_eligible`, and adds `revocation: null`. Spec 055 FR-014 only requires the Spec 026 fields to survive, and they do.
3. **Probe app correction.** `#1786`'s `app/app.manifest.json` used the sha256 of each component manifest file as the component `digest`. For a registry-ref component, `app validate` compares that field with the resolved artifact's wasm digest, and fails with `component_digest_mismatch` otherwise. `#1786` never reached that check, because the contracts didn't parse. This app uses the artifact digests that `app prepare` verified.
4. **No fixture mismatch.** Every field the live rights records use also appears in the vendored corpus (`fixture-vs-live.txt`).

## What this does not establish

- **No live revoked version exists.** `--status revoked` selects nothing, so the revoked path was not exercised live.
- **No live `conditional` or `unknown` rights exist.** The denial paths were not exercised live.
- **The rights records carry no signature of their own.** Their `verification_status` is `maintainer-declared`. The signature checked is the registry record's artifact signature.
- **No production or legal compliance.** This checks that Traverse handles the published records as specified. It does not judge the declared rights.
- **No execution.** The agents were registered, not run.
