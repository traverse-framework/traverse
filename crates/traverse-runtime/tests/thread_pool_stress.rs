//! Stress and platform-stability tests for ThreadPoolExecutor.
//!
//! Governed by spec `047-thread-pool-executor`.
//!
//! All tests are marked `#[ignore]` — they run only when explicitly invoked:
//!   cargo test -p traverse-runtime --test thread_pool_stress -- --ignored --nocapture
//!
//! In CI these run via the `stress-test` matrix job across 5 platforms.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use traverse_runtime::executor::{
    ArtifactType, CapabilityExecutor, ExecutorCapability, ExecutorError, NativeExecutor,
    ThreadPoolExecutor, ThreadPoolExecutorConfig,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn echo_cap() -> ExecutorCapability {
    ExecutorCapability {
        capability_id: "stress.echo".to_string(),
        artifact_type: ArtifactType::Native,
        wasm_binary_path: None,
        wasm_checksum: None,
        host_abi_version: None,
    }
}

fn build_pool(
    capacity: usize,
    handler: impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
) -> ThreadPoolExecutor {
    ThreadPoolExecutor::new(
        ThreadPoolExecutorConfig { capacity },
        Box::new(NativeExecutor::new(handler)),
    )
    .unwrap_or_else(|e| panic!("pool build failed: {e}"))
}

// ---------------------------------------------------------------------------
// Throughput and correctness under load
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn stress_50_threads_burst() {
    let pool = Arc::new(build_pool(8, |input| Ok(input.clone())));
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let start = Instant::now();

    std::thread::scope(|s| {
        for i in 0_u32..50 {
            let pool = Arc::clone(&pool);
            let errors = Arc::clone(&errors);
            s.spawn(move || {
                let input = json!({ "i": i });
                match pool.execute(&echo_cap(), &input) {
                    Ok(out) if out == input => {}
                    Ok(out) => errors
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(format!("thread {i}: wrong output {out}")),
                    Err(e) => errors
                        .lock()
                        .unwrap_or_else(|e2| e2.into_inner())
                        .push(format!("thread {i}: error {e}")),
                }
            });
        }
    });

    let elapsed = start.elapsed();
    let errors = errors.lock().unwrap_or_else(|e| e.into_inner());
    assert!(errors.is_empty(), "burst errors: {errors:?}");
    assert!(elapsed < Duration::from_secs(5), "burst took too long: {elapsed:?}");
}

#[test]
#[ignore]
fn stress_10k_sequential_calls() {
    let pool = build_pool(4, |input| Ok(input.clone()));
    let cap = echo_cap();
    for i in 0_u32..10_000 {
        let result = pool.execute(&cap, &json!({ "i": i }));
        assert!(result.is_ok(), "call {i} failed: {result:?}");
        assert_eq!(result.ok(), Some(json!({ "i": i })));
    }
}

#[test]
#[ignore]
fn stress_mixed_success_and_error() {
    let pool = Arc::new(build_pool(8, |input| {
        if input.get("fail").is_some() {
            Err("deliberate".to_string())
        } else {
            Ok(input.clone())
        }
    }));

    let success_count = Arc::new(Mutex::new(0_u32));
    let error_count = Arc::new(Mutex::new(0_u32));

    std::thread::scope(|s| {
        for i in 0_u32..1_000 {
            let pool = Arc::clone(&pool);
            let success_count = Arc::clone(&success_count);
            let error_count = Arc::clone(&error_count);
            s.spawn(move || {
                let input = if i % 2 == 0 {
                    json!({ "i": i })
                } else {
                    json!({ "fail": true })
                };
                match pool.execute(&echo_cap(), &input) {
                    Ok(_) => *success_count.lock().unwrap_or_else(|e| e.into_inner()) += 1,
                    Err(_) => *error_count.lock().unwrap_or_else(|e| e.into_inner()) += 1,
                }
            });
        }
    });

    let s = *success_count.lock().unwrap_or_else(|e| e.into_inner());
    let e = *error_count.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(s, 500, "expected 500 successes, got {s}");
    assert_eq!(e, 500, "expected 500 errors, got {e}");
}

#[test]
#[ignore]
fn stress_interleaved_panic_and_success() {
    let pool = Arc::new(build_pool(4, |input| {
        if input.get("panic").is_some() {
            panic!("deliberate panic in stress test");
        }
        Ok(input.clone())
    }));

    let success_count = Arc::new(Mutex::new(0_u32));
    let panic_count = Arc::new(Mutex::new(0_u32));

    std::thread::scope(|s| {
        for i in 0_u32..200 {
            let pool = Arc::clone(&pool);
            let success_count = Arc::clone(&success_count);
            let panic_count = Arc::clone(&panic_count);
            s.spawn(move || {
                let input = if i % 5 == 0 {
                    json!({ "panic": true })
                } else {
                    json!({ "i": i })
                };
                match pool.execute(&echo_cap(), &input) {
                    Ok(_) => *success_count.lock().unwrap_or_else(|e| e.into_inner()) += 1,
                    Err(ExecutorError::ExecutionFailed(msg)) if msg.contains("panicked") => {
                        *panic_count.lock().unwrap_or_else(|e| e.into_inner()) += 1;
                    }
                    Err(e) => panic!("unexpected error: {e}"),
                }
            });
        }
    });

    let s = *success_count.lock().unwrap_or_else(|e| e.into_inner());
    let p = *panic_count.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(s, 160, "expected 160 successes, got {s}");
    assert_eq!(p, 40, "expected 40 panic errors, got {p}");
}

// ---------------------------------------------------------------------------
// Pool capacity boundary
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn stress_capacity_1_no_deadlock() {
    let pool = build_pool(1, |input| Ok(input.clone()));
    let cap = echo_cap();
    let start = Instant::now();
    for i in 0_u32..100 {
        let result = pool.execute(&cap, &json!({ "i": i }));
        assert!(result.is_ok(), "call {i} failed: {result:?}");
    }
    let elapsed = start.elapsed();
    assert!(elapsed < Duration::from_secs(10), "capacity-1 took too long: {elapsed:?}");
}

#[test]
#[ignore]
fn stress_capacity_256_max() {
    let pool = Arc::new(build_pool(256, |input| Ok(input.clone())));
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));

    std::thread::scope(|s| {
        for i in 0_u32..256 {
            let pool = Arc::clone(&pool);
            let errors = Arc::clone(&errors);
            s.spawn(move || {
                match pool.execute(&echo_cap(), &json!({ "i": i })) {
                    Ok(_) => {}
                    Err(e) => errors
                        .lock()
                        .unwrap_or_else(|e2| e2.into_inner())
                        .push(format!("thread {i}: {e}")),
                }
            });
        }
    });

    let errors = errors.lock().unwrap_or_else(|e| e.into_inner());
    assert!(errors.is_empty(), "capacity-256 errors: {errors:?}");
}

// ---------------------------------------------------------------------------
// Drop and lifecycle
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn stress_no_fd_leak() {
    fn open_fd_count() -> usize {
        #[cfg(target_os = "linux")]
        {
            std::fs::read_dir("/proc/self/fd")
                .map(|d| d.count())
                .unwrap_or(0)
        }
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            let pid = std::process::id();
            Command::new("lsof")
                .args(["-p", &pid.to_string()])
                .output()
                .map(|o| o.stdout.iter().filter(|&&b| b == b'\n').count())
                .unwrap_or(0)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            0
        }
    }

    let baseline = open_fd_count();

    for _ in 0..1_000 {
        let pool = build_pool(2, |input| Ok(input.clone()));
        let _ = pool.execute(&echo_cap(), &json!({}));
        drop(pool);
    }

    let after = open_fd_count();

    // Allow a small absolute tolerance for OS-level variance.
    // On Linux/macOS where counting works, the count must not grow unboundedly.
    // Skip the assertion on Windows where open_fd_count returns 0.
    if baseline > 0 {
        assert!(
            after <= baseline + 10,
            "potential fd leak: baseline={baseline}, after={after}"
        );
    }
}

#[test]
#[ignore]
fn stress_memory_stable_60s() {
    fn rss_kb() -> u64 {
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.starts_with("VmRSS:"))
                        .and_then(|l| l.split_whitespace().nth(1))
                        .and_then(|v| v.parse().ok())
                })
                .unwrap_or(0)
        }
        #[cfg(not(target_os = "linux"))]
        {
            0
        }
    }

    let pool = Arc::new(build_pool(4, |input| Ok(input.clone())));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let samples: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(vec![]));

    // Sampler thread — records RSS every 5s for 60s
    let stop_clone = Arc::clone(&stop);
    let samples_clone = Arc::clone(&samples);
    let sampler = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            samples_clone
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(rss_kb());
            std::thread::sleep(Duration::from_secs(5));
        }
        stop_clone.store(true, std::sync::atomic::Ordering::Relaxed);
    });

    // Worker threads — continuous burst for 60s
    std::thread::scope(|s| {
        for _ in 0..4 {
            let pool = Arc::clone(&pool);
            let stop = Arc::clone(&stop);
            s.spawn(move || {
                let mut i = 0_u32;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = pool.execute(&echo_cap(), &json!({ "i": i }));
                    i = i.wrapping_add(1);
                }
            });
        }
    });

    sampler.join().unwrap_or_else(|_| {});

    let samples = samples.lock().unwrap_or_else(|e| e.into_inner());
    // On Linux, verify RSS doesn't grow monotonically. On other platforms samples are 0 — skip.
    if samples.iter().any(|&s| s > 0) && samples.len() >= 3 {
        let first = samples[0];
        let last = *samples.last().unwrap_or(&0);
        // Allow up to 2x growth (generous bound — actual leaks grow much faster)
        assert!(
            last <= first * 2 + 1024,
            "RSS grew too much: first={first}KB last={last}KB"
        );
    }
}
