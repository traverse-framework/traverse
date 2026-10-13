"""Decision 112 spike: pre-passed ONNX -> PyTorch (onnx2torch) -> Core ML ML Program.

Usage: python convert.py <prepass.onnx> <out.mlpackage> <fp16|fp32>
"""

import sys
import warnings

import coremltools as ct
import numpy as np
import onnx
import torch
from onnx2torch import convert

from coremltools.converters.mil import Builder as mb
from coremltools.converters.mil.frontend.torch.ops import _get_inputs
from coremltools.converters.mil.frontend.torch.torch_op_registry import register_torch_op

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


def main():
    model = convert(onnx.load(sys.argv[1])).eval()
    example = torch.zeros(1, 144000)
    with torch.no_grad():
        traced = torch.jit.trace(model, example)
    mlmodel = ct.convert(
        traced,
        inputs=[ct.TensorType(name="input", shape=(1, 144000))],
        outputs=[ct.TensorType(name="output")],
        convert_to="mlprogram",
        minimum_deployment_target=ct.target.iOS17,
        compute_precision=ct.precision.FLOAT16 if sys.argv[3] == "fp16" else ct.precision.FLOAT32,
    )
    mlmodel.save(sys.argv[2])
    print("saved", sys.argv[2])


if __name__ == "__main__":
    main()
