//! The packaging guide's supported-operator list is generated from the
//! operators the runner's pinned `tract-onnx` registers (Decision 115,
//! `#1680`). This test fails when the guide drifts from the pin; set
//! `TRAVERSE_UPDATE_ONNX_OPERATORS=1` to rewrite the block.
//!
//! An integration test, so nothing here is compiled into `runner.wasm`.
#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;

const BEGIN: &str = "<!-- onnx-operators:begin (generated; see crates/traverse-onnx-runner-guest/tests/supported_operators.rs) -->";
const END: &str = "<!-- onnx-operators:end -->";

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

/// The `tract-onnx` version this crate's lockfile pins.
fn tract_onnx_version() -> String {
    let lock = fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock"))
        .expect("Cargo.lock");
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line == "name = \"tract-onnx\"" {
            let version = lines.next().expect("version line");
            return version
                .trim_start_matches("version = \"")
                .trim_end_matches('"')
                .to_string();
        }
    }
    panic!("tract-onnx is not in Cargo.lock");
}

fn generated_block() -> String {
    let mut operators: Vec<String> = tract_onnx::onnx().op_register.0.keys().cloned().collect();
    operators.sort();
    let mut block = format!(
        "{BEGIN}\n\nThe runner registers these {} ONNX operators (`tract-onnx` {}, as pinned in\n`crates/traverse-onnx-runner-guest/Cargo.lock`):\n\n",
        operators.len(),
        tract_onnx_version()
    );
    let mut line = String::new();
    for operator in &operators {
        let item = format!("`{operator}`");
        if !line.is_empty() && line.len() + item.len() + 2 > 78 {
            block.push_str(line.trim_end());
            block.push('\n');
            line.clear();
        }
        line.push_str(&item);
        line.push_str(", ");
    }
    block.push_str(line.trim_end().trim_end_matches(','));
    block.push_str("\n\n");
    block.push_str(END);
    block
}

#[test]
fn packaging_guide_lists_the_pinned_tract_operators() {
    let guide_path = repo("docs/model-packaging-guide.md");
    let guide = fs::read_to_string(&guide_path).expect("guide");
    let start = guide.find(BEGIN).expect("guide has the operator block");
    let end = guide[start..].find(END).expect("operator block end") + start + END.len();
    let expected = generated_block();
    if std::env::var_os("TRAVERSE_UPDATE_ONNX_OPERATORS").is_some() {
        let updated = format!("{}{expected}{}", &guide[..start], &guide[end..]);
        fs::write(&guide_path, updated).expect("write guide");
        return;
    }
    assert_eq!(
        &guide[start..end],
        expected,
        "the supported-operator list is stale: run TRAVERSE_UPDATE_ONNX_OPERATORS=1 cargo test --test supported_operators in crates/traverse-onnx-runner-guest"
    );
}
