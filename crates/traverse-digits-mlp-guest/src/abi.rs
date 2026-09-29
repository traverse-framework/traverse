//! Audited Spec 138 guest-ABI boundary (ADR-0077). This is the only module in
//! the crate allowed to use `unsafe`, and only to view host-staged regions of
//! this module's own linear memory as byte slices.

use crate::mlp::{OUTPUT_FRAME_LEN, execute_frame};

/// `model_execute(in_ptr, in_len, out_ptr, out_cap) -> out_len`, or `-1` on
/// any malformed input, undersized output capacity, or invalid region.
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
    if in_ptr == 0 || out_ptr == 0 || out_cap < OUTPUT_FRAME_LEN {
        return -1;
    }
    let in_end = in_ptr.saturating_add(in_len);
    let out_end = out_ptr.saturating_add(OUTPUT_FRAME_LEN);
    // The input and output regions must not overlap.
    if in_ptr < out_end && out_ptr < in_end {
        return -1;
    }
    // SAFETY: under the Spec 138 guest ABI the host staged exactly `in_len`
    // initialized bytes at `in_ptr` inside this module's linear memory before
    // calling, and nothing else aliases them during the call. The pointer is
    // non-null (checked above) and `u8` has alignment 1. An out-of-bounds
    // region traps in the wasm engine rather than reading foreign memory.
    let input = unsafe { core::slice::from_raw_parts(in_ptr as *const u8, in_len) };
    let mut frame = [0_u8; OUTPUT_FRAME_LEN];
    let Some(written) = execute_frame(input, &mut frame) else {
        return -1;
    };
    // SAFETY: the host reserved `out_cap >= OUTPUT_FRAME_LEN` writable bytes
    // at `out_ptr` in this module's linear memory; the region is non-null and
    // disjoint from the input (checked above), and `u8` has alignment 1.
    let output = unsafe { core::slice::from_raw_parts_mut(out_ptr as *mut u8, OUTPUT_FRAME_LEN) };
    output.copy_from_slice(&frame);
    i32::try_from(written).unwrap_or(-1)
}
