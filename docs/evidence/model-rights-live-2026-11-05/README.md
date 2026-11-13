# Model rights against the live registry (2026-11-05)

Evidence for `#1786`. It checks Traverse's Spec 026 model-rights handling against the five agents the registry backfilled in registry#623, using only published packages and the live registry:

- `traverse-cli-rs` 0.16.2
- `traverse-runtime` 0.16.2
- `traverse-contracts` 0.16.2
- `traverse-registry` 0.31.0
- registry index `index-v416` (source commit `3984b73c`)

Run `bash run.sh` to regenerate `proof/` from an empty `.work/` folder.

> **Update (2026-11-12, `#1830`):** finding 1 is resolved. The registry republished the five agents as `ai-assisted` patch versions, and they now pass `app validate` and `app register` on exact pins. See [`../model-rights-reprove-2026-11-12/`](../model-rights-reprove-2026-11-12/).

## What was checked

The rights-bearing version of each agent is pinned exactly in `app/`:

| Agent | Version | Models |
|---|---|---|
| `text.detect-entities` | 1.2.0 | `dslim/distilbert-NER` |
| `text.redact-entities` | 1.2.0 | `dslim/distilbert-NER` |
| `audio.detect-speech-segments` | 1.2.0 | `snakers4/silero-vad` |
| `audio.transcribe-speech` | 1.3.0 | `openai/whisper-tiny`, `snakers4/silero-vad` |
| `report.translate-fr-semantic` | 1.2.0 | `sentence-transformers/paraphrase-MiniLM-L3-v2` |

## Results

| Check | Result | Proof |
|---|---|---|
| `registry sync` keeps the Spec 026 fields (Spec 055 FR-014) | Pass. `ai`, `status`, `model_usage` and `revocation` are unchanged for every version of the five agents. | `sync.json`, `sync-fidelity.txt` |
| `registry inspect` shows the rights record | Pass. Each record shows SPDX, the three rights, `usage_class`, LICENSE and NOTICE files with sha256, `derivation`, `data_obligations: not stated` and `verification_status`. | `inspect-*.json`, `inspect-*.txt` |
| `--model-commercial-use` filter | Pass. `allowed` selects exactly the backfilled version, and `conditional`, `forbidden` and `unknown` select nothing. `transcribe-speech` 1.2.0 has object-shaped models with no declared `commercial_use`, and it is correctly not matched. | `search/` |
| `--status` filter | Pass. `active` selects every version, and `deprecated` and `revoked` select none. | `search/` |
| Runtime rights decision | Pass. The published `check_registry_record_rights` permits every version under the default policy and with `allow_conditional`, matching the declared `allowed` rights. Legacy string-shaped versions are grandfathered, as specified. | `runtime-policy.json` |
| Record digests and signatures | Pass. `app prepare` verifies the contract digest, artifact digest and signature for all five, with `policy: permitted`. | `prepare.json` |
| LICENSE and NOTICE digests | Pass. All 13 referenced files match their declared sha256. | `license-notice-digests.tsv` |
| Consumption through `app validate` and `app register` | **Fail.** See finding 1. | `validate.json`, `register.json` |

## Findings

1. **Defect, filed as registry#656.** Every version of the five agents declares `provenance.source: "derived"`, and six other records declare `"registry-wave2"`. Traverse Spec 002 allows only `greenfield`, `brownfield-extracted`, `ai-generated` and `ai-assisted`. `app prepare` caches the records, then `app validate` and `app register` fail with `registry_reference_requires_resolution`, because the cached contract won't parse. With a Spec 002 value in that field, all five contracts parse (`runtime-policy.json`, `contract_parse_with_spec_002_source`). Until registry#656 is fixed, a Traverse app can't consume these agents.
2. **Observation, not a defect.** Sync drops `risk`, `risk_source` and `is_automatic_eligible` from every record and adds `revocation: null`. Spec 055 FR-014 only requires the Spec 026 fields to survive, and they do.
3. **No fixture mismatch.** Every field the live rights records use also appears in the vendored corpus (`crates/traverse-cli/registry-admission/ai_admission_corpus.json`), which also covers negative cases the live index doesn't have (`fixture-vs-live.txt`).

## What this does not establish

- **No live revoked version exists.** `index-v416` has 196 active and 36 deprecated records and no revoked ones, so the revoked path was not exercised live. Its fixture coverage is in `#1728`.
- **No live `conditional` or `unknown` rights exist.** All six live rights-bearing model references declare `allowed` for every right, so the denial paths were not exercised live.
- **The rights records carry no signature of their own.** Their `verification.status` is `maintainer-declared`. The signature checked is the registry record's artifact signature.
- **No production or legal compliance.** This checks that Traverse handles the published records as specified. It does not judge the declared rights.
- **No execution.** The agents were not run, because finding 1 blocks registration.
