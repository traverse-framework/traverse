# Repo Migration Status

The repository now lives at:

```text
https://github.com/traverse-framework/traverse
```

GitHub redirects the previous cased URL:

```text
https://github.com/traverse-framework/Traverse
```

to the current repository.

## Completed

- Repository ownership moved to `traverse-framework`.
- Repository name normalized to lowercase `traverse`.
- README badges and clone URL point at `traverse-framework/traverse`.
- Workspace package metadata points at `traverse-framework/traverse`.
- Agent coordination examples use `traverse-framework/traverse`.
- Project state audit uses `traverse-framework/traverse`.

## Post-Rename Validation

- `gh repo view traverse-framework/traverse` succeeds.
- `gh repo view traverse-framework/Traverse` resolves to `traverse-framework/traverse`.
- Existing GitHub redirects preserve old links.

## Local Clone Update

Existing local clones can move their remote to the canonical URL with:

```bash
git remote set-url origin https://github.com/traverse-framework/traverse.git
```
