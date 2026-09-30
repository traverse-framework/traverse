//! Host-native probe: can tract load and run BirdNET v2.4 int8?
//! Usage: birdnet-onnx-spike <model.onnx> [input.f32le]
use std::time::Instant;
use tract_onnx::prelude::*;

fn main() -> TractResult<()> {
    let args: Vec<String> = std::env::args().collect();
    let model_path = &args[1];
    let started = Instant::now();
    let inference = tract_onnx::onnx().model_for_path(model_path)?;
    eprintln!("input fact: {:?}", inference.input_fact(0)?);
    let typed = inference.into_typed()?;
    let batch = typed.symbols.sym("batch");
    let model = typed
        .concretize_dims(&SymbolValues::default().with(&batch, 1))?
        .into_optimized()?
        .into_runnable()?;
    eprintln!("load+optimize: {:?}", started.elapsed());
    // Batch mode: <model.onnx> <clips_dir> <out_dir> writes <clip>.tract.f32le.
    if let (Some(clips), Some(out)) = (args.get(2), args.get(3)) {
        let mut names: Vec<_> = std::fs::read_dir(clips)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "f32le"))
            .collect();
        names.sort();
        let mut total = std::time::Duration::ZERO;
        for path in &names {
            let samples: Vec<f32> = std::fs::read(path)?
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            let input: Tensor = tract_ndarray::Array2::from_shape_vec((1, 144_000), samples)?.into();
            let started = Instant::now();
            let result = model.run(tvec!(input.into()))?;
            total += started.elapsed();
            let bytes: Vec<u8> = result[0]
                .as_slice::<f32>()?
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("clip");
            std::fs::write(format!("{out}/{stem}.tract.f32le"), bytes)?;
        }
        eprintln!("clips: {} mean run: {:?}", names.len(), total / names.len().max(1) as u32);
        return Ok(());
    }
    let samples = vec![0.0_f32; 144_000];
    let input: Tensor = tract_ndarray::Array2::from_shape_vec((1, 144_000), samples)?.into();
    let started = Instant::now();
    let result = model.run(tvec!(input.into()))?;
    eprintln!("run: {:?}", started.elapsed());
    let scores = result[0].to_array_view::<f32>()?;
    println!("{}", scores.iter().take(8).map(|v| format!("{v:.6}")).collect::<Vec<_>>().join(","));
    Ok(())
}
