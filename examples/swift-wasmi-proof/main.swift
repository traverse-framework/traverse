import TraverseSwiftHost

let version = traverse_swift_host_abi_version()
guard version == 1 else {
    fatalError("unexpected Traverse Swift host ABI version: \(version)")
}
guard traverse_swift_host_memory_limit_fixture() == 0 else {
    fatalError("memory fixture did not stop at the configured limit")
}
guard traverse_swift_host_fuel_limit_fixture() == 0 else {
    fatalError("fuel fixture did not stop execution")
}
print("Traverse Swift host ABI \(version) is available")
