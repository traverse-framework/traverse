//! Audited Spec 138 guest ABI v2 boundary (ADR-0077 pattern, Decision 105).
//! The only module allowed `unsafe`: it reads the packager-patched blob
//! location, and views host-staged regions of this module's own memory.

use crate::runner::{parse_blob, Model};
use std::sync::OnceLock;

/// `[blob_ptr, blob_len]`, patched by `traverse-cli model package-onnx`. The
/// sentinel (`u32::MAX`) keeps it in the data segment and means "no model".
#[unsafe(no_mangle)]
pub static TRAVERSE_MODEL_BLOB: [u32; 2] = [u32::MAX, u32::MAX];

static MODEL: OnceLock<Option<Model>> = OnceLock::new();

/// Load and optimize the packaged model. `model_execute` does not call this:
/// ABI v3 hosts run it once under `max_prepare_fuel`, then snapshot memory.
fn load_model() -> Option<Model> {
    // SAFETY: volatile reads of this module's own static, so the
    // packager-patched values are not constant-folded away.
    let [ptr, len] = unsafe { core::ptr::read_volatile(&raw const TRAVERSE_MODEL_BLOB) };
    if ptr == u32::MAX || ptr == 0 {
        return None;
    }
    // SAFETY: the packager appended exactly `len` blob bytes at `ptr`
    // in an active data segment of this module's linear memory; they
    // are never written afterwards (the allocator only grows past them).
    let bytes: &'static [u8] =
        unsafe { core::slice::from_raw_parts(ptr as usize as *const u8, len as usize) };
    Model::load(&parse_blob(bytes)?)
}

fn prepared() -> Option<&'static Model> {
    MODEL.get().and_then(Option::as_ref)
}

/// Guest ABI v2: allocate `len` bytes for a host-staged buffer.
#[unsafe(no_mangle)]
pub extern "C" fn model_alloc(len: i32) -> i32 {
    let Ok(len) = usize::try_from(len) else {
        return 0;
    };
    let buffer = vec![0_u8; len.max(1)].into_boxed_slice();
    i32::try_from(Box::leak(buffer).as_mut_ptr() as usize).unwrap_or(0)
}

/// ABI v3 prepare: tract load and optimize. `0` means prepared.
#[unsafe(no_mangle)]
pub extern "C" fn model_prepare() -> i32 {
    if MODEL.get_or_init(load_model).is_some() {
        0
    } else {
        -1
    }
}

/// `model_execute(in_ptr, in_len, out_ptr, out_cap) -> out_len`, or `-1`.
#[unsafe(no_mangle)]
pub extern "C" fn model_execute(in_ptr: i32, in_len: i32, out_ptr: i32, out_cap: i32) -> i32 {
    let (Ok(in_ptr), Ok(in_len), Ok(out_ptr), Ok(out_cap)) = (
        usize::try_from(in_ptr),
        usize::try_from(in_len),
        usize::try_from(out_ptr),
        usize::try_from(out_cap),
    ) else {
        return -1;
    };
    if in_ptr == 0 || out_ptr == 0 {
        return -1;
    }
    let Some(model) = prepared() else { return -1 };
    // SAFETY: under guest ABI v2 the host wrote `in_len` bytes into a region
    // this module's `model_alloc` returned; it is non-null and not aliased.
    let input = unsafe { core::slice::from_raw_parts(in_ptr as *const u8, in_len) };
    let Some(frame) = model.run_frame(input) else {
        return -1;
    };
    if frame.len() > out_cap {
        return -1;
    }
    // SAFETY: the host reserved `out_cap >= frame.len()` bytes at `out_ptr`
    // via `model_alloc`, disjoint from the input (host-checked).
    let output = unsafe { core::slice::from_raw_parts_mut(out_ptr as *mut u8, frame.len()) };
    output.copy_from_slice(&frame);
    i32::try_from(frame.len()).unwrap_or(-1)
}
