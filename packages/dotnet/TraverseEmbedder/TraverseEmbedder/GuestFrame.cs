namespace Traverse;

/// <summary>
/// Spec 138 guest-frame helpers (ONNX multi-IO bundles, #1661).
/// Hosts stage the concatenated bytes as opaque model input/output.
/// </summary>
public static class GuestFrame
{
    /// <summary>Guest frame header version (FRAME_VERSION / MODEL_GUEST_ABI_VERSION).</summary>
    public const ushort Version = 1;

    /// <summary>Encode one little-endian Spec 138 guest frame.</summary>
    public static byte[] Encode(byte dtype, IReadOnlyList<uint> dims, ReadOnlySpan<byte> payload)
    {
        var encoded = new byte[4 + dims.Count * 4 + 4 + payload.Length];
        encoded[0] = (byte)(Version & 0xff);
        encoded[1] = (byte)((Version >> 8) & 0xff);
        encoded[2] = dtype;
        encoded[3] = (byte)dims.Count;
        var offset = 4;
        foreach (var dim in dims)
        {
            encoded[offset] = (byte)(dim & 0xff);
            encoded[offset + 1] = (byte)((dim >> 8) & 0xff);
            encoded[offset + 2] = (byte)((dim >> 16) & 0xff);
            encoded[offset + 3] = (byte)((dim >> 24) & 0xff);
            offset += 4;
        }
        var len = (uint)payload.Length;
        encoded[offset] = (byte)(len & 0xff);
        encoded[offset + 1] = (byte)((len >> 8) & 0xff);
        encoded[offset + 2] = (byte)((len >> 16) & 0xff);
        encoded[offset + 3] = (byte)((len >> 24) & 0xff);
        offset += 4;
        payload.CopyTo(encoded.AsSpan(offset));
        return encoded;
    }

    /// <summary>Concatenate frames into one staged input/output bundle.</summary>
    public static byte[] Concat(IReadOnlyList<byte[]> frames)
    {
        var total = 0;
        foreach (var frame in frames)
        {
            total += frame.Length;
        }
        var bundle = new byte[total];
        var offset = 0;
        foreach (var frame in frames)
        {
            Buffer.BlockCopy(frame, 0, bundle, offset, frame.Length);
            offset += frame.Length;
        }
        return bundle;
    }
}
