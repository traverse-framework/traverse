//! Ignored stress tests for `ThreadPoolExecutor`.
//!
//! Governed by spec `047-thread-pool-executor`.

use std::error::Error;
use std::process::Command;
use std::sync::{
    Arc, Barrier, Mutex, PoisonError,
    atomic::{AtomicUsize, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use traverse_runtime::executor::{
    ArtifactType, CapabilityExecutor, ExecutorCapability, ExecutorError, NativeExecutor,
    ThreadPoolExecutor, ThreadPoolExecutorConfig,
};

type TestResult<T> = Result<T, Box<dyn Error>>;

fn native_capability() -> ExecutorCapability {
    ExecutorCapability {
        capability_id: "stress.thread_pool.native".to_string(),
        artifact_type: ArtifactType::Native,
        wasm_binary_path: None,
        wasm_checksum: None,
        host_abi_version: None,
    }
}

fn executor(
    capacity: usize,
    handler: impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
) -> TestResult<ThreadPoolExecutor> {
    Ok(ThreadPoolExecutor::new(
        ThreadPoolExecutorConfig { capacity },
        Box::new(NativeExecutor::new(handler)),
    )?)
}

fn execute(executor: &ThreadPoolExecutor, input: &Value) -> Result<Value, ExecutorError> {
    executor.execute(&native_capability(), input)
}

fn lock_errors(errors: &Mutex<Vec<String>>) -> std::sync::MutexGuard<'_, Vec<String>> {
    errors.lock().unwrap_or_else(PoisonError::into_inner)
}

fn configured_duration(default_seconds: u64) -> Duration {
    std::env::var("TRAVERSE_STRESS_SECONDS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map_or_else(|| Duration::from_secs(default_seconds), Duration::from_secs)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn parse_first_number(raw: &[u8]) -> Option<u64> {
    String::from_utf8_lossy(raw)
        .split_whitespace()
        .find_map(|part| part.trim().parse::<u64>().ok())
}

#[cfg(target_os = "linux")]
fn current_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u64>().ok())
}

#[cfg(target_os = "macos")]
fn current_rss_kb() -> Option<u64> {
    let pid = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_first_number(&output.stdout)
}

#[cfg(target_os = "windows")]
fn current_rss_kb() -> Option<u64> {
    let pid = std::process::id().to_string();
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {pid}).WorkingSet64"),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_first_number(&output.stdout).map(|bytes| bytes / 1024)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn current_rss_kb() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn current_fd_or_handle_count() -> Option<u64> {
    Some(std::fs::read_dir("/proc/self/fd").ok()?.count() as u64)
}

#[cfg(target_os = "macos")]
fn current_fd_or_handle_count() -> Option<u64> {
    let pid = std::process::id().to_string();
    let output = Command::new("lsof").args(["-p", &pid]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .skip(1)
            .count() as u64,
    )
}

#[cfg(target_os = "windows")]
fn current_fd_or_handle_count() -> Option<u64> {
    let pid = std::process::id().to_string();
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {pid}).HandleCount"),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_first_number(&output.stdout)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn current_fd_or_handle_count() -> Option<u64> {
    None
}

/// Measured create-use-drop cycles in `stress_no_fd_leak`. A real per-cycle
/// leak grows the descriptor count by hundreds over this many cycles, far
/// beyond `FD_LEAK_TOLERANCE`.
const FD_LEAK_CYCLES: u64 = 1_000;

/// Cycles run before the baseline is captured, so lazy process-wide
/// initialization (stdio, threading, platform probes) opens its descriptors
/// outside the measured window.
const FD_LEAK_WARMUP_CYCLES: u64 = 50;

/// Allowance for descriptor churn unrelated to the executor (test-harness
/// threads, background activity on shared CI runners). Must stay far below
/// the growth a genuine per-cycle leak produces over `FD_LEAK_CYCLES`.
const FD_LEAK_TOLERANCE: u64 = 8;

/// Samples taken per measurement; the minimum filters out descriptors that
/// unrelated runner activity opens transiently during sampling.
const FD_SAMPLES_PER_MEASUREMENT: u32 = 5;

/// Re-measurements allowed before the growth assertion fails, giving
/// transient descriptor churn time to settle.
const FD_MEASUREMENT_RETRIES: u32 = 3;

fn run_fd_leak_cycles(cycles: std::ops::Range<u64>) -> TestResult<()> {
    for i in cycles {
        let executor = executor(2, |input| Ok(input.clone()))?;
        let input = json!({ "cycle": i });
        let output = execute(&executor, &input)?;
        assert_eq!(output, input);
    }
    Ok(())
}

fn stable_fd_or_handle_count() -> Option<u64> {
    let mut stable: Option<u64> = None;
    for sample in 0..FD_SAMPLES_PER_MEASUREMENT {
        if sample > 0 {
            thread::sleep(Duration::from_millis(100));
        }
        if let Some(count) = current_fd_or_handle_count() {
            stable = Some(stable.map_or(count, |current| current.min(count)));
        }
    }
    stable
}

#[ignore = "runs only in the dedicated ThreadPoolExecutor stress CI job"]
#[test]
fn stress_50_threads_burst() -> TestResult<()> {
    let executor = Arc::new(executor(8, |input| Ok(input.clone()))?);
    let barrier = Arc::new(Barrier::new(51));
    let errors = Arc::new(Mutex::new(Vec::new()));
    let start = Instant::now();

    thread::scope(|scope| {
        for i in 0_u64..50 {
            let executor = Arc::clone(&executor);
            let barrier = Arc::clone(&barrier);
            let errors = Arc::clone(&errors);
            scope.spawn(move || {
                barrier.wait();
                let expected = json!({ "i": i });
                match execute(&executor, &expected) {
                    Ok(output) if output == expected => {}
                    Ok(output) => lock_errors(&errors).push(format!("wrong output {output:?}")),
                    Err(err) => lock_errors(&errors).push(format!("executor error {err:?}")),
                }
            });
        }
        barrier.wait();
    });

    let elapsed = start.elapsed();
    let errors = lock_errors(&errors);
    assert!(errors.is_empty(), "burst errors: {errors:?}");
    assert!(elapsed < Duration::from_secs(5), "burst took {elapsed:?}");
    Ok(())
}

#[ignore = "runs only in the dedicated ThreadPoolExecutor stress CI job"]
#[test]
fn stress_10k_sequential_calls() -> TestResult<()> {
    let executor = executor(4, |input| Ok(input.clone()))?;

    for i in 0_u64..10_000 {
        let expected = json!({ "i": i });
        let output = execute(&executor, &expected)?;
        assert_eq!(output, expected);
    }

    Ok(())
}

#[ignore = "runs only in the dedicated ThreadPoolExecutor stress CI job"]
#[test]
fn stress_mixed_success_and_error() -> TestResult<()> {
    let executor = executor(8, |input| {
        if input.get("should_error").and_then(Value::as_bool) == Some(true) {
            return Err("deliberate stress error".to_string());
        }
        Ok(input.clone())
    })?;

    let success_count = AtomicUsize::new(0);
    let error_count = AtomicUsize::new(0);

    for i in 0_u64..1_000 {
        let should_error = i % 2 == 1;
        let input = json!({ "i": i, "should_error": should_error });
        match execute(&executor, &input) {
            Ok(output) if !should_error && output == input => {
                success_count.fetch_add(1, Ordering::SeqCst);
            }
            Err(ExecutorError::ExecutionFailed(msg))
                if should_error && msg.contains("deliberate stress error") =>
            {
                error_count.fetch_add(1, Ordering::SeqCst);
            }
            other => return Err(format!("unexpected mixed result for {i}: {other:?}").into()),
        }
    }

    assert_eq!(success_count.load(Ordering::SeqCst), 500);
    assert_eq!(error_count.load(Ordering::SeqCst), 500);
    Ok(())
}

#[ignore = "runs only in the dedicated ThreadPoolExecutor stress CI job"]
#[test]
fn stress_interleaved_panic_and_success() -> TestResult<()> {
    let executor = executor(4, |input| {
        assert!(
            input.get("panic").and_then(Value::as_bool) != Some(true),
            "deliberate stress panic"
        );
        Ok(input.clone())
    })?;

    let success_count = AtomicUsize::new(0);
    let panic_count = AtomicUsize::new(0);

    for i in 0_u64..200 {
        let should_panic = i % 5 == 0;
        let input = json!({ "i": i, "panic": should_panic });
        match execute(&executor, &input) {
            Ok(output) if !should_panic && output == input => {
                success_count.fetch_add(1, Ordering::SeqCst);
            }
            Err(ExecutorError::ExecutionFailed(msg))
                if should_panic && msg.contains("panicked") =>
            {
                panic_count.fetch_add(1, Ordering::SeqCst);
            }
            other => return Err(format!("unexpected panic mix result for {i}: {other:?}").into()),
        }
    }

    let final_input = json!({ "after": "panic" });
    let final_output = execute(&executor, &final_input)?;
    assert_eq!(final_output, json!({ "after": "panic" }));
    assert_eq!(success_count.load(Ordering::SeqCst), 160);
    assert_eq!(panic_count.load(Ordering::SeqCst), 40);
    Ok(())
}

#[ignore = "runs only in the dedicated ThreadPoolExecutor stress CI job"]
#[test]
fn stress_memory_stable_60s() -> TestResult<()> {
    let executor = Arc::new(executor(4, |input| Ok(input.clone()))?);
    let duration = configured_duration(60);
    let deadline = Instant::now() + duration;
    let errors = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let baseline = current_rss_kb();
    let mut samples = Vec::new();

    thread::scope(|scope| {
        for worker in 0_u64..4 {
            let executor = Arc::clone(&executor);
            let errors = Arc::clone(&errors);
            let calls = Arc::clone(&calls);
            scope.spawn(move || {
                let mut iteration = 0_u64;
                while Instant::now() < deadline {
                    let input = json!({ "worker": worker, "iteration": iteration });
                    match execute(&executor, &input) {
                        Ok(output) if output == input => {
                            calls.fetch_add(1, Ordering::SeqCst);
                        }
                        Ok(output) => lock_errors(&errors).push(format!("wrong output {output:?}")),
                        Err(err) => lock_errors(&errors).push(format!("executor error {err:?}")),
                    }
                    iteration = iteration.saturating_add(1);
                }
            });
        }

        while Instant::now() < deadline {
            thread::sleep(Duration::from_secs(5).min(duration));
            if let Some(sample) = current_rss_kb() {
                samples.push(sample);
            }
        }
    });

    let errors = lock_errors(&errors);
    assert!(errors.is_empty(), "memory stress errors: {errors:?}");
    assert!(
        calls.load(Ordering::SeqCst) > 0,
        "memory stress made no executor calls"
    );

    if let Some(baseline) = baseline {
        let peak = samples.iter().copied().max().unwrap_or(baseline);
        let allowed = baseline + (baseline / 10) + 8 * 1024;
        assert!(
            peak <= allowed,
            "RSS grew beyond threshold: baseline={baseline}KiB peak={peak}KiB allowed={allowed}KiB"
        );
    }

    Ok(())
}

#[ignore = "runs only in the dedicated ThreadPoolExecutor stress CI job"]
#[test]
fn stress_no_fd_leak() -> TestResult<()> {
    run_fd_leak_cycles(0..FD_LEAK_WARMUP_CYCLES)?;
    let Some(baseline) = stable_fd_or_handle_count() else {
        return Ok(());
    };

    run_fd_leak_cycles(FD_LEAK_WARMUP_CYCLES..FD_LEAK_WARMUP_CYCLES + FD_LEAK_CYCLES)?;

    let allowed = baseline + FD_LEAK_TOLERANCE;
    let mut after = None;
    for attempt in 0..=FD_MEASUREMENT_RETRIES {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(500));
        }
        after = stable_fd_or_handle_count();
        match after {
            Some(count) if count > allowed => {}
            _ => break,
        }
    }

    if let Some(after) = after {
        assert!(
            after <= allowed,
            "file descriptor or handle count grew beyond tolerance over \
             {FD_LEAK_CYCLES} create-use-drop cycles: \
             baseline={baseline} after={after} allowed={allowed}"
        );
    }

    Ok(())
}

#[ignore = "runs only in the dedicated ThreadPoolExecutor stress CI job"]
#[test]
fn stress_capacity_1_no_deadlock() -> TestResult<()> {
    let executor = executor(1, |input| Ok(input.clone()))?;
    let start = Instant::now();

    for i in 0_u64..100 {
        let input = json!({ "i": i });
        let output = execute(&executor, &input)?;
        assert_eq!(output, json!({ "i": i }));
    }

    assert!(
        start.elapsed() < Duration::from_secs(10),
        "capacity-1 stress exceeded timeout"
    );
    Ok(())
}

#[ignore = "runs only in the dedicated ThreadPoolExecutor stress CI job"]
#[test]
fn stress_capacity_256_max() -> TestResult<()> {
    let executor = Arc::new(executor(256, |input| Ok(input.clone()))?);
    let barrier = Arc::new(Barrier::new(257));
    let errors = Arc::new(Mutex::new(Vec::new()));

    thread::scope(|scope| {
        for i in 0_u64..256 {
            let executor = Arc::clone(&executor);
            let barrier = Arc::clone(&barrier);
            let errors = Arc::clone(&errors);
            scope.spawn(move || {
                barrier.wait();
                let expected = json!({ "i": i });
                match execute(&executor, &expected) {
                    Ok(output) if output == expected => {}
                    Ok(output) => lock_errors(&errors).push(format!("wrong output {output:?}")),
                    Err(err) => lock_errors(&errors).push(format!("executor error {err:?}")),
                }
            });
        }
        barrier.wait();
    });

    let errors = lock_errors(&errors);
    assert!(errors.is_empty(), "capacity-256 errors: {errors:?}");
    if let Some(rss) = current_rss_kb() {
        assert!(rss < 512 * 1024, "RSS exceeded 512MiB: {rss}KiB");
    }
    Ok(())
}

#[ignore = "runs only in the dedicated ThreadPoolExecutor stress CI job"]
#[test]
fn wasm_build_not_broken() -> TestResult<()> {
    let target = "wasm32-wasip1";
    let target_libdir = Command::new("rustc")
        .args(["--print", "target-libdir", "--target", target])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|path| path.trim().to_string());

    let Some(target_libdir) = target_libdir else {
        eprintln!("Skipping WASM build stress check: unable to resolve {target} target libdir");
        return Ok(());
    };

    if !std::path::Path::new(&target_libdir)
        .join("libcore.rlib")
        .is_file()
    {
        eprintln!("Skipping WASM build stress check: {target} stdlib is not installed");
        return Ok(());
    }

    let status = Command::new("cargo")
        .args([
            "build",
            "--target",
            target,
            "-p",
            "traverse-expedition-wasm",
        ])
        .status()?;

    assert!(
        status.success(),
        "WASM build failed for traverse-expedition-wasm target {target}"
    );
    Ok(())
}
