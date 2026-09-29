//! `traverse-model-trainer`: retrain the committed `digits-mlp-1.0.0`
//! weights from the vendored dataset (Decision 102). Offline and seeded.
//!
//! ```text
//! cargo run --release -p traverse-model-trainer
//! ```

use std::path::PathBuf;
use std::process::ExitCode;
use traverse_model_trainer::{
    TEST_SHA256, TRAIN_SHA256, TrainConfig, parse_verified, sha256_hex, train_standardized,
};

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn run() -> Result<(), String> {
    let read = |relative: &str| {
        std::fs::read(repo_path(relative)).map_err(|error| format!("read {relative}: {error}"))
    };
    let train_split = parse_verified(
        &read("fixtures/datasets/uci-optdigits/optdigits.tra")?,
        TRAIN_SHA256,
    )
    .map_err(|error| error.to_string())?;
    let test_split = parse_verified(
        &read("fixtures/datasets/uci-optdigits/optdigits.tes")?,
        TEST_SHA256,
    )
    .map_err(|error| error.to_string())?;

    let config = TrainConfig::published();
    let model = train_standardized(&train_split, config);
    let bytes = model.to_le_bytes();
    let digest = sha256_hex(&bytes);
    let weights = "crates/traverse-digits-mlp-guest/weights/digits-mlp-1.0.0.bin";
    std::fs::write(repo_path(weights), &bytes)
        .map_err(|error| format!("write weights: {error}"))?;
    std::fs::write(
        repo_path(&format!("{weights}.sha256")),
        format!("{digest}\n"),
    )
    .map_err(|error| format!("write digest: {error}"))?;
    println!("config: {config:?}");
    println!("train accuracy: {:.4}", model.accuracy(&train_split));
    println!("test accuracy:  {:.4}", model.accuracy(&test_split));
    println!("weights sha256: {digest}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
