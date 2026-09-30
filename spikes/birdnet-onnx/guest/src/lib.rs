//! #1590 spike guest (not product code): guest ABI v2 prototype.
//! Exports `model_alloc(len) -> ptr` and
//! `model_execute(in_ptr, in_len, out_ptr, out_cap) -> out_len`.
//! Input: 144_000 little-endian f32 samples; output: raw f32 logits.
#![allow(unsafe_code, clippy::missing_safety_doc)]
use std::sync::OnceLock;
use tract_onnx::prelude::*;

// Deterministic, import-free: any randomness request fails instead of
// importing a host entropy source.
fn no_entropy(_: &mut [u8]) -> Result<(), getrandom::Error> {
    Err(getrandom::Error::UNSUPPORTED)
}
getrandom::register_custom_getrandom!(no_entropy);

static MODEL_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/model.onnx"));
type Plan = TypedRunnableModel<TypedModel>;
static PLAN: OnceLock<Option<Plan>> = OnceLock::new();

fn plan() -> Option<&'static Plan> {
    PLAN.get_or_init(|| {
        let inference = tract_onnx::onnx()
            .model_for_read(&mut std::io::Cursor::new(MODEL_BYTES))
            .ok()?;
        let typed = inference.into_typed().ok()?;
        let batch = typed.symbols.sym("batch");
        typed
            .concretize_dims(&SymbolValues::default().with(&batch, 1))
            .ok()?
            .into_optimized()
            .ok()?
            .into_runnable()
            .ok()
    })
    .as_ref()
}

#[unsafe(no_mangle)]
pub extern "C" fn model_alloc(len: i32) -> i32 {
    let mut buffer = vec![0_u8; len.max(0) as usize].into_boxed_slice();
    let ptr = buffer.as_mut_ptr() as i32;
    std::mem::forget(buffer);
    ptr
}

/// Warm-up: load and optimize the model (so per-clip timing excludes it).
#[unsafe(no_mangle)]
pub extern "C" fn model_prepare() -> i32 {
    if plan().is_some() { 0 } else { -1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn model_execute(in_ptr: i32, in_len: i32, out_ptr: i32, out_cap: i32) -> i32 {
    let Some(plan) = plan() else { return -1 };
    let input = unsafe { std::slice::from_raw_parts(in_ptr as *const u8, in_len as usize) };
    if input.len() != 144_000 * 4 { return -2; }
    let samples: Vec<f32> = input.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let Ok(array) = tract_ndarray::Array2::from_shape_vec((1, 144_000), samples) else { return -3 };
    let tensor: Tensor = array.into();
    let Ok(result) = plan.run(tvec!(tensor.into())) else { return -4 };
    let Ok(scores) = result[0].as_slice::<f32>() else { return -5 };
    let bytes = scores.len() * 4;
    if bytes > out_cap as usize { return -6; }
    let out = unsafe { std::slice::from_raw_parts_mut(out_ptr as *mut u8, bytes) };
    for (slot, value) in out.chunks_exact_mut(4).zip(scores) { slot.copy_from_slice(&value.to_le_bytes()); }
    bytes as i32
}
