"""Find the first ONNX node whose Core ML (fp32, CPU) output diverges from PyTorch."""
import os
import sys
import warnings
from pathlib import Path

import coremltools as ct
import numpy as np
import onnx
import torch
from onnx import utils
from onnx2torch import convert

warnings.filterwarnings("ignore")
sys.argv = ["convert.py"]
exec(Path("convert.py").read_text().split("def main")[0])  # registers broadcast_to

model = onnx.load("birdnet.prepass.onnx")
nodes = [n for n in model.graph.node if n.output and n.op_type not in ("Constant",)]
clip = sorted(f for f in os.listdir("clips") if f.endswith(".f32le"))[0]
x = np.array(np.frombuffer(Path("clips", clip).read_bytes(), np.float32)).reshape(1, 144000)


def diverges(index):
    out = nodes[index].output[0]
    utils.extract_model("birdnet.prepass.onnx", "cut.onnx", ["input"], [out])
    m = convert(onnx.load("cut.onnx")).eval()
    with torch.no_grad():
        ref = m(torch.from_numpy(x)).numpy()
        traced = torch.jit.trace(m, torch.zeros(1, 144000))
    ml = ct.convert(traced, inputs=[ct.TensorType(name="input", shape=(1, 144000))],
                    convert_to="mlprogram", minimum_deployment_target=ct.target.iOS17,
                    compute_precision=ct.precision.FLOAT32)
    ml = ct.models.MLModel(ml.get_spec(), weights_dir=ml.weights_dir, compute_units=ct.ComputeUnit.CPU_ONLY)
    got = np.array(list(ml.predict({"input": x}).values())[0])
    if got.size != ref.size:
        print(f"node {index:3d} {nodes[index].op_type:12s} SHAPE coreml={got.shape} torch={ref.shape}", flush=True)
        return True
    got = got.reshape(ref.shape)
    err = float(np.max(np.abs(got - ref)) / (np.max(np.abs(ref)) + 1e-6))
    print(f"node {index:3d} {nodes[index].op_type:12s} rel={err:.2e} shape={ref.shape}", flush=True)
    return err > 1e-3


lo, hi = 0, len(nodes) - 1
while lo < hi:
    mid = (lo + hi) // 2
    if diverges(mid):
        hi = mid
    else:
        lo = mid + 1
n = nodes[lo]
print("FIRST DIVERGING:", lo, n.op_type, n.name, list(n.input), [(a.name, onnx.helper.get_attribute_value(a)) for a in n.attribute])
