//! Deterministic, offline trainer for Traverse's first trained Spec 138
//! exact-ref model package (Decision 102, `#1461`).
//!
//! Trains a 64 → 32 (`ReLU`) → 10 multilayer perceptron on the vendored UCI
//! Optical Recognition of Handwritten Digits dataset (CC BY 4.0) with seeded
//! minibatch SGD (linearly decaying learning rate) and softmax
//! cross-entropy on standardized features, folds the standardization into
//! the first layer, then serializes the weights as
//! little-endian `f32` in the exact layout the `no_std` guest
//! (`crates/traverse-digits-mlp-guest`) embeds:
//! `W1[HIDDEN][INPUTS]`, `b1[HIDDEN]`, `W2[CLASSES][HIDDEN]`, `b2[CLASSES]`.
//!
//! Nothing here touches the network: the dataset is read from
//! `fixtures/datasets/uci-optdigits/` and verified against pinned SHA-256
//! digests before use.

pub mod onnx;

use sha2::{Digest, Sha256};
use std::fmt;

/// Input features per sample (8×8 block pixel counts).
pub const INPUTS: usize = 64;
/// Hidden units.
pub const HIDDEN: usize = 32;
/// Output classes (digits 0–9).
pub const CLASSES: usize = 10;
/// Total `f32` parameters in the serialized weight file.
pub const PARAMETERS: usize = HIDDEN * INPUTS + HIDDEN + CLASSES * HIDDEN + CLASSES;
/// Pixel counts are integers in `0..=16`; features are scaled by `1/16`
/// (an exact power of two, so the guest reproduces it bit-for-bit).
pub const FEATURE_SCALE: f32 = 0.0625;
/// Largest valid raw pixel count.
pub const MAX_PIXEL: f32 = 16.0;

/// Pinned SHA-256 of the vendored training split (`optdigits.tra`).
pub const TRAIN_SHA256: &str = "e1b683cc211604fe8fd8c4417e6a69f31380e0c61d4af22e93cc21e9257ffedd";
/// Pinned SHA-256 of the vendored held-out test split (`optdigits.tes`).
pub const TEST_SHA256: &str = "6ebb3d2fee246a4e99363262ddf8a00a3c41bee6014c373ed9d9216ba7f651b8";

/// Hyperparameters. The committed weights are produced by
/// [`TrainConfig::published`]; changing any field changes the weights.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrainConfig {
    /// PRNG seed for initialization and per-epoch shuffling.
    pub seed: u64,
    /// Passes over the training split.
    pub epochs: usize,
    /// Minibatch size.
    pub batch_size: usize,
    /// Initial SGD learning rate, decayed linearly to 10% by the last epoch.
    pub learning_rate: f32,
}

impl TrainConfig {
    /// The configuration that produced the committed `digits-mlp-1.0.0` weights.
    #[must_use]
    pub fn published() -> Self {
        Self {
            seed: 1461,
            epochs: 60,
            batch_size: 32,
            learning_rate: 0.1,
        }
    }
}

/// One labelled sample with features already scaled by [`FEATURE_SCALE`].
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// Scaled features.
    pub features: [f32; INPUTS],
    /// Class label `0..=9`.
    pub label: usize,
}

/// Dataset parse / integrity failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetError(pub String);

impl fmt::Display for DatasetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "dataset error: {}", self.0)
    }
}

impl std::error::Error for DatasetError {}

/// Lowercase hex SHA-256.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Verify `bytes` against a pinned digest, then parse UCI optdigits rows
/// (64 integers in `0..=16` followed by a label in `0..=9`).
///
/// # Errors
///
/// Returns [`DatasetError`] on a digest mismatch or any malformed row.
pub fn parse_verified(bytes: &[u8], expected_sha256: &str) -> Result<Vec<Sample>, DatasetError> {
    let actual = sha256_hex(bytes);
    if actual != expected_sha256 {
        return Err(DatasetError(format!(
            "digest mismatch: expected {expected_sha256}, got {actual}"
        )));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| DatasetError("dataset is not UTF-8".to_string()))?;
    let samples = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            parse_row(line).map_err(|reason| DatasetError(format!("row {index}: {reason}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if samples.is_empty() {
        return Err(DatasetError("dataset has no rows".to_string()));
    }
    Ok(samples)
}

fn parse_row(line: &str) -> Result<Sample, String> {
    let values: Vec<u8> = line
        .trim()
        .split(',')
        .map(|field| {
            field
                .parse::<u8>()
                .map_err(|_| format!("non-integer field {field:?}"))
        })
        .collect::<Result<_, _>>()?;
    let [pixels @ .., label] = values.as_slice() else {
        return Err("empty row".to_string());
    };
    if pixels.len() != INPUTS {
        return Err(format!("expected {INPUTS} pixels, got {}", pixels.len()));
    }
    if pixels.iter().any(|&pixel| f32::from(pixel) > MAX_PIXEL) {
        return Err("pixel count above 16".to_string());
    }
    let label = usize::from(*label);
    if label >= CLASSES {
        return Err(format!("label {label} out of range"));
    }
    let mut features = [0.0_f32; INPUTS];
    for (feature, &pixel) in features.iter_mut().zip(pixels) {
        *feature = f32::from(pixel) * FEATURE_SCALE;
    }
    Ok(Sample { features, label })
}

/// `SplitMix64`: tiny, well-distributed, and identical on every platform.
#[derive(Debug, Clone)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    /// Seeded generator.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform `f32` in `[-limit, limit)` from the top 24 bits.
    #[allow(clippy::cast_precision_loss)]
    pub fn uniform(&mut self, limit: f32) -> f32 {
        let unit = (self.next_u64() >> 40) as f32 / (1_u64 << 24) as f32;
        (unit * 2.0 - 1.0) * limit
    }

    /// Uniform index in `0..bound` (`bound > 0`).
    #[allow(clippy::cast_possible_truncation)]
    pub fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
}

/// Multilayer perceptron parameters in guest layout order.
#[derive(Debug, Clone, PartialEq)]
pub struct Mlp {
    /// `W1[h][i]`.
    pub w1: Vec<[f32; INPUTS]>,
    /// `b1[h]`.
    pub b1: [f32; HIDDEN],
    /// `W2[c][h]`.
    pub w2: Vec<[f32; HIDDEN]>,
    /// `b2[c]`.
    pub b2: [f32; CLASSES],
}

impl Mlp {
    /// Xavier-uniform weights, zero biases.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn initialize(rng: &mut SplitMix64) -> Self {
        let limit1 = (6.0 / (INPUTS + HIDDEN) as f32).sqrt();
        let limit2 = (6.0 / (HIDDEN + CLASSES) as f32).sqrt();
        let w1 = (0..HIDDEN)
            .map(|_| std::array::from_fn(|_| rng.uniform(limit1)))
            .collect();
        let w2 = (0..CLASSES)
            .map(|_| std::array::from_fn(|_| rng.uniform(limit2)))
            .collect();
        Self {
            w1,
            b1: [0.0; HIDDEN],
            w2,
            b2: [0.0; CLASSES],
        }
    }

    fn hidden(&self, features: &[f32; INPUTS]) -> [f32; HIDDEN] {
        let mut hidden = self.b1;
        for (value, row) in hidden.iter_mut().zip(&self.w1) {
            for (weight, feature) in row.iter().zip(features) {
                *value += weight * feature;
            }
            *value = value.max(0.0);
        }
        hidden
    }

    /// Output logits, accumulated in the same order as the guest.
    #[must_use]
    pub fn logits(&self, features: &[f32; INPUTS]) -> [f32; CLASSES] {
        let hidden = self.hidden(features);
        let mut logits = self.b2;
        for (logit, row) in logits.iter_mut().zip(&self.w2) {
            for (weight, value) in row.iter().zip(&hidden) {
                *logit += weight * value;
            }
        }
        logits
    }

    /// Predicted class: first index of the maximum logit.
    #[must_use]
    pub fn predict(&self, features: &[f32; INPUTS]) -> usize {
        argmax(&self.logits(features))
    }

    /// Fraction of `samples` classified correctly.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn accuracy(&self, samples: &[Sample]) -> f64 {
        if samples.is_empty() {
            return 0.0;
        }
        let correct = samples
            .iter()
            .filter(|sample| self.predict(&sample.features) == sample.label)
            .count();
        correct as f64 / samples.len() as f64
    }

    /// Serialize as little-endian `f32` in guest layout order.
    #[must_use]
    pub fn to_le_bytes(&self) -> Vec<u8> {
        let values = self
            .w1
            .iter()
            .flatten()
            .chain(&self.b1)
            .chain(self.w2.iter().flatten())
            .chain(&self.b2);
        values.flat_map(|value| value.to_le_bytes()).collect()
    }

    /// Parse the serialized layout.
    ///
    /// # Errors
    ///
    /// Returns [`DatasetError`] when the byte length is not `PARAMETERS * 4`.
    pub fn from_le_bytes(bytes: &[u8]) -> Result<Self, DatasetError> {
        if bytes.len() != PARAMETERS * 4 {
            return Err(DatasetError(format!(
                "weights must be {} bytes, got {}",
                PARAMETERS * 4,
                bytes.len()
            )));
        }
        let mut values = bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        let mut take = || values.next().unwrap_or(0.0);
        let w1 = (0..HIDDEN)
            .map(|_| std::array::from_fn(|_| take()))
            .collect();
        let b1 = std::array::from_fn(|_| take());
        let w2 = (0..CLASSES)
            .map(|_| std::array::from_fn(|_| take()))
            .collect();
        let b2 = std::array::from_fn(|_| take());
        Ok(Self { w1, b1, w2, b2 })
    }

    /// One minibatch of SGD on softmax cross-entropy.
    #[allow(clippy::cast_precision_loss)]
    fn step(&mut self, batch: &[&Sample], learning_rate: f32) {
        let mut weight_grad_in = vec![[0.0_f32; INPUTS]; HIDDEN];
        let mut bias_grad_hidden = [0.0_f32; HIDDEN];
        let mut weight_grad_out = vec![[0.0_f32; HIDDEN]; CLASSES];
        let mut bias_grad_out = [0.0_f32; CLASSES];
        for sample in batch {
            let hidden = self.hidden(&sample.features);
            let probabilities = softmax(&self.logits(&sample.features));
            let mut delta_out = probabilities;
            delta_out[sample.label] -= 1.0;
            let mut delta_hidden = [0.0_f32; HIDDEN];
            for (class, delta) in delta_out.iter().enumerate() {
                bias_grad_out[class] += delta;
                for unit in 0..HIDDEN {
                    weight_grad_out[class][unit] += delta * hidden[unit];
                    delta_hidden[unit] += delta * self.w2[class][unit];
                }
            }
            for unit in 0..HIDDEN {
                if hidden[unit] <= 0.0 {
                    continue;
                }
                bias_grad_hidden[unit] += delta_hidden[unit];
                for (grad, feature) in weight_grad_in[unit].iter_mut().zip(&sample.features) {
                    *grad += delta_hidden[unit] * feature;
                }
            }
        }
        let scale = learning_rate / batch.len() as f32;
        for (row, grads) in self.w1.iter_mut().zip(&weight_grad_in) {
            for (weight, grad) in row.iter_mut().zip(grads) {
                *weight -= scale * grad;
            }
        }
        for (bias, grad) in self.b1.iter_mut().zip(&bias_grad_hidden) {
            *bias -= scale * grad;
        }
        for (row, grads) in self.w2.iter_mut().zip(&weight_grad_out) {
            for (weight, grad) in row.iter_mut().zip(grads) {
                *weight -= scale * grad;
            }
        }
        for (bias, grad) in self.b2.iter_mut().zip(&bias_grad_out) {
            *bias -= scale * grad;
        }
    }
}

/// First index of the maximum value (ties resolve to the lowest index).
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

fn softmax(logits: &[f32; CLASSES]) -> [f32; CLASSES] {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exps = logits.map(|logit| (logit - max).exp());
    let sum: f32 = exps.iter().sum();
    exps.map(|value| value / sum)
}

/// Per-feature mean and inverse standard deviation over `samples`
/// (constant features get an inverse deviation of 1).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn standardization(samples: &[Sample]) -> ([f32; INPUTS], [f32; INPUTS]) {
    let count = samples.len().max(1) as f64;
    let mut mean = [0.0_f64; INPUTS];
    for sample in samples {
        for (sum, feature) in mean.iter_mut().zip(&sample.features) {
            *sum += f64::from(*feature);
        }
    }
    for sum in &mut mean {
        *sum /= count;
    }
    let mut variance = [0.0_f64; INPUTS];
    for sample in samples {
        for ((sum, feature), mu) in variance.iter_mut().zip(&sample.features).zip(&mean) {
            *sum += (f64::from(*feature) - mu).powi(2);
        }
    }
    #[allow(clippy::cast_possible_truncation)]
    let inverse = variance.map(|sum| {
        let deviation = (sum / count).sqrt();
        if deviation > 1e-6 {
            (1.0 / deviation) as f32
        } else {
            1.0
        }
    });
    #[allow(clippy::cast_possible_truncation)]
    (mean.map(|value| value as f32), inverse)
}

fn standardized(samples: &[Sample], mean: &[f32; INPUTS], inverse: &[f32; INPUTS]) -> Vec<Sample> {
    samples
        .iter()
        .map(|sample| {
            let mut features = sample.features;
            for ((feature, mu), inv) in features.iter_mut().zip(mean).zip(inverse) {
                *feature = (*feature - mu) * inv;
            }
            Sample {
                features,
                label: sample.label,
            }
        })
        .collect()
}

impl Mlp {
    /// Fold a standardization `(x - mean) * inverse` into the first layer so
    /// the folded model consumes raw scaled features directly.
    #[must_use]
    pub fn fold_standardization(mut self, mean: &[f32; INPUTS], inverse: &[f32; INPUTS]) -> Self {
        for (row, bias) in self.w1.iter_mut().zip(self.b1.iter_mut()) {
            for ((weight, mu), inv) in row.iter_mut().zip(mean).zip(inverse) {
                *weight *= inv;
                *bias -= *weight * mu;
            }
        }
        self
    }
}

/// Train on standardized features, then fold the standardization into the
/// first layer so the published model consumes `pixel / 16` directly.
#[must_use]
pub fn train_standardized(samples: &[Sample], config: TrainConfig) -> Mlp {
    let (mean, inverse) = standardization(samples);
    train(&standardized(samples, &mean, &inverse), config).fold_standardization(&mean, &inverse)
}

/// Train an [`Mlp`] deterministically: same samples + config ⇒ same bytes on
/// the same platform.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn train(samples: &[Sample], config: TrainConfig) -> Mlp {
    let mut rng = SplitMix64::new(config.seed);
    let mut model = Mlp::initialize(&mut rng);
    let mut order: Vec<usize> = (0..samples.len()).collect();
    let batch_size = config.batch_size.max(1);
    let epochs = config.epochs.max(1);
    for epoch in 0..epochs {
        let progress = epoch as f32 / epochs as f32;
        let learning_rate = config.learning_rate * (1.0 - 0.9 * progress);
        for index in (1..order.len()).rev() {
            let swap = rng.below(index + 1);
            order.swap(index, swap);
        }
        for chunk in order.chunks(batch_size) {
            let batch: Vec<&Sample> = chunk.iter().map(|&index| &samples[index]).collect();
            model.step(&batch, learning_rate);
        }
    }
    model
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;

    const TRAIN: &[u8] = include_bytes!("../../../fixtures/datasets/uci-optdigits/optdigits.tra");
    const TEST: &[u8] = include_bytes!("../../../fixtures/datasets/uci-optdigits/optdigits.tes");
    const WEIGHTS: &[u8] =
        include_bytes!("../../traverse-digits-mlp-guest/weights/digits-mlp-1.0.0.bin");
    /// Pinned SHA-256 of the committed `digits-mlp-1.0.0` weights.
    const WEIGHTS_SHA256: &str =
        include_str!("../../traverse-digits-mlp-guest/weights/digits-mlp-1.0.0.bin.sha256");

    #[test]
    fn vendored_dataset_matches_pinned_digests_and_shape() {
        let train = parse_verified(TRAIN, TRAIN_SHA256).expect("train split");
        let test = parse_verified(TEST, TEST_SHA256).expect("test split");
        assert_eq!(train.len(), 3823);
        assert_eq!(test.len(), 1797);
        assert!(train.iter().chain(&test).all(|sample| {
            sample
                .features
                .iter()
                .all(|feature| (0.0..=1.0).contains(feature))
        }));
        for label in 0..CLASSES {
            assert!(test.iter().any(|sample| sample.label == label));
        }
    }

    #[test]
    fn parse_rejects_tampered_or_malformed_data() {
        assert!(parse_verified(b"0,1", TRAIN_SHA256).is_err());
        let check = |row: &str| {
            let bytes = row.as_bytes();
            parse_verified(bytes, &sha256_hex(bytes)).expect_err(row)
        };
        let pixels = vec!["1"; INPUTS].join(",");
        check(&format!("{pixels},x"));
        check("");
        let _ = check(",");
        check(&format!("{},3", vec!["1"; INPUTS - 1].join(",")));
        check(&format!("{},17,3", vec!["1"; INPUTS - 1].join(",")));
        check(&format!("{pixels},10"));
        let bad_utf8 = [0xff_u8, 0xfe];
        assert!(parse_verified(&bad_utf8, &sha256_hex(&bad_utf8)).is_err());
        let ok = format!("{pixels},3\n\n");
        let parsed = parse_verified(ok.as_bytes(), &sha256_hex(ok.as_bytes())).expect("ok");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].label, 3);
        assert_eq!(DatasetError("x".into()).to_string(), "dataset error: x");
    }

    #[test]
    fn training_is_deterministic_for_a_fixed_seed() {
        let train_split = parse_verified(TRAIN, TRAIN_SHA256).expect("train split");
        let config = TrainConfig {
            epochs: 1,
            ..TrainConfig::published()
        };
        let first = train(&train_split[..512], config).to_le_bytes();
        let second = train(&train_split[..512], config).to_le_bytes();
        assert_eq!(first, second);
        let other_seed =
            train(&train_split[..512], TrainConfig { seed: 7, ..config }).to_le_bytes();
        assert_ne!(first, other_seed);
    }

    #[test]
    fn committed_weights_are_pinned_round_trip_and_clear_the_floor() {
        assert_eq!(sha256_hex(WEIGHTS), WEIGHTS_SHA256.trim());
        let model = Mlp::from_le_bytes(WEIGHTS).expect("weights");
        assert_eq!(model.to_le_bytes(), WEIGHTS);
        let test = parse_verified(TEST, TEST_SHA256).expect("test split");
        assert!(
            model.accuracy(&test) >= 0.95,
            "accuracy {}",
            model.accuracy(&test)
        );
        assert!(Mlp::from_le_bytes(&WEIGHTS[1..]).is_err());
        assert_eq!(model.accuracy(&[]), 0.0);
    }

    #[test]
    fn argmax_prefers_the_lowest_index_on_ties() {
        let mut values = [0.0_f32; CLASSES];
        assert_eq!(argmax(&values), 0);
        values[3] = 2.0;
        values[7] = 2.0;
        assert_eq!(argmax(&values), 3);
        let mut rng = SplitMix64::new(1);
        assert!(rng.below(5) < 5);
    }
    #[test]
    fn folded_standardization_preserves_predictions() {
        let train_split = parse_verified(TRAIN, TRAIN_SHA256).expect("train split");
        let subset = &train_split[..256];
        let (mean, inverse) = standardization(subset);
        // optdigits has always-zero border pixels: they keep an inverse of 1.
        assert!(inverse.contains(&1.0));
        let config = TrainConfig {
            epochs: 2,
            ..TrainConfig::published()
        };
        let unfolded = train(&standardized(subset, &mean, &inverse), config);
        let folded = train_standardized(subset, config);
        assert_eq!(
            folded,
            unfolded.clone().fold_standardization(&mean, &inverse)
        );
        let standardized_subset = standardized(subset, &mean, &inverse);
        let agree = subset
            .iter()
            .zip(&standardized_subset)
            .filter(|(raw, std)| folded.predict(&raw.features) == unfolded.predict(&std.features))
            .count();
        assert!(
            agree >= subset.len() - 2,
            "fold changed {} predictions",
            subset.len() - agree
        );
        assert_eq!(standardization(&[]).1, [1.0; INPUTS]);
    }
}
