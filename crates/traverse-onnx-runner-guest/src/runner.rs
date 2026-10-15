//! Safe model-blob parsing, tensor-config validation, frames, and tract run.
#![forbid(unsafe_code)]

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use tract_onnx::prelude::*;

/// Model blob magic (8 bytes) written by `model package-onnx`.
pub const BLOB_MAGIC: &[u8; 8] = b"TVONNX01";
/// Spec 138 guest frame version (unchanged by guest ABI v2/v3).
pub const FRAME_VERSION: u16 = 1;

/// Spec 138 guest reject codes `model_execute` returns (0.14.0, Decision
/// 115). Hosts map each one to its own stable code and reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    /// The input frame does not match the tensor config (`-2`).
    Frame = -2,
    /// tract failed to run the plan (`-3`).
    Inference = -3,
    /// An output is not the declared shape or element type (`-4`).
    Output = -4,
}

impl Reject {
    /// The negative `model_execute` return for this rejection.
    #[must_use]
    pub const fn code(self) -> i32 {
        self as i32
    }
}

/// Physical element type carried in a guest-frame payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElementType {
    /// Little-endian IEEE `f32` (4 bytes). Finite values only.
    F32,
    /// Little-endian `i64` (8 bytes). Used for token ids / masks.
    I64,
}

impl ElementType {
    /// Bytes per element in the frame payload.
    #[must_use]
    pub const fn width(self) -> usize {
        match self {
            Self::F32 => 4,
            Self::I64 => 8,
        }
    }
}

impl Default for ElementType {
    fn default() -> Self {
        Self::F32
    }
}

/// One named tensor fixed at package time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedTensor {
    /// ONNX graph tensor name.
    pub name: String,
    /// Exact shape after symbol concretization.
    pub shape: Vec<usize>,
    /// Guest-defined frame dtype code this tensor's frame must carry.
    pub dtype: u8,
    /// Physical payload element type. Defaults to `f32`.
    #[serde(default)]
    pub element: ElementType,
}

/// Tensor config fixed at package time: N named inputs and M named outputs.
///
/// JSON accepts the legacy single-IO form (`input_name` / `output_name` / …)
/// and the multi-IO form (`inputs` / `outputs`). Both serialize back to the
/// same shape they were read as, so single-IO packages keep working.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TensorConfig {
    /// Ordered named inputs (at least one).
    pub inputs: Vec<NamedTensor>,
    /// Ordered named outputs (at least one).
    pub outputs: Vec<NamedTensor>,
    /// Symbolic dimensions to concretize (for example `{"batch": 1}`).
    pub symbols: BTreeMap<String, i64>,
    /// Whether this config was read (and should be written) as legacy JSON.
    legacy_single: bool,
}

impl TensorConfig {
    /// Build a multi-IO config.
    #[must_use]
    pub fn multi(
        inputs: Vec<NamedTensor>,
        outputs: Vec<NamedTensor>,
        symbols: BTreeMap<String, i64>,
    ) -> Self {
        Self {
            inputs,
            outputs,
            symbols,
            legacy_single: false,
        }
    }

    /// Build a legacy single-IO config (`f32` elements).
    #[must_use]
    pub fn single(
        input_name: impl Into<String>,
        output_name: impl Into<String>,
        input_shape: Vec<usize>,
        output_shape: Vec<usize>,
        input_dtype: u8,
        output_dtype: u8,
        symbols: BTreeMap<String, i64>,
    ) -> Self {
        Self {
            inputs: vec![NamedTensor {
                name: input_name.into(),
                shape: input_shape,
                dtype: input_dtype,
                element: ElementType::F32,
            }],
            outputs: vec![NamedTensor {
                name: output_name.into(),
                shape: output_shape,
                dtype: output_dtype,
                element: ElementType::F32,
            }],
            symbols,
            legacy_single: true,
        }
    }

    /// True when the config was (or should be) the legacy single-IO JSON.
    #[must_use]
    pub const fn is_legacy_single(&self) -> bool {
        self.legacy_single
    }
}

impl<'de> Deserialize<'de> for TensorConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Multi {
            inputs: Vec<NamedTensor>,
            outputs: Vec<NamedTensor>,
            #[serde(default)]
            symbols: BTreeMap<String, i64>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Single {
            input_name: String,
            output_name: String,
            input_shape: Vec<usize>,
            output_shape: Vec<usize>,
            input_dtype: u8,
            output_dtype: u8,
            #[serde(default)]
            symbols: BTreeMap<String, i64>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Form {
            Multi(Multi),
            Single(Single),
        }
        match Form::deserialize(deserializer)? {
            Form::Multi(multi) => Ok(Self {
                inputs: multi.inputs,
                outputs: multi.outputs,
                symbols: multi.symbols,
                legacy_single: false,
            }),
            Form::Single(single) => Ok(Self::single(
                single.input_name,
                single.output_name,
                single.input_shape,
                single.output_shape,
                single.input_dtype,
                single.output_dtype,
                single.symbols,
            )),
        }
    }
}

impl Serialize for TensorConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if self.legacy_single {
            if self.inputs.len() != 1 || self.outputs.len() != 1 {
                return Err(serde::ser::Error::custom(
                    "legacy single tensor config must have exactly one input and one output",
                ));
            }
            #[derive(Serialize)]
            struct Single<'a> {
                input_name: &'a str,
                output_name: &'a str,
                input_shape: &'a [usize],
                output_shape: &'a [usize],
                input_dtype: u8,
                output_dtype: u8,
                #[serde(skip_serializing_if = "BTreeMap::is_empty")]
                symbols: &'a BTreeMap<String, i64>,
            }
            Single {
                input_name: &self.inputs[0].name,
                output_name: &self.outputs[0].name,
                input_shape: &self.inputs[0].shape,
                output_shape: &self.outputs[0].shape,
                input_dtype: self.inputs[0].dtype,
                output_dtype: self.outputs[0].dtype,
                symbols: &self.symbols,
            }
            .serialize(serializer)
        } else {
            #[derive(Serialize)]
            struct Multi<'a> {
                inputs: &'a [NamedTensor],
                outputs: &'a [NamedTensor],
                #[serde(skip_serializing_if = "BTreeMap::is_empty")]
                symbols: &'a BTreeMap<String, i64>,
            }
            Multi {
                inputs: &self.inputs,
                outputs: &self.outputs,
                symbols: &self.symbols,
            }
            .serialize(serializer)
        }
    }
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

fn shape_ok(shape: &[usize]) -> bool {
    !shape.is_empty() && shape.len() <= 8 && !shape.contains(&0)
}

fn config_ok(config: &TensorConfig) -> bool {
    !config.inputs.is_empty()
        && !config.outputs.is_empty()
        && config.inputs.len() <= 16
        && config.outputs.len() <= 16
        && config
            .inputs
            .iter()
            .chain(config.outputs.iter())
            .all(|tensor| !tensor.name.trim().is_empty() && shape_ok(&tensor.shape))
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
    config_ok(&config).then_some(Blob {
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
    /// Parse, check every input/output name, concretize symbols, optimize,
    /// and keep the declared tensor config.
    #[must_use]
    pub fn load(blob: &Blob<'_>) -> Option<Self> {
        let typed = tract_onnx::onnx()
            .model_for_read(&mut std::io::Cursor::new(blob.onnx))
            .ok()?
            .into_typed()
            .ok()?;
        let input_outlets = typed.input_outlets().ok()?;
        let output_outlets = typed.output_outlets().ok()?;
        if input_outlets.len() != blob.config.inputs.len()
            || output_outlets.len() != blob.config.outputs.len()
        {
            return None;
        }
        for (outlet, expected) in input_outlets.iter().zip(&blob.config.inputs) {
            if typed.node(outlet.node).name != expected.name {
                return None;
            }
        }
        for (outlet, expected) in output_outlets.iter().zip(&blob.config.outputs) {
            if outlet_name(&typed, *outlet) != expected.name {
                return None;
            }
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

    /// Run one input frame bundle → output frame bundle bytes.
    ///
    /// A bundle is the concatenation of one Spec 138 frame per declared
    /// tensor, in `inputs` / `outputs` order.
    ///
    /// # Errors
    ///
    /// [`Reject::Frame`] when any input frame's version, dtype, dims, length,
    /// or values don't match the tensor config; [`Reject::Inference`] when
    /// tract fails; [`Reject::Output`] when an output is not the declared
    /// shape or element type.
    pub fn run_frame(&self, frame: &[u8]) -> Result<Vec<u8>, Reject> {
        let mut offset = 0;
        let mut inputs = TVec::new();
        for tensor in &self.config.inputs {
            let (consumed, values) = decode_frame_at(frame, offset, tensor).ok_or(Reject::Frame)?;
            offset = offset.checked_add(consumed).ok_or(Reject::Frame)?;
            let array = array_from_values(tensor, values).map_err(|_| Reject::Frame)?;
            inputs.push(array.into());
        }
        if offset != frame.len() {
            return Err(Reject::Frame);
        }
        let result = self.plan.run(inputs).map_err(|_| Reject::Inference)?;
        if result.len() != self.config.outputs.len() {
            return Err(Reject::Output);
        }
        let mut out = Vec::new();
        for (tensor, value) in self.config.outputs.iter().zip(result.iter()) {
            if value.shape() != tensor.shape.as_slice() {
                return Err(Reject::Output);
            }
            out.extend_from_slice(&encode_tensor_frame(tensor, value)?);
        }
        Ok(out)
    }
}

fn outlet_name(model: &TypedModel, outlet: OutletId) -> &str {
    model
        .outlet_label(outlet)
        .unwrap_or(model.node(outlet.node).name.as_str())
}

enum Decoded {
    F32(Vec<f32>),
    I64(Vec<i64>),
}

fn array_from_values(tensor: &NamedTensor, values: Decoded) -> Result<Tensor, ()> {
    match (tensor.element, values) {
        (ElementType::F32, Decoded::F32(values)) => {
            tract_ndarray::ArrayD::from_shape_vec(tensor.shape.clone(), values)
                .map(Tensor::from)
                .map_err(|_| ())
        }
        (ElementType::I64, Decoded::I64(values)) => {
            tract_ndarray::ArrayD::from_shape_vec(tensor.shape.clone(), values)
                .map(Tensor::from)
                .map_err(|_| ())
        }
        _ => Err(()),
    }
}

fn encode_tensor_frame(tensor: &NamedTensor, value: &Tensor) -> Result<Vec<u8>, Reject> {
    match tensor.element {
        ElementType::F32 => {
            let scores = value.as_slice::<f32>().map_err(|_| Reject::Output)?;
            if !scores.iter().all(|v| v.is_finite()) {
                return Err(Reject::Output);
            }
            Ok(write_frame(
                tensor.dtype,
                &tensor.shape,
                ElementType::F32,
                scores.len(),
                |out| {
                    for score in scores {
                        out.extend_from_slice(&score.to_le_bytes());
                    }
                },
            ))
        }
        ElementType::I64 => {
            let values = value.as_slice::<i64>().map_err(|_| Reject::Output)?;
            Ok(write_frame(
                tensor.dtype,
                &tensor.shape,
                ElementType::I64,
                values.len(),
                |out| {
                    for value in values {
                        out.extend_from_slice(&value.to_le_bytes());
                    }
                },
            ))
        }
    }
}

/// Decode one frame starting at `offset`; returns `(bytes_consumed, values)`.
fn decode_frame_at(
    bundle: &[u8],
    offset: usize,
    tensor: &NamedTensor,
) -> Option<(usize, Decoded)> {
    let frame = bundle.get(offset..)?;
    let version = u16::from_le_bytes([*frame.first()?, *frame.get(1)?]);
    let rank = usize::from(*frame.get(3)?);
    if version != FRAME_VERSION || *frame.get(2)? != tensor.dtype || rank != tensor.shape.len() {
        return None;
    }
    for (index, expected) in tensor.shape.iter().enumerate() {
        if u32_at(frame, 4 + index * 4)? != *expected {
            return None;
        }
    }
    let count: usize = tensor.shape.iter().product();
    let width = tensor.element.width();
    let payload_bytes = count.checked_mul(width)?;
    let payload_at = 4 + rank * 4 + 4;
    if u32_at(frame, 4 + rank * 4)? != payload_bytes {
        return None;
    }
    let end = payload_at.checked_add(payload_bytes)?;
    let payload = frame.get(payload_at..end)?;
    let values = match tensor.element {
        ElementType::F32 => {
            let values = payload
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect::<Vec<f32>>();
            values
                .iter()
                .all(|value| value.is_finite())
                .then_some(Decoded::F32(values))?
        }
        ElementType::I64 => Decoded::I64(
            payload
                .chunks_exact(8)
                .map(|c| i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
                .collect(),
        ),
    };
    Some((end, values))
}

/// Decode an `f32` frame that must match `dtype` and `shape` exactly.
#[must_use]
pub fn decode_frame(frame: &[u8], dtype: u8, shape: &[usize]) -> Option<Vec<f32>> {
    let tensor = NamedTensor {
        name: String::new(),
        shape: shape.to_vec(),
        dtype,
        element: ElementType::F32,
    };
    let (consumed, values) = decode_frame_at(frame, 0, &tensor)?;
    (consumed == frame.len()).then_some(())?;
    match values {
        Decoded::F32(values) => Some(values),
        Decoded::I64(_) => None,
    }
}

/// Encode a guest frame header plus caller-written payload.
fn write_frame(
    dtype: u8,
    shape: &[usize],
    element: ElementType,
    count: usize,
    write_payload: impl FnOnce(&mut Vec<u8>),
) -> Vec<u8> {
    let payload_bytes = count.saturating_mul(element.width());
    let mut out = Vec::with_capacity(8 + shape.len() * 4 + payload_bytes);
    out.extend_from_slice(&FRAME_VERSION.to_le_bytes());
    out.push(dtype);
    out.push(u8::try_from(shape.len()).unwrap_or(u8::MAX));
    for dim in shape {
        out.extend_from_slice(&u32::try_from(*dim).unwrap_or(u32::MAX).to_le_bytes());
    }
    out.extend_from_slice(&u32::try_from(payload_bytes).unwrap_or(u32::MAX).to_le_bytes());
    write_payload(&mut out);
    out
}

/// Encode an `f32` output frame.
#[must_use]
pub fn encode_frame(dtype: u8, shape: &[usize], values: &[f32]) -> Vec<u8> {
    write_frame(dtype, shape, ElementType::F32, values.len(), |out| {
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
    })
}

/// Encode an `i64` frame.
#[must_use]
pub fn encode_frame_i64(dtype: u8, shape: &[usize], values: &[i64]) -> Vec<u8> {
    write_frame(dtype, shape, ElementType::I64, values.len(), |out| {
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
    })
}

/// Byte length of one frame for `tensor` (header + payload).
#[must_use]
pub fn frame_byte_len(tensor: &NamedTensor) -> Option<u64> {
    let count = tensor
        .shape
        .iter()
        .try_fold(1_u64, |acc, dim| acc.checked_mul(u64::try_from(*dim).ok()?))?;
    let header = 8 + 4 * u64::try_from(tensor.shape.len()).ok()?;
    let payload = count.checked_mul(u64::try_from(tensor.element.width()).ok()?)?;
    header.checked_add(payload)
}

/// Total byte length of a concatenated frame bundle.
#[must_use]
pub fn bundle_byte_len(tensors: &[NamedTensor]) -> Option<u64> {
    tensors.iter().try_fold(0_u64, |acc, tensor| {
        acc.checked_add(frame_byte_len(tensor)?)
    })
}

/// A zero-valued input bundle matching `config.inputs`.
#[must_use]
pub fn zero_input_bundle(config: &TensorConfig) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    for tensor in &config.inputs {
        let count: usize = tensor.shape.iter().product();
        match tensor.element {
            ElementType::F32 => {
                out.extend_from_slice(&encode_frame(
                    tensor.dtype,
                    &tensor.shape,
                    &vec![0.0; count],
                ));
            }
            ElementType::I64 => {
                out.extend_from_slice(&encode_frame_i64(
                    tensor.dtype,
                    &tensor.shape,
                    &vec![0; count],
                ));
            }
        }
    }
    Some(out)
}

/// Whether `bundle` is exactly one frame per `tensors` entry, matching dtype
/// and shape (payload contents are not interpreted beyond length).
#[must_use]
pub fn bundle_matches(bundle: &[u8], tensors: &[NamedTensor]) -> bool {
    let mut offset = 0;
    for tensor in tensors {
        let Some((consumed, _)) = decode_frame_at(bundle, offset, tensor) else {
            return false;
        };
        offset = match offset.checked_add(consumed) {
            Some(value) => value,
            None => return false,
        };
    }
    offset == bundle.len()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const DIGITS_ONNX: &[u8] = include_bytes!("../../../fixtures/onnx/digits-mlp-1.0.0.onnx");
    const TEXT_ONNX: &[u8] =
        include_bytes!("../../../fixtures/onnx/hf-text-encoder-toy-1.0.0.onnx");

    fn config_json(input_name: &str, output_name: &str, input_shape: &str) -> String {
        format!(
            r#"{{"input_name":"{input_name}","output_name":"{output_name}","input_shape":{input_shape},"output_shape":[1,10],"input_dtype":2,"output_dtype":3}}"#
        )
    }

    fn text_config_json() -> String {
        r#"{
            "inputs": [
                {"name":"input_ids","shape":[1,4],"dtype":6,"element":"i64"},
                {"name":"attention_mask","shape":[1,4],"dtype":6,"element":"i64"}
            ],
            "outputs": [
                {"name":"last_hidden_state","shape":[1,4,8],"dtype":2,"element":"f32"},
                {"name":"pooler_output","shape":[1,8],"dtype":3,"element":"f32"}
            ]
        }"#
        .to_string()
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

    fn text_encoder() -> Model {
        let bytes = blob(&text_config_json(), TEXT_ONNX);
        Model::load(&parse_blob(&bytes).unwrap()).unwrap()
    }

    fn input_frame(values: &[f32]) -> Vec<u8> {
        encode_frame(2, &[1, 64], values)
    }

    fn text_input(ids: [i64; 4], mask: [i64; 4]) -> Vec<u8> {
        let mut out = encode_frame_i64(6, &[1, 4], &ids);
        out.extend_from_slice(&encode_frame_i64(6, &[1, 4], &mask));
        out
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
    fn runs_multi_io_text_encoder_deterministically() {
        let model = text_encoder();
        let input = text_input([1, 2, 3, 0], [1, 1, 1, 0]);
        let output = model.run_frame(&input).unwrap();
        assert!(bundle_matches(&output, &model.config.outputs));
        assert_eq!(output, model.run_frame(&input).unwrap());
        let hidden_bytes = frame_byte_len(&model.config.outputs[0]).unwrap() as usize;
        let hidden = decode_frame(&output[..hidden_bytes], 2, &[1, 4, 8]).unwrap();
        let pooler = decode_frame(&output[hidden_bytes..], 3, &[1, 8]).unwrap();
        assert_eq!(hidden.len(), 32);
        assert_eq!(pooler.len(), 8);
        assert!(hidden[24..32].iter().all(|value| *value == 0.0));
    }

    #[test]
    fn multi_io_rejects_truncated_and_extra_frames() {
        let model = text_encoder();
        let good = text_input([1, 2, 3, 0], [1, 1, 1, 0]);
        assert!(model.run_frame(&good).is_ok());
        assert_eq!(
            model.run_frame(&good[..good.len() - 8]),
            Err(Reject::Frame)
        );
        let mut extra = good.clone();
        extra.extend_from_slice(&encode_frame_i64(6, &[1, 4], &[0; 4]));
        assert_eq!(model.run_frame(&extra), Err(Reject::Frame));
        let mut bad = encode_frame_i64(6, &[1, 4], &[1, 2, 3, 0]);
        bad.extend_from_slice(&encode_frame_i64(7, &[1, 4], &[1, 1, 1, 0]));
        assert_eq!(model.run_frame(&bad), Err(Reject::Frame));
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
        assert!(parse_blob(&blob(&text_config_json(), TEXT_ONNX)).is_some());
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
        let bad = text_config_json().replace("input_ids", "tokens");
        assert!(Model::load(&parse_blob(&blob(&bad, TEXT_ONNX)).unwrap()).is_none());
    }

    #[test]
    fn fails_closed_on_declared_shape_mismatch() {
        let bytes = blob(&config_json("x", "y", "[1,32]"), DIGITS_ONNX);
        let model = Model::load(&parse_blob(&bytes).unwrap());
        assert!(model.is_none_or(|model| {
            model
                .run_frame(&encode_frame(2, &[1, 32], &[0.0; 32]))
                .is_err()
        }));
        let wrong_output = config_json("x", "y", "[1,64]").replace("[1,10]", "[1,11]");
        let bytes = blob(&wrong_output, DIGITS_ONNX);
        let model = Model::load(&parse_blob(&bytes).unwrap()).unwrap();
        assert_eq!(
            model.run_frame(&input_frame(&[0.0; 64])),
            Err(Reject::Output)
        );
    }

    #[test]
    fn rejects_bad_input_frames_as_frame_mismatch() {
        let model = digits();
        let good = input_frame(&[1.0; 64]);
        assert!(model.run_frame(&good).is_ok());
        let mut oversize = good.clone();
        oversize.extend_from_slice(&[0; 4]);
        let mut version = good.clone();
        version[0] = 9;
        let mut values = [1.0_f32; 64];
        values[7] = f32::NAN;
        let bad = [
            ("dtype", encode_frame(3, &[1, 64], &[1.0; 64])),
            ("dims", encode_frame(2, &[64, 1], &[1.0; 64])),
            ("rank", encode_frame(2, &[64], &[1.0; 64])),
            ("oversize payload", oversize),
            ("undersize payload", good[..good.len() - 4].to_vec()),
            ("payload length", encode_frame(2, &[1, 64], &[1.0; 65])),
            ("frame version", version),
            ("truncated header", good[..3].to_vec()),
            ("non-finite value", input_frame(&values)),
        ];
        for (case, frame) in bad {
            assert_eq!(model.run_frame(&frame), Err(Reject::Frame), "{case}");
        }
    }

    #[test]
    fn reject_codes_are_the_spec_138_values() {
        assert_eq!(Reject::Frame.code(), -2);
        assert_eq!(Reject::Inference.code(), -3);
        assert_eq!(Reject::Output.code(), -4);
    }

    #[test]
    fn legacy_and_multi_json_round_trip() {
        let legacy: TensorConfig = serde_json::from_str(&config_json("x", "y", "[1,64]")).unwrap();
        assert!(legacy.is_legacy_single());
        assert_eq!(legacy.inputs.len(), 1);
        let legacy_json = serde_json::to_value(&legacy).unwrap();
        assert_eq!(legacy_json["input_name"], "x");
        assert!(legacy_json.get("inputs").is_none());

        let multi: TensorConfig = serde_json::from_str(&text_config_json()).unwrap();
        assert!(!multi.is_legacy_single());
        assert_eq!(multi.inputs.len(), 2);
        assert_eq!(multi.outputs[0].element, ElementType::F32);
        assert_eq!(multi.inputs[0].element, ElementType::I64);
        let multi_json = serde_json::to_value(&multi).unwrap();
        assert!(multi_json.get("input_name").is_none());
        assert_eq!(multi_json["inputs"][0]["name"], "input_ids");
    }
}
