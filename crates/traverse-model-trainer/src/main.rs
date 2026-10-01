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

fn export_onnx() -> Result<(), String> {
    let weights = "crates/traverse-digits-mlp-guest/weights/digits-mlp-1.0.0.bin";
    let bytes =
        std::fs::read(repo_path(weights)).map_err(|error| format!("read {weights}: {error}"))?;
    let model =
        traverse_model_trainer::Mlp::from_le_bytes(&bytes).map_err(|error| error.to_string())?;
    let onnx = traverse_model_trainer::onnx::mlp_to_onnx(&model);
    let path = "fixtures/onnx/digits-mlp-1.0.0.onnx";
    std::fs::create_dir_all(repo_path("fixtures/onnx")).map_err(|error| error.to_string())?;
    std::fs::write(repo_path(path), &onnx).map_err(|error| format!("write {path}: {error}"))?;
    println!(
        "wrote {path} ({} bytes) sha256 {}",
        onnx.len(),
        sha256_hex(&onnx)
    );
    Ok(())
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
    let result = if std::env::args().nth(1).as_deref() == Some("export-onnx") {
        export_onnx()
    } else {
        run()
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
