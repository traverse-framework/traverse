# youaskm3 Starter Kit and Integration Guide

This document is the first integration guide for browser-hosted downstream apps such as `youaskm3`.

It pairs with the reference starter kit at [https://github.com/traverse-framework/App-References/tree/main/apps/youaskm3-starter-kit/README.md](https://github.com/traverse-framework/App-References/tree/main/apps/youaskm3-starter-kit/README.md).

This youaskm3 starter kit and integration guide is the canonical browser-hosted adoption path for the downstream app.

## Goal

The goal is to give a downstream app team one clear adoption path for Traverse without forcing them to reverse-engineer the repo.

## What to Install

Downstream apps should adopt the versioned Traverse consumer bundle and the browser-targeted consumer package documented in:

- [docs/app-consumable-consumer-bundle.md](app-consumable-consumer-bundle.md)
- [https://github.com/traverse-framework/App-References/tree/main/apps/browser-consumer/README.md](https://github.com/traverse-framework/App-References/tree/main/apps/browser-consumer/README.md)

## Setup

1. Read the root [quickstart.md](../quickstart.md).
2. Review the app-consumable consumer bundle.
3. Use the browser-targeted consumer package from the starter kit.
4. Run the browser-hosted and MCP-facing validation paths.
5. Use the documented browser-hosted and MCP-facing validations to prove the release pairing.

## Supported Surfaces

The integration guide covers:

- the versioned Traverse consumer bundle
- the browser-targeted consumer package
- the browser-hosted live adapter validation path
- the MCP consumption validation path

## Validation

Run the documented repo-local commands:

```bash
bash scripts/ci/youaskm3_integration_validation.sh
bash scripts/ci/youaskm3_starter_kit_smoke.sh
```

## Known Limits

This guide does not promise:

- custom downstream app UX
- full auth hardening
- multi-tenant deployment guarantees
- direct Traverse source checkout coupling
