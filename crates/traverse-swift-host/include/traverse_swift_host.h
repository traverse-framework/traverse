#ifndef TRAVERSE_SWIFT_HOST_H
#define TRAVERSE_SWIFT_HOST_H

#include <stdint.h>

/// Returns the version of the deliberately narrow Swift-host C boundary.
uint32_t traverse_swift_host_abi_version(void);

/// Runs the bounded-memory fixture. Zero means the host stopped growth.
uint32_t traverse_swift_host_memory_limit_fixture(void);

/// Runs the fuel-budget fixture. Zero means the host stopped execution.
uint32_t traverse_swift_host_fuel_limit_fixture(void);

#endif
