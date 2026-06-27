# Governed Model Dependency Authoring Guide

Spec `045-governed-model-dependency-resolution` defines app manifest model
dependencies for real inference. These declarations belong in application
manifests, not inside downstream product code.

## Inference Interface

The first governed inference interface is:

```text
traverse.inference.generate
```

Downstream apps declare this abstract interface plus concrete model candidates.
Traverse owns provider selection and execution. Product code must not
hardcode Ollama, llama.cpp, WebLLM, cloud APIs, or provider-specific paths.

## App Manifest Shape

```json
{
  "model_dependencies": [
    {
      "interface_id": "traverse.inference.generate",
      "version_range": "^1.0",
      "selection_policy": {
        "strategy": "priority",
        "allow_fallback": true
      },
      "required_capabilities": ["text_generation"],
      "minimum_context_window": 8192,
      "candidates": [
        {
          "candidate_id": "ollama-llama-3-2",
          "provider_capability_id": "traverse.inference.generate",
          "provider_implementation_id": "ollama.local.generate",
          "model_identifier": "llama3.2:3b",
          "placement_target": "local",
          "priority": 10,
          "required_provider_config_keys": ["ollama_base_url"],
          "metadata": {
            "implementation_kind": "real_local_provider",
            "provider": "ollama",
            "model_context_window": 8192,
            "supports_streaming": true
          }
        }
      ]
    }
  ]
}
```

## Validation Rules

- `interface_id` must be `traverse.inference.generate`.
- `selection_policy.strategy` must be `priority`.
- `minimum_context_window` must be greater than zero.
- Each dependency must declare at least one candidate.
- Candidate ids must be unique within one dependency.
- Candidate provider ids, model identifier, config keys, priority, placement,
  and metadata must be concrete.
- Candidate metadata must be non-sensitive and safe for readiness/trace
  evidence.
- Fake, stub, placeholder, or documentation-only provider implementations do
  not satisfy Spec 045.

## Local Ollama Provider

The first concrete provider implementation is `ollama.local.generate` behind the
`traverse.inference.generate` interface. Workspace config supplies
`ollama_base_url`, usually `http://127.0.0.1:11434`; public readiness and trace
evidence must report the selected provider and model without exposing prompts or
secret config values.

The provider checks `/api/tags` before generation and invokes `/api/generate`
with `stream: false`. It reports stable machine-readable failures:

- `model_candidate_config_invalid` for invalid endpoint, prompt, model, or
  options config.
- `model_provider_unavailable` when the local Ollama endpoint cannot be reached.
- `model_candidate_unavailable` when the requested model is not installed.
- `model_provider_failure` when Ollama returns a non-success HTTP status.
- `model_provider_invalid_response` when Ollama returns malformed or incomplete
  JSON.

Runtime hosts execute app-declared model dependencies through Traverse's
governed model dependency surface. The runtime revalidates the selected
candidate at execution time, invokes the real provider implementation, and
returns public `ModelResolutionEvidence` alongside the inference output.
