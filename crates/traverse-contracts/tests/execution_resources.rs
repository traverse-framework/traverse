use traverse_contracts::{ExecutionResources, host_meets_execution_resources};

#[test]
fn host_meets_execution_resources_accepts_adequate_ceiling() {
    let resources = ExecutionResources {
        min_memory_bytes: Some(64 * 1024 * 1024),
        max_fuel: Some(1_000_000),
    };
    host_meets_execution_resources("example.cap", &resources, 128 * 1024 * 1024, 5_000_000)
        .expect("host covers declared needs");
}

#[test]
fn host_meets_execution_resources_rejects_low_memory() {
    let resources = ExecutionResources {
        min_memory_bytes: Some(64 * 1024 * 1024),
        max_fuel: None,
    };
    let err = host_meets_execution_resources("example.cap", &resources, 32 * 1024 * 1024, 5_000_000)
        .expect_err("under-ceiling memory");
    assert!(err.contains("resource_requirements_exceed_host"));
    assert!(err.contains("example.cap"));
    assert!(err.contains("67108864"));
}

#[test]
fn host_meets_execution_resources_rejects_low_fuel() {
    let resources = ExecutionResources {
        min_memory_bytes: None,
        max_fuel: Some(10_000_000),
    };
    let err = host_meets_execution_resources("example.cap", &resources, 32 * 1024 * 1024, 1_000)
        .expect_err("under-ceiling fuel");
    assert!(err.contains("resource_requirements_exceed_host"));
    assert!(err.contains("10000000"));
}
