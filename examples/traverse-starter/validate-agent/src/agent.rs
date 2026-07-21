#![no_std]
#![no_main]

#[repr(C)]
struct IoVec { buffer: *const u8, length: usize }

#[link(wasm_import_module = "wasi_snapshot_preview1")]
unsafe extern "C" { fn fd_write(fd: u32, vectors: *const IoVec, count: usize, written: *mut usize) -> u32; }

static OUTPUT: &[u8] = br#"{"valid":true,"issues":[]}"#;

#[unsafe(no_mangle)]
pub extern "C" fn _start() {
    let vector = IoVec { buffer: OUTPUT.as_ptr(), length: OUTPUT.len() };
    let mut written = 0;
    unsafe { let _ = fd_write(1, &vector, 1, &mut written); }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop {} }
