"""Decision 112 `mlpackage-zip`: canonical, deterministic zip of a .mlpackage.

1. `Data/com.apple.CoreML/model.mlmodel` is re-serialized with protobuf
   `deterministic=True` (coremltools writes map fields in unstable order).
2. `Manifest.json` item ids (random UUIDs) become UUIDv5 of the item path,
   written with sorted keys.
3. Entries sorted by path, stored (no compression), 1980-01-01 timestamps,
   mode 0644.

Usage: python canonical_zip.py <dir.mlpackage> <out.zip>
"""
import json
import os
import sys
import uuid
import zipfile

from coremltools.proto import Model_pb2

root, out = sys.argv[1], sys.argv[2]
NAMESPACE = uuid.uuid5(uuid.NAMESPACE_URL, "https://traverse-framework.github.io/mlpackage-zip")


def canonical_bytes(rel):
    data = open(os.path.join(root, rel), "rb").read()
    if rel.endswith("com.apple.CoreML/model.mlmodel"):
        spec = Model_pb2.Model()
        spec.ParseFromString(data)
        return spec.SerializeToString(deterministic=True)
    if rel == "Manifest.json":
        manifest = json.loads(data)
        ids = {old: str(uuid.uuid5(NAMESPACE, entry["path"])).upper()
               for old, entry in manifest["itemInfoEntries"].items()}
        manifest["itemInfoEntries"] = {ids[old]: entry for old, entry in manifest["itemInfoEntries"].items()}
        manifest["rootModelIdentifier"] = ids[manifest["rootModelIdentifier"]]
        return (json.dumps(manifest, indent=4, sort_keys=True) + "\n").encode()
    return data


files = sorted(os.path.relpath(os.path.join(d, f), root).replace(os.sep, "/")
               for d, _, fs in os.walk(root) for f in fs)
with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_STORED) as z:
    for rel in files:
        info = zipfile.ZipInfo(rel, date_time=(1980, 1, 1, 0, 0, 0))
        info.external_attr = 0o644 << 16
        info.create_system = 3
        z.writestr(info, canonical_bytes(rel))
