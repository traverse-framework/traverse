# Next Release Notes

## Persist resolved app state machines for `serve`

`traverse-cli serve` now dispatches Registry-backed app commands from the
persisted `state_machine` in workspace registration state. It does not reopen
or re-resolve the source application manifest. Legacy registrations without
that declaration require an explicit `app register` refresh;
`app_registration_requires_refresh` and `503 app_unavailable` replace silent
`404 app_not_registered` for registered apps that cannot materialize.

## Verified Registry application references

The next Traverse release after `v0.10.0` includes the verified Registry
application-reference lifecycle.

- A host prepares an active, signed Registry release and commits its verified
  contract and WASM bytes under immutable digest keys.
- `app validate`, `app register`, and `app activate` consume prepared evidence
  only; they do not fetch from the Registry, replace an exact version, or
  substitute a local component path.
- Cache integrity failures are redacted. Operators receive stable lifecycle
  outcomes rather than endpoints, credentials, headers, cache paths, or raw
  artifact bytes.

The minimum supported Traverse version is this next release; `v0.10.0` and
earlier do not provide the complete preparation-to-offline-activation path.

### Downstream proof

The checked-in Callweave `inference.evidence-normalize@1.0.1` signed-release
fixture proves exact-version validation, registration, and local activation
using prepared cache bytes. Its fixture contains no credential or private
endpoint requirement.
