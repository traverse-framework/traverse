"""Convert an ONNX model into a Spec 138 `coreml` variant (`mlpackage-zip`).

Pipeline (Decisions 112 and 113): deterministic pre-pass -> onnx2torch ->
torch.jit.trace -> coremltools ML Program (fp32, iOS 17) -> canonical zip.
Run it in the pinned environment from `requirements.lock`; the variant's
`conversion.tool_version` records that lockfile's digest.

Usage: python convert_variant.py <model.onnx> <out.mlpackage.zip>
Prints one JSON line: {source_digest, digest, lock_digest, rewrites}.
"""

import hashlib
import json
import sys
import tempfile
import warnings
from pathlib import Path

import coremltools as ct
import numpy as np
import onnx
import torch
from coremltools.converters.mil import Builder as mb
from coremltools.converters.mil.frontend.torch.ops import _get_inputs
from coremltools.converters.mil.frontend.torch.torch_op_registry import register_torch_op
from onnx2torch import convert

sys.path.insert(0, str(Path(__file__).parent))
import canonical_zip  # noqa: E402
from prepass import prepass  # noqa: E402

warnings.filterwarnings("ignore")
torch.manual_seed(0)


@register_torch_op
def broadcast_to(context, node):
    """aten::broadcast_to (emitted by onnx2torch's Min/Max): x + zeros(shape)."""
    x, shape = _get_inputs(context, node, expected=2)
    if shape.val is not None:
        dims = [int(d) for d in np.array(shape.val).reshape(-1)]
        if not dims:
            context.add(mb.identity(x=x, name=node.name))
            return
        target = np.array(dims, dtype=np.int32)
    else:
        target = mb.cast(x=shape, dtype="int32")
    zeros = mb.fill(shape=target, value=0.0)
    context.add(mb.add(x=x, y=zeros, name=node.name))


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    source, out = sys.argv[1], sys.argv[2]
    model = onnx.load(source)
    rewrites = prepass(model)
    graph_input = model.graph.input[0]
    shape = tuple(d.dim_value for d in graph_input.type.tensor_type.shape.dim)
    torch_model = convert(model).eval()
    with torch.no_grad():
        traced = torch.jit.trace(torch_model, torch.zeros(*shape))
    mlmodel = ct.convert(
        traced,
        inputs=[ct.TensorType(name=graph_input.name, shape=shape)],
        outputs=[ct.TensorType(name=model.graph.output[0].name)],
        convert_to="mlprogram",
        minimum_deployment_target=ct.target.iOS17,
        compute_precision=ct.precision.FLOAT32,
    )
    with tempfile.TemporaryDirectory() as scratch:
        package = Path(scratch, "model.mlpackage")
        mlmodel.save(str(package))
        canonical_zip.write(str(package), out)
    print(json.dumps({
        "source_digest": sha256(source),
        "digest": sha256(out),
        "lock_digest": sha256(Path(__file__).parent / "requirements.lock"),
        "rewrites": rewrites,
    }, sort_keys=True))


if __name__ == "__main__":
    main()
