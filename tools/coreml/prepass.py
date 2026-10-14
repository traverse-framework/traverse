"""Deterministic, semantics-preserving ONNX pre-pass for Core ML conversion
(Spec 138 `mlpackage-zip`, Decisions 112 and 113).

Each rewrite runs only where its construct appears:

1. Conv `auto_pad=SAME_UPPER` -> explicit `pads` (onnx2torch cannot convert
   it), computed from static shape inference.
2. A full-axis reverse `Slice` (step -1 from -1 to INT_MIN; TensorFlow's
   ReverseV2) -> `Gather` with reversed indices (coremltools drops one element
   when lowering that slice).
3. ONNX dynamic int8 matmul (`DynamicQuantizeLinear` -> `MatMulInteger` ->
   `Cast`) -> float ops that reproduce DynamicQuantizeLinear exactly, with
   clamps built from `Relu` (onnx2torch lowers Min/Max/Clip to broadcast_to on
   0-d tensors, which coremltools mistypes).

Dynamic input dimensions are fixed to 1: Core ML variants are converted for
the package's static frame shape.
"""

import numpy as np
from onnx import helper, numpy_helper, shape_inference


def fix_dynamic_dims(model):
    for value in model.graph.input:
        for dim in value.type.tensor_type.shape.dim:
            if not dim.HasField("dim_value"):
                dim.dim_value = 1


def static_shapes(model):
    inferred = shape_inference.infer_shapes(model)
    values = list(inferred.graph.value_info) + list(inferred.graph.input)
    return {
        vi.name: [d.dim_value if d.HasField("dim_value") else None for d in vi.type.tensor_type.shape.dim]
        for vi in values
    }


def fix_same_upper(model, shapes, inits):
    fixed = 0
    for node in model.graph.node:
        attrs = {a.name: a for a in node.attribute}
        if node.op_type != "Conv" or attrs.get("auto_pad") is None or attrs["auto_pad"].s != b"SAME_UPPER":
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
    return fixed


def fix_reverse_slices(model, shapes, inits):
    rewritten = 0
    for node in list(model.graph.node):
        if node.op_type != "Slice" or len(node.input) < 5:
            continue
        starts, ends, axes, steps = (inits.get(name) for name in node.input[1:5])
        if any(v is None for v in (starts, ends, axes, steps)):
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
    return rewritten


def rewrite_dynamic_matmuls(model):
    graph = model.graph
    rewritten = 0
    for dql in [n for n in graph.node if n.op_type == "DynamicQuantizeLinear"]:
        mmi = next(n for n in graph.node if n.op_type == "MatMulInteger" and n.input[0] == dql.output[0])
        cast = next(n for n in graph.node if n.op_type == "Cast" and n.input[0] == mmi.output[0])
        inits = {i.name: i for i in graph.initializer}
        x = dql.input[0]
        _, scale_name, zp_name = dql.output
        weights = numpy_helper.to_array(inits[mmi.input[1]]).astype(np.float32)
        w_zp = numpy_helper.to_array(inits[mmi.input[3]]).astype(np.float32)
        p = f"coreml_dql{rewritten}_"
        graph.initializer.append(numpy_helper.from_array(weights - w_zp, p + "w_centred"))
        graph.initializer.append(numpy_helper.from_array(np.array([255.0], dtype=np.float32), p + "255"))

        def relu(src, dst):
            return helper.make_node("Relu", [src], [dst])

        def neg(src, dst):
            return helper.make_node("Neg", [src], [dst])

        def clamp_0_255(src, dst):
            # min(max(v, 0), 255) == 255 - relu(255 - relu(v)), with no Min/Max ops.
            return [
                relu(src, dst + "_pos"),
                helper.make_node("Sub", [p + "255", dst + "_pos"], [dst + "_gap"]),
                relu(dst + "_gap", dst + "_gap_pos"),
                helper.make_node("Sub", [p + "255", dst + "_gap_pos"], [dst]),
            ]

        nodes = [
            helper.make_node("ReduceMin", [x], [p + "min"], keepdims=1),
            helper.make_node("ReduceMax", [x], [p + "max"], keepdims=1),
            neg(p + "min", p + "min_neg"),
            relu(p + "min_neg", p + "min_neg_pos"),
            neg(p + "min_neg_pos", p + "lo"),
            relu(p + "max", p + "hi"),
            helper.make_node("Sub", [p + "hi", p + "lo"], [p + "range"]),
            helper.make_node("Div", [p + "range", p + "255"], [scale_name]),
            helper.make_node("Div", [p + "lo", scale_name], [p + "lo_scaled"]),
            neg(p + "lo_scaled", p + "zp_raw"),
            helper.make_node("Round", [p + "zp_raw"], [p + "zp_round"]),
            *clamp_0_255(p + "zp_round", zp_name + "_f"),
            helper.make_node("Div", [x, scale_name], [p + "x_scaled"]),
            helper.make_node("Round", [p + "x_scaled"], [p + "x_round"]),
            helper.make_node("Add", [p + "x_round", zp_name + "_f"], [p + "x_shift"]),
            *clamp_0_255(p + "x_shift", p + "q"),
            helper.make_node("Sub", [p + "q", zp_name + "_f"], [p + "q_centred"]),
            helper.make_node("MatMul", [p + "q_centred", p + "w_centred"], [cast.output[0]]),
        ]
        index = list(graph.node).index(dql)
        for node in (dql, mmi, cast):
            graph.node.remove(node)
        for offset, node in enumerate(nodes):
            graph.node.insert(index + offset, node)
        rewritten += 1
    return rewritten


def prepass(model):
    """Rewrite `model` in place; returns a summary of what changed."""
    fix_dynamic_dims(model)
    shapes = static_shapes(model)
    inits = {i.name: numpy_helper.to_array(i) for i in model.graph.initializer}
    summary = {
        "explicit_pads": fix_same_upper(model, shapes, inits),
        "reverse_slices": fix_reverse_slices(model, shapes, inits),
        "dynamic_int8_matmuls": rewrite_dynamic_matmuls(model),
    }
    return summary
