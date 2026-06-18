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
Traverse owns provider selection and later execution. Product code must not
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

Provider availability checks and execution-time resolution are implemented in
later Spec 045 slices. This schema slice makes app manifests precise enough for
those runtime paths to be deterministic and testable.
