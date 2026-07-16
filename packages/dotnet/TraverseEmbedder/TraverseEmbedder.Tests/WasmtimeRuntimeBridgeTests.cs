using System.Security.Cryptography;
using Traverse.Embedder;
using Xunit;

namespace TraverseEmbedder.Tests;

public sealed class WasmtimeRuntimeBridgeTests
{
    private const string BridgeFixture = "AGFzbQEAAAABFgRgAAF/YAF/AX9gAn9/AGADf39/AX8DDAsAAQIDAwEDAwMDAQUEAQEBEAf8AQwGbWVtb3J5AgAbdHJhdmVyc2VfYnJpZGdlX2FiaV92ZXJzaW9uAAAOdHJhdmVyc2VfYWxsb2MAARB0cmF2ZXJzZV9kZWFsbG9jAAINdHJhdmVyc2VfaW5pdAADD3RyYXZlcnNlX3N1Ym1pdAAEE3RyYXZlcnNlX25leHRfZXZlbnQABQ90cmF2ZXJzZV9jYW5jZWwABhl0cmF2ZXJzZV9jb21wYXRpYmxlX3N0YXJ0AAcYdHJhdmVyc2VfY29tcGF0aWJsZV9zdG9wAAgYdHJhdmVyc2VfY29tcGF0aWJsZV9raWxsAAkRdHJhdmVyc2Vfc2h1dGRvd24ACgo5CwYAQfTOAAsFAEHAAAsCAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQAL";
    private const string ImportedFixture = "AGFzbQEAAAABCAJgAABgAAF/AiMBFndhc2lfc25hcHNob3RfcHJldmlldzEIZmRfd3JpdGUAAAMCAQEFAwEAAQcoAgZtZW1vcnkCABt0cmF2ZXJzZV9icmlkZ2VfYWJpX3ZlcnNpb24AAQoIAQYAQfTOAAs=";
    private const string BridgeTenFixture = "AGFzbQEAAAABFgRgAAF/YAF/AX9gAn9/AGADf39/AX8DDAsAAQIDAwEDAwMDAQUEAQEBEAf8AQwGbWVtb3J5AgAbdHJhdmVyc2VfYnJpZGdlX2FiaV92ZXJzaW9uAAAOdHJhdmVyc2VfYWxsb2MAARB0cmF2ZXJzZV9kZWFsbG9jAAINdHJhdmVyc2VfaW5pdAADD3RyYXZlcnNlX3N1Ym1pdAAEE3RyYXZlcnNlX25leHRfZXZlbnQABQ90cmF2ZXJzZV9jYW5jZWwABhl0cmF2ZXJzZV9jb21wYXRpYmxlX3N0YXJ0AAcYdHJhdmVyc2VfY29tcGF0aWJsZV9zdG9wAAgYdHJhdmVyc2VfY29tcGF0aWJsZV9raWxsAAkRdHJhdmVyc2Vfc2h1dGRvd24ACgo5CwYAQZDOAAsFAEHAAAsCAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQAL";

    [Fact]
    public void VerifiesAndInstantiatesTheGovernedBridge()
    {
        var (bundle, bytes) = FixtureBundle();
        using var bridge = new WasmtimeRuntimeBridge(bundle);

        Assert.Equal(Digest(bytes), bridge.RuntimeWasmDigest);
        Assert.Equal("runtime.wasm", Path.GetFileName(bridge.RuntimePath));
    }

    [Fact]
    public void RejectsTamperingBeforeInstantiation()
    {
        var (bundle, _) = FixtureBundle("sha256:" + new string('0', 64));

        var error = Assert.Throws<TraverseBundleException>(() => new WasmtimeRuntimeBridge(bundle));
        Assert.Equal("bundle_digest_mismatch", error.Message);
    }

    [Fact]
    public void RejectsAmbientImportsAndBridgeTen()
    {
        var importError = Assert.Throws<TraverseBundleException>(
            () => new WasmtimeRuntimeBridge(FixtureBundle(fixture: ImportedFixture).Bundle));
        Assert.Equal("runtime/runtime.wasm requires undeclared ambient imports", importError.Message);

        var versionError = Assert.Throws<TraverseBundleException>(
            () => new WasmtimeRuntimeBridge(FixtureBundle(fixture: BridgeTenFixture).Bundle));
        Assert.Equal("bridge_version_mismatch", versionError.Message);
    }

    private static (TraverseBundle Bundle, byte[] Bytes) FixtureBundle(
        string? declaredDigest = null,
        string fixture = BridgeFixture)
    {
        var bytes = Convert.FromBase64String(fixture);
        var root = Path.Combine(Path.GetTempPath(), $"traverse-dotnet-bridge-{Guid.NewGuid():N}");
        var runtime = Path.Combine(root, "runtime");
        Directory.CreateDirectory(runtime);
        File.WriteAllBytes(Path.Combine(runtime, "runtime.wasm"), bytes);
        return (new TraverseBundle(root, declaredDigest ?? Digest(bytes)), bytes);
    }

    private static string Digest(byte[] bytes) =>
        "sha256:" + Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant();
}
