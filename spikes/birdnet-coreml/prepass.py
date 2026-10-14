"""Deterministic ONNX pre-pass for the Decision 112 Core ML conversion spike.

Rewrites two constructs onnx2torch cannot convert, without changing the
model's semantics:

1. Conv `auto_pad=SAME_UPPER` becomes explicit `pads`, computed from the
   statically inferred input shapes (BirdNET's input length is fixed at
   144000 samples).
2. A full-axis reverse `Slice` (step -1 from -1 to INT_MIN, TensorFlow's
   ReverseV2) becomes `Gather` with reversed indices: coremltools drops one
   element when lowering that slice.
3. ONNX dynamic int8 matmul (`DynamicQuantizeLinear` -> `MatMulInteger` ->
   `Cast`) becomes float ops that reproduce DynamicQuantizeLinear exactly:
   x_min = min(0, min(x)), x_max = max(0, max(x)), scale = (x_max - x_min) / 255,
   zp = min(max(round(-x_min / scale), 0), 255), q = min(max(round(x / scale) + zp, 0), 255)
   (Max/Min instead of Clip, which onnx2torch lowers to an op coremltools lacks),
   then (q - zp) @ (W - w_zp) in float. The existing scale Mul is kept.

Usage: python prepass.py <in.onnx> <out.onnx>
"""

import sys

import numpy as np
import onnx
from onnx import helper, numpy_helper, shape_inference


def static_shape(value_info):
    dims = value_info.type.tensor_type.shape.dim
    return [d.dim_value if d.HasField("dim_value") else None for d in dims]


def fix_same_upper(model):
    model.graph.input[0].type.tensor_type.shape.dim[0].dim_value = 1
    inferred = shape_inference.infer_shapes(model)
    shapes = {vi.name: static_shape(vi) for vi in list(inferred.graph.value_info) + list(inferred.graph.input)}
    inits = {i.name: numpy_helper.to_array(i) for i in model.graph.initializer}
    fixed = 0
    for node in model.graph.node:
        if node.op_type != "Conv":
            continue
        attrs = {a.name: a for a in node.attribute}
        if "auto_pad" not in attrs or attrs["auto_pad"].s != b"SAME_UPPER":
            continue
        in_shape = shapes[node.input[0]]
        kernel = list(inits[node.input[1]].shape[2:])
        rank = len(kernel)
        strides = list(attrs["strides"].ints) if "strides" in attrs else [1] * rank
        dilations = list(attrs["dilations"].ints) if "dilations" in attrs else [1] * rank
        begin, end = [], []
        for axis in range(rank):
            size = in_shape[2 + axis]
            if size is None:
                raise SystemExit(f"{node.name}: dynamic spatial dim, cannot make pads explicit")
            out = -(-size // strides[axis])
            effective = (kernel[axis] - 1) * dilations[axis] + 1
            total = max(0, (out - 1) * strides[axis] + effective - size)
            begin.append(total // 2)
            end.append(total - total // 2)
        node.attribute.remove(attrs["auto_pad"])
        node.attribute.append(helper.make_attribute("pads", begin + end))
        fixed += 1
    model.graph.input[0].type.tensor_type.shape.dim[0].dim_param = "batch"
    return fixed


def fix_reverse_slices(model):
    model.graph.input[0].type.tensor_type.shape.dim[0].dim_value = 1
    inferred = shape_inference.infer_shapes(model)
    shapes = {vi.name: static_shape(vi) for vi in inferred.graph.value_info}
    inits = {i.name: numpy_helper.to_array(i) for i in model.graph.initializer}
    rewritten = 0
    for node in list(model.graph.node):
        if node.op_type != "Slice" or len(node.input) < 5:
            continue
        starts, ends, axes, steps = (inits.get(name) for name in node.input[1:5])
        if steps is None or starts is None or axes is None or ends is None:
            continue
        if steps.tolist() != [-1] or starts.tolist() != [-1] or ends.tolist()[0] > -(2**62):
            continue
        axis = int(axes.tolist()[0])
        size = shapes[node.input[0]][axis]
        if not size:
            raise SystemExit(f"{node.name}: reverse slice over a dynamic axis")
        indices = numpy_helper.from_array(np.arange(size - 1, -1, -1, dtype=np.int64), node.name + "_reverse_indices")
        model.graph.initializer.append(indices)
        gather = helper.make_node("Gather", [node.input[0], indices.name], list(node.output), axis=axis, name=node.name + "_as_gather")
        position = list(model.graph.node).index(node)
        model.graph.node.remove(node)
        model.graph.node.insert(position, gather)
        rewritten += 1
    model.graph.input[0].type.tensor_type.shape.dim[0].dim_param = "batch"
    return rewritten


def const(name, value):
    return numpy_helper.from_array(np.array(value, dtype=np.float32), name)


def rewrite_dynamic_matmul(model):
    graph = model.graph
    dql = next(n for n in graph.node if n.op_type == "DynamicQuantizeLinear")
    mmi = next(n for n in graph.node if n.op_type == "MatMulInteger")
    cast = next(n for n in graph.node if n.op_type == "Cast" and n.input[0] == mmi.output[0])
    inits = {i.name: i for i in graph.initializer}
    x = dql.input[0]
    q_name, scale_name, zp_name = dql.output
    weights = numpy_helper.to_array(inits[mmi.input[1]]).astype(np.float32)
    w_zp = numpy_helper.to_array(inits[mmi.input[3]]).astype(np.float32)
    centred = numpy_helper.from_array(weights - w_zp, "spike_w_centred")
    graph.initializer.append(centred)
    graph.initializer.append(const("spike_255", [255.0]))
    p = "spike_dql_"

    def relu(src, dst):
        return helper.make_node("Relu", [src], [dst])

    def neg(src, dst):
        return helper.make_node("Neg", [src], [dst])

    def clamp_0_255(src, dst):
        # min(max(v, 0), 255) == 255 - relu(255 - relu(v)), with no Min/Max ops.
        return [
            relu(src, dst + "_pos"),
            helper.make_node("Sub", ["spike_255", dst + "_pos"], [dst + "_gap"]),
            relu(dst + "_gap", dst + "_gap_pos"),
            helper.make_node("Sub", ["spike_255", dst + "_gap_pos"], [dst]),
        ]

    nodes = [
        helper.make_node("ReduceMin", [x], [p + "min"], keepdims=1),
        helper.make_node("ReduceMax", [x], [p + "max"], keepdims=1),
        # lo = min(x_min, 0) = -relu(-x_min); hi = max(x_max, 0) = relu(x_max)
        neg(p + "min", p + "min_neg"),
        relu(p + "min_neg", p + "min_neg_pos"),
        neg(p + "min_neg_pos", p + "lo"),
        relu(p + "max", p + "hi"),
        helper.make_node("Sub", [p + "hi", p + "lo"], [p + "range"]),
        helper.make_node("Div", [p + "range", "spike_255"], [scale_name]),
        helper.make_node("Div", [p + "lo", scale_name], [p + "lo_scaled"]),
        neg(p + "lo_scaled", p + "zp_raw"),
        helper.make_node("Round", [p + "zp_raw"], [p + "zp_round"]),
        *clamp_0_255(p + "zp_round", zp_name + "_f"),
        helper.make_node("Div", [x, scale_name], [p + "x_scaled"]),
        helper.make_node("Round", [p + "x_scaled"], [p + "x_round"]),
        helper.make_node("Add", [p + "x_round", zp_name + "_f"], [p + "x_shift"]),
        *clamp_0_255(p + "x_shift", p + "q"),
        helper.make_node("Sub", [p + "q", zp_name + "_f"], [p + "q_centred"]),
        helper.make_node("MatMul", [p + "q_centred", "spike_w_centred"], [cast.output[0]]),
    ]
    index = list(graph.node).index(dql)
    for node in (dql, mmi, cast):
        graph.node.remove(node)
    for offset, node in enumerate(nodes):
        graph.node.insert(index + offset, node)


def main():
    model = onnx.load(sys.argv[1])
    fixed = fix_same_upper(model)
    reversed_slices = fix_reverse_slices(model)
    rewrite_dynamic_matmul(model)
    onnx.checker.check_model(model)
    onnx.save(model, sys.argv[2])
    print(f"explicit pads on {fixed} convs; {reversed_slices} reverse slices -> Gather; dynamic int8 matmul rewritten to float")


if __name__ == "__main__":
    main()
