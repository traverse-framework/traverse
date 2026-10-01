//! Minimal, dependency-free ONNX (protobuf) writer for the digits MLP, so the
//! generic ONNX runner (#1591) has a permissively licensed conformance model
//! (Decision 105). Graph: `x[1,64] * 1/16 -> Gemm(W1,b1) -> Relu ->
//! Gemm(W2,b2) -> y[1,10]` (raw logits), weights from the committed MLP.

use crate::{CLASSES, FEATURE_SCALE, HIDDEN, INPUTS, Mlp};

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(u8::try_from(value & 0x7f).unwrap_or(0) | 0x80);
        value >>= 7;
    }
    out.push(u8::try_from(value).unwrap_or(0));
}

fn key(out: &mut Vec<u8>, field: u64, wire: u64) {
    varint(out, (field << 3) | wire);
}

fn int_field(out: &mut Vec<u8>, field: u64, value: u64) {
    key(out, field, 0);
    varint(out, value);
}

fn bytes_field(out: &mut Vec<u8>, field: u64, value: &[u8]) {
    key(out, field, 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}

fn tensor(name: &str, dims: &[u64], values: &[f32]) -> Vec<u8> {
    let mut out = Vec::new();
    for dim in dims {
        int_field(&mut out, 1, *dim);
    }
    int_field(&mut out, 2, 1); // data_type FLOAT
    bytes_field(&mut out, 8, name.as_bytes());
    let raw: Vec<u8> = values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    bytes_field(&mut out, 9, &raw);
    out
}

fn node(op: &str, inputs: &[&str], output: &str, trans_b: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for input in inputs {
        bytes_field(&mut out, 1, input.as_bytes());
    }
    bytes_field(&mut out, 2, output.as_bytes());
    bytes_field(&mut out, 3, output.as_bytes());
    bytes_field(&mut out, 4, op.as_bytes());
    if trans_b {
        let mut attribute = Vec::new();
        bytes_field(&mut attribute, 1, b"transB");
        int_field(&mut attribute, 3, 1);
        int_field(&mut attribute, 20, 2); // AttributeType INT
        bytes_field(&mut out, 5, &attribute);
    }
    out
}

fn value_info(name: &str, dims: &[u64]) -> Vec<u8> {
    let mut shape = Vec::new();
    for dim in dims {
        let mut dimension = Vec::new();
        int_field(&mut dimension, 1, *dim);
        bytes_field(&mut shape, 1, &dimension);
    }
    let mut tensor_type = Vec::new();
    int_field(&mut tensor_type, 1, 1); // elem_type FLOAT
    bytes_field(&mut tensor_type, 2, &shape);
    let mut type_proto = Vec::new();
    bytes_field(&mut type_proto, 1, &tensor_type);
    let mut out = Vec::new();
    bytes_field(&mut out, 1, name.as_bytes());
    bytes_field(&mut out, 2, &type_proto);
    out
}

/// Serialize the MLP as an ONNX model (opset 13, IR version 8).
#[must_use]
pub fn mlp_to_onnx(model: &Mlp) -> Vec<u8> {
    let w1: Vec<f32> = model.w1.iter().flatten().copied().collect();
    let w2: Vec<f32> = model.w2.iter().flatten().copied().collect();
    let mut graph = Vec::new();
    for bytes in [
        node("Mul", &["x", "scale"], "scaled", false),
        node("Gemm", &["scaled", "w1", "b1"], "hidden", true),
        node("Relu", &["hidden"], "activated", false),
        node("Gemm", &["activated", "w2", "b2"], "y", true),
    ] {
        bytes_field(&mut graph, 1, &bytes);
    }
    bytes_field(&mut graph, 2, b"traverse-digits-mlp");
    for bytes in [
        tensor("scale", &[1], &[FEATURE_SCALE]),
        tensor("w1", &[HIDDEN as u64, INPUTS as u64], &w1),
        tensor("b1", &[HIDDEN as u64], &model.b1),
        tensor("w2", &[CLASSES as u64, HIDDEN as u64], &w2),
        tensor("b2", &[CLASSES as u64], &model.b2),
    ] {
        bytes_field(&mut graph, 5, &bytes);
    }
    bytes_field(&mut graph, 11, &value_info("x", &[1, INPUTS as u64]));
    bytes_field(&mut graph, 12, &value_info("y", &[1, CLASSES as u64]));
    let mut opset = Vec::new();
    bytes_field(&mut opset, 1, b"");
    int_field(&mut opset, 2, 13);
    let mut out = Vec::new();
    int_field(&mut out, 1, 8); // ir_version
    bytes_field(&mut out, 2, b"traverse-model-trainer");
    bytes_field(&mut out, 7, &graph);
    bytes_field(&mut out, 8, &opset);
    out
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    #[test]
    fn committed_onnx_export_matches_the_committed_weights() {
        let weights = std::fs::read(format!(
            "{ROOT}/crates/traverse-digits-mlp-guest/weights/digits-mlp-1.0.0.bin"
        ))
        .expect("weights");
        let model = Mlp::from_le_bytes(&weights).expect("model");
        let committed =
            std::fs::read(format!("{ROOT}/fixtures/onnx/digits-mlp-1.0.0.onnx")).expect("onnx");
        assert!(
            mlp_to_onnx(&model) == committed,
            "fixtures/onnx/digits-mlp-1.0.0.onnx is stale: run `cargo run --release -p traverse-model-trainer -- export-onnx`"
        );
    }

    #[test]
    fn varints_use_seven_bit_groups() {
        let mut out = Vec::new();
        varint(&mut out, 300);
        assert_eq!(out, [0xac, 0x02]);
    }
}
