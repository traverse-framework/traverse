using System.Security.Cryptography;
using Traverse.Embedder;
using Xunit;

namespace TraverseEmbedder.Tests;

public sealed class WasmtimeRuntimeBridgeTests
{
    private const string BridgeFixture = "AGFzbQEAAAABFgRgAAF/YAF/AX9gAn9/AGADf39/AX8DDAsAAQIDAwEDAwMDAQUEAQEBEAf8AQwGbWVtb3J5AgAbdHJhdmVyc2VfYnJpZGdlX2FiaV92ZXJzaW9uAAAOdHJhdmVyc2VfYWxsb2MAARB0cmF2ZXJzZV9kZWFsbG9jAAINdHJhdmVyc2VfaW5pdAADD3RyYXZlcnNlX3N1Ym1pdAAEE3RyYXZlcnNlX25leHRfZXZlbnQABQ90cmF2ZXJzZV9jYW5jZWwABhl0cmF2ZXJzZV9jb21wYXRpYmxlX3N0YXJ0AAcYdHJhdmVyc2VfY29tcGF0aWJsZV9zdG9wAAgYdHJhdmVyc2VfY29tcGF0aWJsZV9raWxsAAkRdHJhdmVyc2Vfc2h1dGRvd24ACgo5CwYAQfTOAAsFAEHAAAsCAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQAL";
    private const string ImportedFixture = "AGFzbQEAAAABCAJgAABgAAF/AiMBFndhc2lfc25hcHNob3RfcHJldmlldzEIZmRfd3JpdGUAAAMCAQEFAwEAAQcoAgZtZW1vcnkCABt0cmF2ZXJzZV9icmlkZ2VfYWJpX3ZlcnNpb24AAQoIAQYAQfTOAAs=";
    private const string BridgeTenFixture = "AGFzbQEAAAABFgRgAAF/YAF/AX9gAn9/AGADf39/AX8DDAsAAQIDAwEDAwMDAQUEAQEBEAf8AQwGbWVtb3J5AgAbdHJhdmVyc2VfYnJpZGdlX2FiaV92ZXJzaW9uAAAOdHJhdmVyc2VfYWxsb2MAARB0cmF2ZXJzZV9kZWFsbG9jAAINdHJhdmVyc2VfaW5pdAADD3RyYXZlcnNlX3N1Ym1pdAAEE3RyYXZlcnNlX25leHRfZXZlbnQABQ90cmF2ZXJzZV9jYW5jZWwABhl0cmF2ZXJzZV9jb21wYXRpYmxlX3N0YXJ0AAcYdHJhdmVyc2VfY29tcGF0aWJsZV9zdG9wAAgYdHJhdmVyc2VfY29tcGF0aWJsZV9raWxsAAkRdHJhdmVyc2Vfc2h1dGRvd24ACgo5CwYAQZDOAAsFAEHAAAsCAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQAL";
    private const string ClientFixture = "AGFzbQEAAAABFgRgAAF/YAF/AX9gAn9/AGADf39/AX8DDQwAAQIDAwMBAwMDAwEFBAEBARAGBgF/AUEACwf8AQwGbWVtb3J5AgAbdHJhdmVyc2VfYnJpZGdlX2FiaV92ZXJzaW9uAAAOdHJhdmVyc2VfYWxsb2MAARB0cmF2ZXJzZV9kZWFsbG9jAAINdHJhdmVyc2VfaW5pdAAED3RyYXZlcnNlX3N1Ym1pdAAFE3RyYXZlcnNlX25leHRfZXZlbnQABg90cmF2ZXJzZV9jYW5jZWwABxl0cmF2ZXJzZV9jb21wYXRpYmxlX3N0YXJ0AAgYdHJhdmVyc2VfY29tcGF0aWJsZV9zdG9wAAkYdHJhdmVyc2VfY29tcGF0aWJsZV9raWxsAAoRdHJhdmVyc2Vfc2h1dGRvd24ACwqXAQwGAEH0zgALBQBBwAALAgALFQAgACABNgIAIABBBGogAjYCAEEACwsAIAJBgARBEhADCwsAIAJBoARBFRADCxsAIwBFBH9BASQAIABBwARBDhADGkEBBUEACwsLACACQaAEQRUQAwsLACACQaAEQRUQAwsLACACQaAEQRUQAwsLACACQaAEQRUQAwsLACAAQeAEQRQQAwsLYgQAQYAECxJ7InN0YXR1cyI6InJlYWR5In0AQaAECxV7InN0YXR1cyI6ImFjY2VwdGVkIn0AQcAECw57InNlcXVlbmNlIjoxfQBB4AQLFHsic3RhdHVzIjoic3RvcHBlZCJ9";

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

    [Fact]
    public void EnforcesMemoryAndFuelLimitsBeforeAcceptingTheBridge()
    {
        var bundle = FixtureBundle().Bundle;
        var memoryError = Assert.Throws<TraverseBundleException>(
            () => new WasmtimeRuntimeBridge(bundle, maximumMemoryBytes: 32 * 1024));
        Assert.Equal("bridge_resource_limit", memoryError.Message);

        var fuelError = Assert.Throws<TraverseBundleException>(
            () => new WasmtimeRuntimeBridge(bundle, fuelPerCall: 1));
        Assert.Equal("bridge_resource_limit", fuelError.Message);
    }

    [Fact]
    public void ClientCopiesResultsAndDrainsEventsInOrder()
    {
        using var bridge = new WasmtimeRuntimeBridge(FixtureBundle(fixture: ClientFixture).Bundle);
        var client = new WasmtimeBridgeClient(bridge);

        Assert.Equal("{\"status\":\"ready\"}", Text(client.Initialize("{}"u8)));
        Assert.Equal("{\"status\":\"accepted\"}", Text(client.Submit("{\"target_id\":\"demo\"}"u8)));
        Assert.Equal("{\"sequence\":1}", Text(client.NextEvent()!));
        Assert.Null(client.NextEvent());
        Assert.Equal("{\"status\":\"stopped\"}", Text(client.Shutdown()));
    }

    private static (TraverseBundle Bundle, byte[] Bytes) FixtureBundle(
        string? declaredDigest = null,
        string fixture = BridgeFixture)
    {
        var bytes = Convert.FromBase64String(fixture);
        var root = Path.Join(Path.GetTempPath(), $"traverse-dotnet-bridge-{Guid.NewGuid():N}");
        var runtime = Path.Join(root, "runtime");
        Directory.CreateDirectory(runtime);
        File.WriteAllBytes(Path.Join(runtime, "runtime.wasm"), bytes);
        return (new TraverseBundle(root, declaredDigest ?? Digest(bytes)), bytes);
    }

    private static string Digest(byte[] bytes) =>
        "sha256:" + Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant();

    private static string Text(byte[] bytes) => System.Text.Encoding.UTF8.GetString(bytes);
}
