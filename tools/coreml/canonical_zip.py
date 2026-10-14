"""Spec 138 `mlpackage-zip`: the canonical, deterministic zip of a .mlpackage.

1. `Data/com.apple.CoreML/model.mlmodel` is re-serialized with protobuf
   `deterministic=True` (coremltools writes map fields in unstable order).
2. `Manifest.json` item ids (random UUIDs) become UUIDv5 of the item path,
   written with sorted keys.
3. Entries are sorted by path, stored (no compression), timestamped
   1980-01-01 00:00:00, mode 0644.
"""

import json
import os
import uuid
import zipfile
from pathlib import Path

from coremltools.proto import Model_pb2

NAMESPACE = uuid.uuid5(uuid.NAMESPACE_URL, "https://traverse-framework.github.io/mlpackage-zip")


def _canonical_bytes(root, rel):
    data = Path(root, rel).read_bytes()
    if rel.endswith("com.apple.CoreML/model.mlmodel"):
        spec = Model_pb2.Model()
        spec.ParseFromString(data)
        return spec.SerializeToString(deterministic=True)
    if rel == "Manifest.json":
        manifest = json.loads(data)
        ids = {old: str(uuid.uuid5(NAMESPACE, entry["path"])).upper() for old, entry in manifest["itemInfoEntries"].items()}
        manifest["itemInfoEntries"] = {ids[old]: entry for old, entry in manifest["itemInfoEntries"].items()}
        manifest["rootModelIdentifier"] = ids[manifest["rootModelIdentifier"]]
        return (json.dumps(manifest, indent=4, sort_keys=True) + "\n").encode()
    return data


def write(root, out):
    files = sorted(
        os.path.relpath(os.path.join(directory, name), root).replace(os.sep, "/")
        for directory, _, names in os.walk(root)
        for name in names
    )
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_STORED) as archive:
        for rel in files:
            info = zipfile.ZipInfo(rel, date_time=(1980, 1, 1, 0, 0, 0))
            info.external_attr = 0o644 << 16
            info.create_system = 3
            archive.writestr(info, _canonical_bytes(root, rel))
