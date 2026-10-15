import Foundation

/// Spec 138 guest-frame helpers (ONNX multi-IO bundles, #1661).
public enum GuestFrame {
    /// Guest frame header version (matches `MODEL_GUEST_ABI_VERSION` / FRAME_VERSION).
    public static let version: UInt16 = 1

    /// Encode one little-endian Spec 138 guest frame.
    public static func encode(dtype: UInt8, dims: [UInt32], payload: Data) -> Data {
        var out = Data()
        out.reserveCapacity(4 + dims.count * 4 + 4 + payload.count)
        var ver = version.littleEndian
        withUnsafeBytes(of: &ver) { out.append(contentsOf: $0) }
        out.append(dtype)
        out.append(UInt8(truncatingIfNeeded: dims.count))
        for dim in dims {
            var le = dim.littleEndian
            withUnsafeBytes(of: &le) { out.append(contentsOf: $0) }
        }
        var len = UInt32(payload.count).littleEndian
        withUnsafeBytes(of: &len) { out.append(contentsOf: $0) }
        out.append(payload)
        return out
    }

    /// Concatenate frames into one staged input/output bundle (order = package tensor config).
    public static func concat(_ frames: [Data]) -> Data {
        var total = 0
        for frame in frames { total += frame.count }
        var out = Data()
        out.reserveCapacity(total)
        for frame in frames { out.append(frame) }
        return out
    }
}
