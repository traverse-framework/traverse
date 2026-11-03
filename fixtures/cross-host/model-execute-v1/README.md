# Catalog `model.execute` guest

`model-execute-agent.wasm` is the published `model/model.execute@1.0.0` artifact,
unchanged, from the `artifacts/model.execute-1.0.0` release of
`traverse-framework/registry`.

sha256: `5ccfa3721db61b42a598d703c2a110c57beaf57cde26e1c5cb7f9c5fbc8b4275`

Hosts run it over `fixtures/models/digits-onnx-1.0.0` to prove the capability
works without an app-side connector shim (#1719).

`fail-closed.json` holds the failures every host must report the same way
(#1719): the registration's reason, and the capability's output after it. Each
case sets up the host with one of `trusted: false`, `pin_digest`, `tamper_wasm`
(flip the last byte of `model.wasm`) or `revoke` (after registration).
