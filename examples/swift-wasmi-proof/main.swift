import TraverseSwiftHost

let version = traverse_swift_host_abi_version()
guard version == 1 else {
    fatalError("unexpected Traverse Swift host ABI version: \(version)")
}
print("Traverse Swift host ABI \(version) is available")
