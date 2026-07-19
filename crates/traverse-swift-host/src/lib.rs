//! Minimal C-ABI boundary for the Apple `wasmi` feasibility proof.
//!
//! The production surface remains deliberately small: Swift configures limits
//! and invokes only governed runtime operations through a later safe wrapper.

#![allow(unsafe_code)] // Temporary, scoped C-ABI exception tracked by #771.

use wasmi::{Config, Engine, Linker, Module, Store, StoreLimitsBuilder, TrapCode};

/// Builds the engine-owned memory ceiling used by the feasibility fixture.
///
/// This function proves that the engine limit API is available to a static
/// Apple host without relying on SPI or a watchdog.
#[must_use]
pub fn configured_memory_limit(bytes: usize) -> usize {
    let _limits = StoreLimitsBuilder::new().memory_size(bytes).build();
    bytes
}

/// C-compatible build marker for Swift/XCFramework integration checks.
#[unsafe(no_mangle)]
pub extern "C" fn traverse_swift_host_abi_version() -> u32 {
    1
}

fn memory_fixture() -> Result<(), wasmi::Error> {
    let engine = Engine::default();
    let mut store = Store::new(
        &engine,
        StoreLimitsBuilder::new()
            .memory_size(65_536)
            .trap_on_grow_failure(true)
            .build(),
    );
    store.limiter(|limits| limits);
    let bytes = wat::parse_str("(module (memory 1) (func (export \"grow\") (result i32) i32.const 1 memory.grow))")
        .map_err(|_| wasmi::Error::new("invalid memory fixture"))?;
    let module = Module::new(store.engine(), bytes)?;
    let instance = Linker::new(store.engine()).instantiate_and_start(&mut store, &module)?;
    let grow = instance.get_typed_func::<(), i32>(&store, "grow")?;
    match grow.call(&mut store, ()) {
        Err(error) if error.as_trap_code() == Some(TrapCode::GrowthOperationLimited) => Ok(()),
        Err(error) => Err(error),
        Ok(_) => Err(wasmi::Error::new("memory fixture unexpectedly grew")),
    }
}

fn fuel_fixture() -> Result<(), wasmi::Error> {
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let mut store = Store::new(&engine, ());
    store.set_fuel(10)?;
    let bytes = wat::parse_str("(module (func (export \"loop\") (loop br 0)))")
        .map_err(|_| wasmi::Error::new("invalid fuel fixture"))?;
    let module = Module::new(&engine, bytes)?;
    let instance = Linker::new(&engine).instantiate_and_start(&mut store, &module)?;
    let run = instance.get_typed_func::<(), ()>(&store, "loop")?;
    match run.call(&mut store, ()) {
        Err(error) if error.as_trap_code() == Some(TrapCode::OutOfFuel) => Ok(()),
        Err(error) => Err(error),
        Ok(()) => Err(wasmi::Error::new("fuel fixture unexpectedly returned")),
    }
}

/// Runs the bounded-memory fixture through the static-library boundary.
#[unsafe(no_mangle)]
pub extern "C" fn traverse_swift_host_memory_limit_fixture() -> u32 {
    u32::from(memory_fixture().is_err())
}

/// Runs the fuel-budget fixture through the static-library boundary.
#[unsafe(no_mangle)]
pub extern "C" fn traverse_swift_host_fuel_limit_fixture() -> u32 {
    u32::from(fuel_fixture().is_err())
}

#[cfg(test)]
mod tests {
    use super::configured_memory_limit;
    use wasmi::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, TrapCode};

    fn memory_store(limit: usize) -> Store<StoreLimits> {
        let engine = Engine::default();
        let mut store = Store::new(
            &engine,
            StoreLimitsBuilder::new()
                .memory_size(limit)
                .trap_on_grow_failure(true)
                .build(),
        );
        store.limiter(|limits| limits);
        store
    }

    #[test]
    fn exposes_the_configured_memory_limit() {
        assert_eq!(configured_memory_limit(65_536), 65_536);
    }

    #[test]
    fn memory_growth_traps_at_the_configured_limit() {
        let mut store = memory_store(65_536);
        let bytes = wat::parse_str("(module (memory 1) (func (export \"grow\") (result i32) i32.const 1 memory.grow))")
            .expect("fixture is valid WAT");
        let module = Module::new(store.engine(), bytes).expect("module compiles");
        let instance = Linker::new(store.engine())
            .instantiate_and_start(&mut store, &module)
            .expect("module instantiates");
        let grow = instance
            .get_typed_func::<(), i32>(&store, "grow")
            .expect("grow export exists");
        let error = grow.call(&mut store, ()).expect_err("growth must trap");
        assert_eq!(error.as_trap_code(), Some(TrapCode::GrowthOperationLimited));
    }

    #[test]
    fn fuel_exhaustion_stops_a_non_terminating_module() {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config);
        let mut store = Store::new(&engine, ());
        store.set_fuel(10).expect("fuel metering is enabled");
        let bytes = wat::parse_str("(module (func (export \"loop\") (loop br 0)))")
            .expect("fixture is valid WAT");
        let module = Module::new(&engine, bytes).expect("module compiles");
        let instance = Linker::new(&engine)
            .instantiate_and_start(&mut store, &module)
            .expect("module instantiates");
        let run = instance
            .get_typed_func::<(), ()>(&store, "loop")
            .expect("loop export exists");
        let error = run.call(&mut store, ()).expect_err("fuel must stop the loop");
        assert_eq!(error.as_trap_code(), Some(TrapCode::OutOfFuel));
    }
}
