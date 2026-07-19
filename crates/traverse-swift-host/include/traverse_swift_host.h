#ifndef TRAVERSE_SWIFT_HOST_H
#define TRAVERSE_SWIFT_HOST_H

#include <stdint.h>

/// Returns the version of the deliberately narrow Swift-host C boundary.
uint32_t traverse_swift_host_abi_version(void);

#endif
