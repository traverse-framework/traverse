#ifndef TRAVERSE_SWIFT_HOST_H
#define TRAVERSE_SWIFT_HOST_H

#include <stdint.h>
#include <stddef.h>

/// Returns the version of the production Swift-host C boundary.
uint32_t traverse_swift_host_abi_version(void);

typedef struct traverse_swift_host_limits {
  uint64_t maximum_artifact_bytes;
  uint64_t maximum_memory_bytes;
  uint64_t fuel_per_invocation;
  uint64_t maximum_input_bytes;
  uint64_t maximum_output_bytes;
  uint64_t maximum_queued_events;
} traverse_swift_host_limits;

/// Verifies and instantiates a bridge module. `handle_out` receives an opaque
/// value that must be passed only to this ABI and later destroyed.
int32_t traverse_swift_host_create(
  const uint8_t *runtime_bytes, size_t runtime_length,
  const uint8_t *expected_sha256, size_t expected_sha256_length,
  const traverse_swift_host_limits *limits, uint64_t *handle_out);

/// Invokes one bridge operation with caller-owned UTF-8 JSON buffers.
/// On `TRAVERSE_SWIFT_HOST_BUFFER_TOO_SMALL`, `output_length_out` is the exact
/// required length and the caller may retry once with that capacity.
int32_t traverse_swift_host_invoke(
  uint64_t handle, const uint8_t *operation, size_t operation_length,
  const uint8_t *input, size_t input_length,
  uint8_t *output, size_t output_capacity, size_t *output_length_out);

/// Invalidates and releases an opaque host handle. Calling it twice is safe.
int32_t traverse_swift_host_destroy(uint64_t handle);

/// Returns a stable, static UTF-8 description for a status code.
const char *traverse_swift_host_status_message(int32_t status);

#endif
