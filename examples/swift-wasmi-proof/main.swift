import TraverseSwiftHost

let version = traverse_swift_host_abi_version()
guard version == 2 else {
    fatalError("unexpected Traverse Swift host ABI version: \(version)")
}
guard String(cString: traverse_swift_host_status_message(0)) == "ok" else {
    fatalError("production status mapping is unavailable")
}
print("Traverse Swift host production ABI \(version) is available")
