//! Safe model-blob parsing, tensor-config validation, frames, and tract run.
#![forbid(unsafe_code)]

use serde::Deserialize;
use std::collections::BTreeMap;
use tract_onnx::prelude::*;

/// Model blob magic (8 bytes) written by `model package-onnx`.
pub const BLOB_MAGIC: &[u8; 8] = b"TVONNX01";
/// Spec 138 guest frame version (unchanged by guest ABI v2).
pub const FRAME_VERSION: u16 = 1;

/// Tensor config fixed at package time (one input, one output, `f32`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TensorConfig {
    /// The ONNX graph's single input tensor name.
    pub input_name: String,
    /// The ONNX graph's single output tensor name.
    pub output_name: String,
    /// Exact input tensor shape (after symbol concretization).
    pub input_shape: Vec<usize>,
    /// Exact output tensor shape.
    pub output_shape: Vec<usize>,
    /// Frame dtype code the input frame must carry.
    pub input_dtype: u8,
    /// Frame dtype code written on the output frame.
    pub output_dtype: u8,
    /// Symbolic dimensions to concretize (for example `{"batch": 1}`).
    #[serde(default)]
    pub symbols: BTreeMap<String, i64>,
}

/// A parsed model blob.
pub struct Blob<'a> {
    pub config: TensorConfig,
    pub onnx: &'a [u8],
}

fn u32_at(bytes: &[u8], at: usize) -> Option<usize> {
    let raw: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    usize::try_from(u32::from_le_bytes(raw)).ok()
}

/// Parse `[magic][u32 config_len][u32 onnx_len][config json][onnx]`.
#[must_use]
pub fn parse_blob(bytes: &[u8]) -> Option<Blob<'_>> {
    if bytes.get(..8)? != BLOB_MAGIC {
        return None;
    }
    let config_len = u32_at(bytes, 8)?;
    let onnx_len = u32_at(bytes, 12)?;
    let config_end = 16usize.checked_add(config_len)?;
    let onnx_end = config_end.checked_add(onnx_len)?;
    let config: TensorConfig = serde_json::from_slice(bytes.get(16..config_end)?).ok()?;
    let valid = !config.input_shape.is_empty()
        && !config.output_shape.is_empty()
        && config.input_shape.len() <= 8
        && config.output_shape.len() <= 8;
    valid.then_some(Blob {
        config,
        onnx: bytes.get(config_end..onnx_end)?,
    })
}

/// Runnable model plus its config.
pub struct Model {
    plan: TypedRunnableModel<TypedModel>,
    config: TensorConfig,
}

impl Model {
    /// Parse, check the single input/output names, concretize symbols,
    /// optimize, and check the declared shapes.
    #[must_use]
    pub fn load(blob: &Blob<'_>) -> Option<Self> {
        let typed = tract_onnx::onnx()
            .model_for_read(&mut std::io::Cursor::new(blob.onnx))
            .ok()?
            .into_typed()
            .ok()?;
        let [input] = typed.input_outlets().ok()? else {
            return None;
        };
        let [output] = typed.output_outlets().ok()? else {
            return None;
        };
        if typed.node(input.node).name != blob.config.input_name
            || outlet_name(&typed, *output) != blob.config.output_name
        {
            return None;
        }
        let mut values = SymbolValues::default();
        for (name, value) in &blob.config.symbols {
            values = values.with(&typed.symbols.sym(name), *value);
        }
        let plan = typed
            .concretize_dims(&values)
            .ok()?
            .into_optimized()
            .ok()?
            .into_runnable()
            .ok()?;
        Some(Self {
            plan,
            config: blob.config.clone(),
        })
    }

    /// Run one input frame → output frame bytes, or `None` if the frame's
    /// version, dtype, dims, or length don't match the tensor config.
    #[must_use]
    pub fn run_frame(&self, frame: &[u8]) -> Option<Vec<u8>> {
        let values = decode_frame(frame, self.config.input_dtype, &self.config.input_shape)?;
        let input =
            tract_ndarray::ArrayD::from_shape_vec(self.config.input_shape.clone(), values).ok()?;
        let result = self.plan.run(tvec!(Tensor::from(input).into())).ok()?;
        let output = result.first()?;
        if output.shape() != self.config.output_shape.as_slice() {
            return None;
        }
        let scores = output.as_slice::<f32>().ok()?;
        Some(encode_frame(
            self.config.output_dtype,
            &self.config.output_shape,
            scores,
        ))
    }
}

fn outlet_name(model: &TypedModel, outlet: OutletId) -> &str {
    model
        .outlet_label(outlet)
        .unwrap_or(model.node(outlet.node).name.as_str())
}

/// Decode an `f32` frame that must match `dtype` and `shape` exactly.
#[must_use]
pub fn decode_frame(frame: &[u8], dtype: u8, shape: &[usize]) -> Option<Vec<f32>> {
    let version = u16::from_le_bytes([*frame.first()?, *frame.get(1)?]);
    let rank = usize::from(*frame.get(3)?);
    if version != FRAME_VERSION || *frame.get(2)? != dtype || rank != shape.len() {
        return None;
    }
    for (index, expected) in shape.iter().enumerate() {
        if u32_at(frame, 4 + index * 4)? != *expected {
            return None;
        }
    }
    let count: usize = shape.iter().product();
    let payload_at = 4 + rank * 4 + 4;
    if u32_at(frame, 4 + rank * 4)? != count.checked_mul(4)?
        || frame.len() != payload_at + count * 4
    {
        return None;
    }
    let values = frame[payload_at..]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect::<Vec<f32>>();
    values
        .iter()
        .all(|value| value.is_finite())
        .then_some(values)
}

/// Encode an `f32` output frame.
#[must_use]
pub fn encode_frame(dtype: u8, shape: &[usize], values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + shape.len() * 4 + values.len() * 4);
    out.extend_from_slice(&FRAME_VERSION.to_le_bytes());
    out.push(dtype);
    out.push(u8::try_from(shape.len()).unwrap_or(u8::MAX));
    for dim in shape {
        out.extend_from_slice(&u32::try_from(*dim).unwrap_or(u32::MAX).to_le_bytes());
    }
    out.extend_from_slice(
        &u32::try_from(values.len() * 4)
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const DIGITS_ONNX: &[u8] = include_bytes!("../../../fixtures/onnx/digits-mlp-1.0.0.onnx");

    fn config_json(input_name: &str, output_name: &str, input_shape: &str) -> String {
        format!(
            r#"{{"input_name":"{input_name}","output_name":"{output_name}","input_shape":{input_shape},"output_shape":[1,10],"input_dtype":2,"output_dtype":3}}"#
        )
    }

    fn blob(config: &str, onnx: &[u8]) -> Vec<u8> {
        let mut out = BLOB_MAGIC.to_vec();
        out.extend_from_slice(&u32::try_from(config.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&u32::try_from(onnx.len()).unwrap().to_le_bytes());
        out.extend_from_slice(config.as_bytes());
        out.extend_from_slice(onnx);
        out
    }

    fn digits() -> Model {
        let bytes = blob(&config_json("x", "y", "[1,64]"), DIGITS_ONNX);
        Model::load(&parse_blob(&bytes).unwrap()).unwrap()
    }

    fn input_frame(values: &[f32]) -> Vec<u8> {
        encode_frame(2, &[1, 64], values)
    }

    #[test]
    fn runs_digits_and_returns_raw_logits() {
        let output = digits().run_frame(&input_frame(&[8.0; 64])).unwrap();
        let logits = decode_frame(&output, 3, &[1, 10]).unwrap();
        assert_eq!(logits.len(), 10);
        assert_eq!(
            output,
            digits().run_frame(&input_frame(&[8.0; 64])).unwrap()
        );
    }

    #[test]
    fn rejects_malformed_blobs() {
        let good = blob(&config_json("x", "y", "[1,64]"), DIGITS_ONNX);
        assert!(parse_blob(&good).is_some());
        assert!(parse_blob(b"short").is_none());
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(parse_blob(&bad_magic).is_none());
        assert!(parse_blob(&good[..good.len() - 1]).is_none());
        assert!(parse_blob(&blob(&config_json("x", "y", "[]"), DIGITS_ONNX)).is_none());
        assert!(
            parse_blob(&blob(
                &config_json("x", "y", "[1,1,1,1,1,1,1,1,1]"),
                DIGITS_ONNX
            ))
            .is_none()
        );
        assert!(parse_blob(&blob(r#"{"input_name":"x"}"#, DIGITS_ONNX)).is_none());
        let unknown = config_json("x", "y", "[1,64]").replace('}', r#","extra":1}"#);
        assert!(parse_blob(&blob(&unknown, DIGITS_ONNX)).is_none());
    }

    #[test]
    fn fails_closed_on_tensor_name_mismatch() {
        for config in [
            config_json("input", "y", "[1,64]"),
            config_json("x", "logits", "[1,64]"),
        ] {
            let bytes = blob(&config, DIGITS_ONNX);
            assert!(
                Model::load(&parse_blob(&bytes).unwrap()).is_none(),
                "{config}"
            );
        }
    }

    #[test]
    fn fails_closed_on_declared_shape_mismatch() {
        let bytes = blob(&config_json("x", "y", "[1,32]"), DIGITS_ONNX);
        let model = Model::load(&parse_blob(&bytes).unwrap());
        assert!(model.is_none_or(|model| {
            model
                .run_frame(&encode_frame(2, &[1, 32], &[0.0; 32]))
                .is_none()
        }));
        let wrong_output = config_json("x", "y", "[1,64]").replace("[1,10]", "[1,11]");
        let bytes = blob(&wrong_output, DIGITS_ONNX);
        let model = Model::load(&parse_blob(&bytes).unwrap()).unwrap();
        assert!(model.run_frame(&input_frame(&[0.0; 64])).is_none());
    }

    #[test]
    fn fails_closed_on_bad_input_frames() {
        let model = digits();
        let good = input_frame(&[1.0; 64]);
        assert!(model.run_frame(&good).is_some());
        // dtype mismatch
        assert!(
            model
                .run_frame(&encode_frame(3, &[1, 64], &[1.0; 64]))
                .is_none()
        );
        // shape mismatch: wrong dims, wrong rank
        assert!(
            model
                .run_frame(&encode_frame(2, &[64, 1], &[1.0; 64]))
                .is_none()
        );
        assert!(
            model
                .run_frame(&encode_frame(2, &[64], &[1.0; 64]))
                .is_none()
        );
        // oversize and undersize payloads
        let mut oversize = good.clone();
        oversize.extend_from_slice(&[0; 4]);
        assert!(model.run_frame(&oversize).is_none());
        assert!(model.run_frame(&good[..good.len() - 4]).is_none());
        assert!(
            model
                .run_frame(&encode_frame(2, &[1, 64], &[1.0; 65]))
                .is_none()
        );
        // version, truncated header, non-finite values
        let mut version = good.clone();
        version[0] = 9;
        assert!(model.run_frame(&version).is_none());
        assert!(model.run_frame(&good[..3]).is_none());
        let mut values = [1.0_f32; 64];
        values[7] = f32::NAN;
        assert!(model.run_frame(&input_frame(&values)).is_none());
    }
}
