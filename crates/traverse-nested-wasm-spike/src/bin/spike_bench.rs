//! Native host for the #1403 nested-wasmi spike.
//!
//! Times `execute_capability` against a fixture (default: echo WAT) or an
//! explicit capability wasm path.

use std::env;
use std::fs;
use std::time::Instant;
use traverse_nested_wasm_spike::execute_capability;

const ECHO_WAT: &str = r#"
  (module
    (import "wasi_snapshot_preview1" "fd_read"
      (func $fd_read (param i32 i32 i32 i32) (result i32)))
    (import "wasi_snapshot_preview1" "fd_write"
      (func $fd_write (param i32 i32 i32 i32) (result i32)))
    (memory (export "memory") 1)
    (func (export "_start")
      (i32.store (i32.const 0) (i32.const 8))
      (i32.store (i32.const 4) (i32.const 1024))
      (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 4100)))
      (i32.store (i32.const 0) (i32.const 8))
      (i32.store (i32.const 4) (i32.load (i32.const 4100)))
      (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 4104)))
    )
  )
"#;

fn main() {
    let iterations: u32 = env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(50);
    let artifact_arg = env::args().nth(2);
    let artifact = match artifact_arg.as_deref() {
        Some(path) => fs::read(path).unwrap_or_else(|error| {
            eprintln!("failed to read artifact {path}: {error}");
            std::process::exit(2);
        }),
        None => wat::parse_str(ECHO_WAT).unwrap_or_else(|error| {
            eprintln!("failed to parse echo WAT: {error}");
            std::process::exit(2);
        }),
    };
    // say-hello-agent ignores stdin and writes a fixed greeting; echo expects
    // the input bytes back. Choose the expected stdout accordingly.
    let input = br#"{"hello":"nested-spike","n":1}"#;
    let expect_echo = artifact_arg.is_none();

    if let Err(error) = execute_capability(&artifact, input) {
        eprintln!("warmup failed: {error}");
        std::process::exit(1);
    }

    let started = Instant::now();
    let mut last = Vec::new();
    for _ in 0..iterations {
        last = execute_capability(&artifact, input).unwrap_or_else(|error| {
            eprintln!("execute failed: {error}");
            std::process::exit(1);
        });
        if expect_echo && last != input {
            eprintln!("stdout mismatch for echo fixture");
            std::process::exit(1);
        }
    }
    let elapsed = started.elapsed();
    let per_call_us = elapsed.as_secs_f64() * 1_000_000.0 / f64::from(iterations);
    println!("nested_wasmi_iterations={iterations}");
    println!(
        "nested_wasmi_total_ms={:.3}",
        elapsed.as_secs_f64() * 1000.0
    );
    println!("nested_wasmi_per_call_us={per_call_us:.1}");
    println!("artifact_bytes={}", artifact.len());
    println!("stdout_bytes={}", last.len());
}
