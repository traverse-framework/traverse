#![no_std]
#![no_main]

#[repr(C)]
struct IoVec {
    buffer: *const u8,
    length: usize,
}

#[link(wasm_import_module = "wasi_snapshot_preview1")]
unsafe extern "C" {
    fn fd_write(
        file_descriptor: u32,
        vectors: *const IoVec,
        vector_count: usize,
        bytes_written: *mut usize,
    ) -> u32;
}

static OUTPUT: &[u8] = br#"{"docType":"document","parties":[],"amounts":[],"confidence":"high","recommendation":"manual_review"}"#;

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    let vector = IoVec {
        buffer: OUTPUT.as_ptr(),
        length: OUTPUT.len(),
    };
    let mut bytes_written = 0;
    // The only host interaction is the explicitly whitelisted stdout write.
    unsafe {
        let _ = fd_write(1, &vector, 1, &mut bytes_written);
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
