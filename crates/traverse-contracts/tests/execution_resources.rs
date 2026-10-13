use traverse_contracts::{ExecutionResources, host_meets_execution_resources};

#[test]
fn host_meets_execution_resources_accepts_adequate_ceiling() {
    let resources = ExecutionResources {
        min_memory_bytes: Some(64 * 1024 * 1024),
        max_fuel: Some(1_000_000),
    };
    assert!(
        host_meets_execution_resources("example.cap", &resources, 128 * 1024 * 1024, 5_000_000)
            .is_ok()
    );
}

#[test]
fn host_meets_execution_resources_rejects_low_memory() {
    let resources = ExecutionResources {
        min_memory_bytes: Some(64 * 1024 * 1024),
        max_fuel: None,
    };
    let result =
        host_meets_execution_resources("example.cap", &resources, 32 * 1024 * 1024, 5_000_000);
    assert!(result.is_err());
    let err = result.err().map_or(String::new(), |message| message);
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
    let result =
        host_meets_execution_resources("example.cap", &resources, 32 * 1024 * 1024, 1_000);
    assert!(result.is_err());
    let err = result.err().map_or(String::new(), |message| message);
    assert!(err.contains("resource_requirements_exceed_host"));
    assert!(err.contains("10000000"));
}
