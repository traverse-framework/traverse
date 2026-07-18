# Traverse Planning Board

This document is the local planning view for MVP work and mirrors the active backlog in GitHub Project 1.

Status meanings:

- `Ready`: can be implemented now under approved specs and current repo rules
- `In Progress`: currently being worked on in an active issue or pull request
- `Blocked`: should not start yet; the item must state why it is blocked
- `Needs Spec`: implementation must not start because the governing slice is not approved yet
- `Needs Enrico`: blocked on product or governance direction from Enrico
- `No Spec Needed`: the work is artifact authoring, docs, or another task already fully governed by approved specs
- `Future`: valid MVP-following work that is tracked but not part of the current active slice

## Active Backlog

### `In Progress`

- [#66](https://github.com/traverse-framework/traverse/issues/66) `Codify MVP backlog completeness and ticket-quality enforcement`
  - area: `quality`, `documentation`
  - status: active in PR [#67](https://github.com/traverse-framework/traverse/pull/67)
  - done when: the backlog standards, templates, planning board, and repo checks land on `main`

- [#158](https://github.com/traverse-framework/traverse/issues/158) `Implement MCP stdio server package foundation`
  - area: `runtime`, `mcp`
  - status: active implementation work
  - done when: the dedicated stdio server boots deterministically, emits machine-readable startup/shutdown envelopes, and passes the new smoke path

Only tickets with real active execution should appear in this section.

### `Ready` + `No Spec Needed`

- [#42](https://github.com/traverse-framework/traverse/issues/42) `Author expedition event contract files`
  - area: `contracts`
  - why ready: governed by `003`, `008`, and `009`
  - done when: canonical event contract artifacts exist and validate under the approved example domain

- [#43](https://github.com/traverse-framework/traverse/issues/43) `Author workflow-backed composed capability contract for plan-expedition`
  - area: `contracts`, `workflow`
  - why ready: governed by `002`, `007`, `008`, and `009`
  - done when: the composed capability contract for `plan-expedition` is authored and valid

- [#44](https://github.com/traverse-framework/traverse/issues/44) `Author expedition atomic capability contract files`
  - area: `contracts`
  - why ready: governed by `002`, `008`, and `009`
  - done when: all five atomic expedition capability contracts are authored and valid

- [#45](https://github.com/traverse-framework/traverse/issues/45) `Author plan-expedition workflow definition artifact`
  - area: `workflow`
  - why ready: governed by `007`, `008`, and `009`
  - done when: the canonical workflow artifact is authored and validates against the approved workflow shape

### `Blocked` + `No Spec Needed`

- [#46](https://github.com/traverse-framework/traverse/issues/46) `Seed expedition example registry bundle and CLI walkthrough`
  - blocked by: example contracts and workflow artifacts are not authored yet
  - unblock path: complete [#42](https://github.com/traverse-framework/traverse/issues/42), [#43](https://github.com/traverse-framework/traverse/issues/43), [#44](https://github.com/traverse-framework/traverse/issues/44), and [#45](https://github.com/traverse-framework/traverse/issues/45)

- [#47](https://github.com/traverse-framework/traverse/issues/47) `Document expedition example authoring and validation walkthrough`
  - blocked by: the first concrete example artifacts and smoke path are not finished yet
  - unblock path: complete [#42](https://github.com/traverse-framework/traverse/issues/42), [#43](https://github.com/traverse-framework/traverse/issues/43), [#44](https://github.com/traverse-framework/traverse/issues/44), [#45](https://github.com/traverse-framework/traverse/issues/45), and [#48](https://github.com/traverse-framework/traverse/issues/48)

- [#48](https://github.com/traverse-framework/traverse/issues/48) `Add example artifact validation smoke path`
  - blocked by: the example artifact set is not complete yet
  - unblock path: complete [#42](https://github.com/traverse-framework/traverse/issues/42), [#43](https://github.com/traverse-framework/traverse/issues/43), [#44](https://github.com/traverse-framework/traverse/issues/44), and [#45](https://github.com/traverse-framework/traverse/issues/45)

## Future MVP Backlog

### `Needs Spec`

- [#35](https://github.com/traverse-framework/traverse/issues/35) `Future: specify placement abstraction beyond local execution`
- [#36](https://github.com/traverse-framework/traverse/issues/36) `Future: specify event-driven composition slice`
- [#37](https://github.com/traverse-framework/traverse/issues/37) `Future: specify metadata graph model`
- [#38](https://github.com/traverse-framework/traverse/issues/38) `Future: specify browser runtime subscription surface`
- [#39](https://github.com/traverse-framework/traverse/issues/39) `Future: specify trace artifact slice`
- [#40](https://github.com/traverse-framework/traverse/issues/40) `Future: specify MCP surface`
- [#41](https://github.com/traverse-framework/traverse/issues/41) `Future: specify runtime state machine slice`
- [#49](https://github.com/traverse-framework/traverse/issues/49) `Future: specify AI agent execution and WASM agent packaging slice`
- [#50](https://github.com/traverse-framework/traverse/issues/50) `Future: specify macOS demo app slice`
- [#51](https://github.com/traverse-framework/traverse/issues/51) `Future: specify Android demo app slice`
- [#52](https://github.com/traverse-framework/traverse/issues/52) `Future: specify event registry slice`

### `Blocked`

- [#53](https://github.com/traverse-framework/traverse/issues/53) `Future: implement second WASM AI agent example`
  - blocked by: [#49](https://github.com/traverse-framework/traverse/issues/49), [#40](https://github.com/traverse-framework/traverse/issues/40), and [#54](https://github.com/traverse-framework/traverse/issues/54)

- [#54](https://github.com/traverse-framework/traverse/issues/54) `Future: implement first WASM AI agent example`
  - blocked by: [#49](https://github.com/traverse-framework/traverse/issues/49) and [#40](https://github.com/traverse-framework/traverse/issues/40)

- [#55](https://github.com/traverse-framework/traverse/issues/55) `Future: implement React browser demo app`
  - blocked by: [#38](https://github.com/traverse-framework/traverse/issues/38) and the expedition example artifacts becoming runnable

- [#56](https://github.com/traverse-framework/traverse/issues/56) `Future: implement event registry foundation`
  - blocked by: [#52](https://github.com/traverse-framework/traverse/issues/52)

- [#57](https://github.com/traverse-framework/traverse/issues/57) `Future: implement Android demo app`
  - blocked by: [#51](https://github.com/traverse-framework/traverse/issues/51)

- [#58](https://github.com/traverse-framework/traverse/issues/58) `Future: implement MCP surface`
  - blocked by: [#40](https://github.com/traverse-framework/traverse/issues/40)

- [#59](https://github.com/traverse-framework/traverse/issues/59) `Future: implement macOS demo app`
  - blocked by: [#50](https://github.com/traverse-framework/traverse/issues/50)

- [#60](https://github.com/traverse-framework/traverse/issues/60) `Future: implement runtime state machine`
  - blocked by: [#41](https://github.com/traverse-framework/traverse/issues/41)

- [#61](https://github.com/traverse-framework/traverse/issues/61) `Future: implement browser runtime subscription surface`
  - blocked by: [#38](https://github.com/traverse-framework/traverse/issues/38)

- [#62](https://github.com/traverse-framework/traverse/issues/62) `Future: implement metadata graph projection`
  - blocked by: [#37](https://github.com/traverse-framework/traverse/issues/37)

- [#63](https://github.com/traverse-framework/traverse/issues/63) `Future: implement trace artifacts`
  - blocked by: [#39](https://github.com/traverse-framework/traverse/issues/39)

- [#64](https://github.com/traverse-framework/traverse/issues/64) `Future: implement placement abstraction beyond local executor`
  - blocked by: [#35](https://github.com/traverse-framework/traverse/issues/35)

- [#65](https://github.com/traverse-framework/traverse/issues/65) `Future: implement event-driven composition in runtime`
  - blocked by: [#36](https://github.com/traverse-framework/traverse/issues/36) and [#52](https://github.com/traverse-framework/traverse/issues/52)

## Quality Rules

- Every active ticket must have:
  - a clear summary
  - explicit dependencies
  - a blocker note if blocked
  - a Definition of Done with no ambiguity
  - exact validation steps

- If a problem is required to make the current slice correct, governed, or mergeable, it must be fixed in the active PR.
- If a problem is valid but not required for the active slice, it must become a `future` ticket instead of silently disappearing.

## Recommended Next Sequence

1. Complete [#42](https://github.com/traverse-framework/traverse/issues/42), [#43](https://github.com/traverse-framework/traverse/issues/43), [#44](https://github.com/traverse-framework/traverse/issues/44), and [#45](https://github.com/traverse-framework/traverse/issues/45)
2. Unblock and complete [#48](https://github.com/traverse-framework/traverse/issues/48)
3. Unblock and complete [#46](https://github.com/traverse-framework/traverse/issues/46)
4. Unblock and complete [#47](https://github.com/traverse-framework/traverse/issues/47)
5. Then choose the next future spec slice based on MVP priority

## Project 1

This planning board is mirrored into:

- [GitHub Project 1](https://github.com/orgs/traverse-framework/projects/1/)
