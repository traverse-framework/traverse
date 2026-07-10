# Security Policy

## Supported Stage

Traverse is currently in early foundation setup and should be treated as pre-production software.

## Reporting a Vulnerability

**Please do not open public GitHub issues for suspected security vulnerabilities.**

Report them privately through GitHub's private vulnerability reporting for this
repository:

1. Go to the repository's **Security** tab:
   <https://github.com/traverse-framework/traverse/security>
2. Choose **Report a vulnerability** to open a private advisory visible only to
   the maintainers.

If you cannot use private advisories, follow the private reporting expectations
in [SUPPORT.md](SUPPORT.md) and clearly mark the report as a security issue.

Please do not disclose the issue publicly until a fix has been released and a
coordinated disclosure timeline has been agreed with the maintainers.

## What to Include

Please include:

- affected area (CLI, HTTP API, MCP, WASM execution, registry, federation, supply chain)
- reproduction steps
- impact and affected versions
- any known mitigation

## Scope and Trust Boundaries

The system's trust boundaries and the controls protecting each are documented in
the [Threat Model](docs/threat-model.md). Reports that identify a bypass of one
of those documented controls are especially valuable.

## Security Direction

The project is designed toward:

- validated and versioned artifacts
- provenance-aware workflows
- explicit contract boundaries
- runtime traceability
- controlled exceptions for unsafe or privileged behavior

This policy will evolve as the project matures.
