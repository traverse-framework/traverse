// Copies the ONNX model named by $BIRDNET_ONNX into OUT_DIR so it is embedded
// in the wasm at build time. The weights never enter the repository.
fn main() {
    let source = std::env::var("BIRDNET_ONNX").expect("set BIRDNET_ONNX to a local .onnx path");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::copy(&source, out.join("model.onnx")).expect("copy model");
    println!("cargo:rerun-if-env-changed=BIRDNET_ONNX");
    println!("cargo:rerun-if-changed={source}");
}
