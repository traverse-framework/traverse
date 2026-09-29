//! Safe forward pass for the trained 64 → 32 (`ReLU`) → 10 digits MLP.
//!
//! Frames follow the Spec 138 guest ABI v1 (little-endian):
//! `abi_version: u16`, `dtype: u8`, `rank: u8`, `dims: [u32; rank]`,
//! `payload_len: u32`, payload.
//!
//! - Input: dtype `2` (`f32`), dims `[64]`, payload = 64 raw pixel counts in
//!   `0..=16` (the UCI optdigits 8×8 grid, row-major).
//! - Output: dtype `3` (`f32`), dims `[11]`, payload = 10 class logits then
//!   the predicted class as `f32` (first index of the maximum logit).
//!
//! Accumulation order matches the trainer exactly (bias first, then each
//! weight × input in order), and wasm has no fused multiply-add, so logits
//! are bit-identical across wasmtime, browsers, and the host trainer.

#![forbid(unsafe_code)]

/// Input features.
pub const INPUTS: usize = 64;
/// Hidden units.
pub const HIDDEN: usize = 32;
/// Output classes.
pub const CLASSES: usize = 10;
/// Serialized `f32` parameters.
pub const PARAMETERS: usize = HIDDEN * INPUTS + HIDDEN + CLASSES * HIDDEN + CLASSES;
/// Guest frame ABI version.
pub const ABI_VERSION: u16 = 1;
/// `f32` input dtype code.
pub const INPUT_DTYPE: u8 = 2;
/// `f32` output dtype code.
pub const OUTPUT_DTYPE: u8 = 3;
/// Output payload values: 10 logits + predicted class.
pub const OUTPUT_VALUES: usize = CLASSES + 1;
/// Exact input frame length (header + 64 `f32`).
pub const INPUT_FRAME_LEN: usize = 12 + INPUTS * 4;
/// Exact output frame length (header + 11 `f32`).
pub const OUTPUT_FRAME_LEN: usize = 12 + OUTPUT_VALUES * 4;

const FEATURE_SCALE: f32 = 0.0625;
const MAX_PIXEL: f32 = 16.0;
const B1: usize = HIDDEN * INPUTS;
const W2: usize = B1 + HIDDEN;
const B2: usize = W2 + CLASSES * HIDDEN;

/// Trained weights from `traverse-model-trainer` (`TrainConfig::published`).
static WEIGHTS: &[u8; PARAMETERS * 4] = include_bytes!("../weights/digits-mlp-1.0.0.bin");

fn param(index: usize) -> f32 {
    let start = index * 4;
    match WEIGHTS.get(start..start + 4) {
        Some(&[a, b, c, d]) => f32::from_le_bytes([a, b, c, d]),
        _ => 0.0,
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    match bytes.get(offset..offset + 4)? {
        &[a, b, c, d] => Some(u32::from_le_bytes([a, b, c, d])),
        _ => None,
    }
}

/// Decode and validate an input frame into raw pixel counts.
#[must_use]
pub fn decode_input(frame: &[u8]) -> Option<[f32; INPUTS]> {
    let header = frame.get(..12)?;
    let valid_header = u16::from_le_bytes([header[0], header[1]]) == ABI_VERSION
        && header[2] == INPUT_DTYPE
        && header[3] == 1
        && read_u32(header, 4)? == 64
        && read_u32(header, 8)? == 256;
    if !valid_header || frame.len() != INPUT_FRAME_LEN {
        return None;
    }
    let mut pixels = [0.0_f32; INPUTS];
    for (pixel, chunk) in pixels.iter_mut().zip(frame.get(12..)?.chunks_exact(4)) {
        let value = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if !(0.0..=MAX_PIXEL).contains(&value) {
            return None;
        }
        *pixel = value;
    }
    Some(pixels)
}

/// Class logits for raw pixel counts in `0..=16`.
#[must_use]
pub fn logits(pixels: &[f32; INPUTS]) -> [f32; CLASSES] {
    let features = pixels.map(|pixel| pixel * FEATURE_SCALE);
    let mut hidden = [0.0_f32; HIDDEN];
    for (unit, value) in hidden.iter_mut().enumerate() {
        *value = param(B1 + unit);
        for (input, feature) in features.iter().enumerate() {
            *value += param(unit * INPUTS + input) * feature;
        }
        *value = value.max(0.0);
    }
    let mut logits = [0.0_f32; CLASSES];
    for (class, logit) in logits.iter_mut().enumerate() {
        *logit = param(B2 + class);
        for (unit, value) in hidden.iter().enumerate() {
            *logit += param(W2 + class * HIDDEN + unit) * value;
        }
    }
    logits
}

/// First index of the maximum logit (ties resolve to the lowest index).
#[must_use]
pub fn argmax(values: &[f32; CLASSES]) -> usize {
    let mut best = 0;
    for (index, value) in values.iter().enumerate() {
        if *value > values[best] {
            best = index;
        }
    }
    best
}

/// Run one input frame, writing the output frame into `out`. Returns the
/// number of bytes written, or `None` for a malformed input frame.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn execute_frame(input: &[u8], out: &mut [u8; OUTPUT_FRAME_LEN]) -> Option<usize> {
    let pixels = decode_input(input)?;
    let logits = logits(&pixels);
    let predicted = argmax(&logits) as f32;
    out[..2].copy_from_slice(&ABI_VERSION.to_le_bytes());
    out[2] = OUTPUT_DTYPE;
    out[3] = 1;
    out[4..8].copy_from_slice(&11_u32.to_le_bytes());
    out[8..12].copy_from_slice(&44_u32.to_le_bytes());
    for (slot, value) in out[12..]
        .chunks_exact_mut(4)
        .zip(logits.iter().chain(core::iter::once(&predicted)))
    {
        slot.copy_from_slice(&value.to_le_bytes());
    }
    Some(OUTPUT_FRAME_LEN)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::cast_precision_loss, clippy::float_cmp)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    const TEST_SPLIT: &str = include_str!("../../../fixtures/datasets/uci-optdigits/optdigits.tes");

    fn frame(pixels: &[f32]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&ABI_VERSION.to_le_bytes());
        out.extend_from_slice(&[INPUT_DTYPE, 1]);
        out.extend_from_slice(&64_u32.to_le_bytes());
        out.extend_from_slice(&256_u32.to_le_bytes());
        for pixel in pixels {
            out.extend_from_slice(&pixel.to_le_bytes());
        }
        out
    }

    fn rows() -> Vec<(Vec<f32>, usize)> {
        TEST_SPLIT
            .lines()
            .map(|line| {
                let values: Vec<u8> = line.split(',').map(|v| v.parse().expect("int")).collect();
                let (label, pixels) = values.split_last().expect("row");
                (
                    pixels.iter().map(|&p| f32::from(p)).collect(),
                    usize::from(*label),
                )
            })
            .collect()
    }

    #[test]
    fn guest_reproduces_the_trainers_held_out_accuracy_exactly() {
        let rows = rows();
        assert_eq!(rows.len(), 1797);
        let correct = rows
            .iter()
            .filter(|(pixels, label)| {
                let mut out = [0_u8; OUTPUT_FRAME_LEN];
                execute_frame(&frame(pixels), &mut out).expect("frame");
                let predicted = f32::from_le_bytes([out[52], out[53], out[54], out[55]]);
                predicted == *label as f32
            })
            .count();
        // 1727 / 1797 = 96.10%, the same count the host trainer reports.
        assert_eq!(correct, 1727);
    }

    #[test]
    fn output_frame_has_the_documented_shape() {
        let (pixels, _) = &rows()[0];
        let mut out = [0_u8; OUTPUT_FRAME_LEN];
        assert_eq!(execute_frame(&frame(pixels), &mut out), Some(56));
        assert_eq!(&out[..4], &[1, 0, OUTPUT_DTYPE, 1]);
        assert_eq!(read_u32(&out, 4), Some(11));
        assert_eq!(read_u32(&out, 8), Some(44));
        let logits: [f32; CLASSES] = core::array::from_fn(|index| {
            let at = 12 + index * 4;
            f32::from_le_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]])
        });
        let predicted = f32::from_le_bytes([out[52], out[53], out[54], out[55]]);
        assert_eq!(predicted, argmax(&logits) as f32);
    }

    #[test]
    fn malformed_frames_fail_closed() {
        let good = frame(&[1.0; INPUTS]);
        let mut out = [0_u8; OUTPUT_FRAME_LEN];
        assert!(execute_frame(&good, &mut out).is_some());
        assert!(execute_frame(&good[..11], &mut out).is_none());
        assert!(execute_frame(&good[..good.len() - 1], &mut out).is_none());
        let mut longer = good.clone();
        longer.push(0);
        assert!(execute_frame(&longer, &mut out).is_none());
        for (offset, byte) in [(0, 2_u8), (2, 3), (3, 2), (4, 63), (8, 255)] {
            let mut bad = good.clone();
            bad[offset] = byte;
            assert!(execute_frame(&bad, &mut out).is_none(), "offset {offset}");
        }
        for value in [-1.0, 16.5, f32::NAN, f32::INFINITY] {
            let mut pixels = [1.0; INPUTS];
            pixels[5] = value;
            assert!(
                execute_frame(&frame(&pixels), &mut out).is_none(),
                "{value}"
            );
        }
        assert_eq!(read_u32(&[1, 2, 3], 0), None);
    }

    #[test]
    fn argmax_prefers_the_lowest_index_on_ties() {
        let mut values = [0.0_f32; CLASSES];
        values[2] = 1.0;
        values[8] = 1.0;
        assert_eq!(argmax(&values), 2);
        assert_eq!(param(PARAMETERS), 0.0);
    }
}
