"""#1830 checks over downloaded files: LICENSE/NOTICE digests, sync fidelity,
the vendored-fixture shape, and deprecated predecessors naming their successors.
Run with `python3 -I compare.py <work> <repo>`."""
import hashlib, json, pathlib, sys, urllib.request

work, repo = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
proof = pathlib.Path(__file__).parent / "proof"
synced = json.loads((work / ".traverse/workspaces/rights-live/registry/public/index.json").read_text())
published = json.loads((work / "index.json").read_text())
pins = {"text.detect-entities": "1.2.1", "text.redact-entities": "1.2.1",
        "audio.detect-speech-segments": "1.2.1", "audio.transcribe-speech": "1.3.1",
        "report.translate-fr-semantic": "1.2.1"}
predecessors = {"text.detect-entities": "1.2.0", "text.redact-entities": "1.2.0",
                "audio.detect-speech-segments": "1.2.0", "audio.transcribe-speech": "1.3.0",
                "report.translate-fr-semantic": "1.2.0"}

rows = []
for record in synced["capabilities"]:
    if pins.get(record["id"]) != record["version"]:
        continue
    for model in record["ai"]["models"]:
        for kind in ("license_files", "notice_files"):
            for entry in model.get(kind, []):
                body = urllib.request.urlopen(entry["url"]).read()
                ok = hashlib.sha256(body).hexdigest() == entry["sha256"]
                rows.append("\t".join([record["id"], model["id"], kind, entry["url"].rsplit("/", 1)[1],
                                       str(len(body)), "verified" if ok else "MISMATCH"]))
(proof / "license-notice-digests.tsv").write_text("\n".join(sorted(rows)) + "\n")

key = lambda r: (r["namespace"], r["id"], r["version"])
P = {key(r): r for r in published["capabilities"]}
S = {key(r): r for r in synced["capabilities"]}
lines = [f"published records: {len(P)} synced records: {len(S)} same keys: {P.keys() == S.keys()}"]
for k in sorted(P):
    if k[1] in pins:
        p, s = P[k], S[k]
        changed = sorted(f for f in set(p) & set(s) if p[f] != s[f])
        lines.append(f"{k[1]}@{k[2]}: dropped={sorted(set(p) - set(s))} added={sorted(set(s) - set(p))} changed={changed}")
(proof / "sync-fidelity.txt").write_text("\n".join(lines) + "\n")

lines = []
for name, index in (("published", P), ("synced", S)):
    for rid, old in sorted(predecessors.items()):
        r = next(r for k, r in index.items() if k[1] == rid and k[2] == old)
        succ = r.get("superseded_by")
        target = next((t for k, t in index.items() if k[1] == rid and k[2] == succ), None)
        ok = (r.get("status") == "deprecated" and succ == pins[rid] and target is not None
              and target.get("status") == "active"
              and (target.get("provenance") or {}).get("source") == "ai-assisted")
        lines.append(f"{name} {rid}@{old}: status={r.get('status')} superseded_by={succ} "
                     f"successor_status={target and target.get('status')} "
                     f"successor_source={target and (target.get('provenance') or {}).get('source')} "
                     f"{'pass' if ok else 'FAIL'}")
(proof / "successors.txt").write_text("\n".join(lines) + "\n")

def model_refs(obj, out):
    if isinstance(obj, dict):
        if "spdx_expression" in obj and "id" in obj:
            out.append(obj)
        for value in obj.values():
            model_refs(value, out)
    elif isinstance(obj, list):
        for value in obj:
            model_refs(value, out)
    return out

def paths(obj, acc, path=""):
    if isinstance(obj, dict):
        for k, v in obj.items():
            acc.add(f"{path}.{k}")
            paths(v, acc, f"{path}.{k}")
    elif isinstance(obj, list):
        for v in obj:
            paths(v, acc, f"{path}[]")
    return acc

fixtures = model_refs(json.loads((repo / "crates/traverse-cli/registry-admission/ai_admission_corpus.json").read_text()), [])
live = [m for r in published["capabilities"] for m in (r.get("ai") or {}).get("models") or []
        if isinstance(m, dict) and "commercial_use" in m]
F, L = set(), set()
for m in fixtures:
    paths(m, F)
for m in live:
    paths(m, L)
(proof / "fixture-vs-live.txt").write_text(
    f"fixture model refs: {len(fixtures)} live rights model refs: {len(live)}\n"
    f"in live, never in fixtures: {sorted(L - F)}\n"
    f"in fixtures, never live: {sorted(F - L)}\n")
