using System.Buffers.Binary;
using System.Security.Cryptography;
using System.Text.Json;
using Traverse.Embedder;
using Xunit;

namespace TraverseEmbedder.Tests;

public sealed class WasmtimeRuntimeBridgeTests
{
    // A WASI-command capability that echoes stdin to stdout, then calls
    // `traverse_host::emit_event` with a fixed declared domain event — the
    // compiled form of the same WAT source as
    // `crates/traverse-runtime/tests/native_bridge_conformance.rs`'s
    // `NESTED_CAPABILITY_WAT`, so every host profile's conformance run
    // exercises the same nested-capability behavior. No WAT-to-wasm compiler
    // is available in this test project (unlike Swift/Kotlin, which compile
    // WAT at test time), so this is precompiled the same way the other
    // fixtures below are.
    private const string NestedConformanceCapabilityFixture = "AGFzbQEAAAABEgNgBH9/f38Bf2ACf38Bf2AAAAJfAxZ3YXNpX3NuYXBzaG90X3ByZXZpZXcxB2ZkX3JlYWQAABZ3YXNpX3NuYXBzaG90X3ByZXZpZXcxCGZkX3dyaXRlAAANdHJhdmVyc2VfaG9zdAplbWl0X2V2ZW50AAEDAgECBQMBAAEHEwIGbWVtb3J5AgAGX3N0YXJ0AAMKRgFEAEEAQQg2AgBBBEGACDYCAEEAQQBBAUGEIBAAGkEAQQg2AgBBBEGEICgCADYCAEEBQQBBAUGIIBABGkGIJ0HJABACGgsLUAEAQYgnC0l7ImV2ZW50X2lkIjoiY29uZm9ybWFuY2UuZWNob2VkIiwidmVyc2lvbiI6IjEuMC4wIiwicGF5bG9hZCI6eyJvayI6dHJ1ZX19";
    private const string BridgeFixture = "AGFzbQEAAAABFgRgAAF/YAF/AX9gAn9/AGADf39/AX8DDAsAAQIDAwEDAwMDAQUEAQEBEAf8AQwGbWVtb3J5AgAbdHJhdmVyc2VfYnJpZGdlX2FiaV92ZXJzaW9uAAAOdHJhdmVyc2VfYWxsb2MAARB0cmF2ZXJzZV9kZWFsbG9jAAINdHJhdmVyc2VfaW5pdAADD3RyYXZlcnNlX3N1Ym1pdAAEE3RyYXZlcnNlX25leHRfZXZlbnQABQ90cmF2ZXJzZV9jYW5jZWwABhl0cmF2ZXJzZV9jb21wYXRpYmxlX3N0YXJ0AAcYdHJhdmVyc2VfY29tcGF0aWJsZV9zdG9wAAgYdHJhdmVyc2VfY29tcGF0aWJsZV9raWxsAAkRdHJhdmVyc2Vfc2h1dGRvd24ACgo5CwYAQfTOAAsFAEHAAAsCAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQAL";
    private const string ImportedFixture = "AGFzbQEAAAABCAJgAABgAAF/AiMBFndhc2lfc25hcHNob3RfcHJldmlldzEIZmRfd3JpdGUAAAMCAQEFAwEAAQcoAgZtZW1vcnkCABt0cmF2ZXJzZV9icmlkZ2VfYWJpX3ZlcnNpb24AAQoIAQYAQfTOAAs=";
    private const string BridgeTenFixture = "AGFzbQEAAAABFgRgAAF/YAF/AX9gAn9/AGADf39/AX8DDAsAAQIDAwEDAwMDAQUEAQEBEAf8AQwGbWVtb3J5AgAbdHJhdmVyc2VfYnJpZGdlX2FiaV92ZXJzaW9uAAAOdHJhdmVyc2VfYWxsb2MAARB0cmF2ZXJzZV9kZWFsbG9jAAINdHJhdmVyc2VfaW5pdAADD3RyYXZlcnNlX3N1Ym1pdAAEE3RyYXZlcnNlX25leHRfZXZlbnQABQ90cmF2ZXJzZV9jYW5jZWwABhl0cmF2ZXJzZV9jb21wYXRpYmxlX3N0YXJ0AAcYdHJhdmVyc2VfY29tcGF0aWJsZV9zdG9wAAgYdHJhdmVyc2VfY29tcGF0aWJsZV9raWxsAAkRdHJhdmVyc2Vfc2h1dGRvd24ACgo5CwYAQZDOAAsFAEHAAAsCAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQALBABBAAsEAEEACwQAQQAL";
    private const string ClientFixture = "AGFzbQEAAAABFgRgAAF/YAF/AX9gAn9/AGADf39/AX8DDQwAAQIDAwMBAwMDAwEFBAEBARAGBgF/AUEACwf8AQwGbWVtb3J5AgAbdHJhdmVyc2VfYnJpZGdlX2FiaV92ZXJzaW9uAAAOdHJhdmVyc2VfYWxsb2MAARB0cmF2ZXJzZV9kZWFsbG9jAAINdHJhdmVyc2VfaW5pdAAED3RyYXZlcnNlX3N1Ym1pdAAFE3RyYXZlcnNlX25leHRfZXZlbnQABg90cmF2ZXJzZV9jYW5jZWwABxl0cmF2ZXJzZV9jb21wYXRpYmxlX3N0YXJ0AAgYdHJhdmVyc2VfY29tcGF0aWJsZV9zdG9wAAkYdHJhdmVyc2VfY29tcGF0aWJsZV9raWxsAAoRdHJhdmVyc2Vfc2h1dGRvd24ACwqXAQwGAEH0zgALBQBBwAALAgALFQAgACABNgIAIABBBGogAjYCAEEACwsAIAJBgARBEhADCwsAIAJBoARBFRADCxsAIwBFBH9BASQAIABBwARBDhADGkEBBUEACwsLACACQaAEQRUQAwsLACACQaAEQRUQAwsLACACQaAEQRUQAwsLACACQaAEQRUQAwsLACAAQeAEQRQQAwsLYgQAQYAECxJ7InN0YXR1cyI6InJlYWR5In0AQaAECxV7InN0YXR1cyI6ImFjY2VwdGVkIn0AQcAECw57InNlcXVlbmNlIjoxfQBB4AQLFHsic3RhdHVzIjoic3RvcHBlZCJ9";
    private const string TypedClientFixture = "AGFzbQEAAAABFgRgAAF/YAF/AX9gAn9/AGADf39/AX8DDQwAAQIDAwMBAwMDAwEFBAEBARAGBgF/AUEACwf8AQwGbWVtb3J5AgAbdHJhdmVyc2VfYnJpZGdlX2FiaV92ZXJzaW9uAAAOdHJhdmVyc2VfYWxsb2MAARB0cmF2ZXJzZV9kZWFsbG9jAAINdHJhdmVyc2VfaW5pdAAED3RyYXZlcnNlX3N1Ym1pdAAFE3RyYXZlcnNlX25leHRfZXZlbnQABg90cmF2ZXJzZV9jYW5jZWwABxl0cmF2ZXJzZV9jb21wYXRpYmxlX3N0YXJ0AAgYdHJhdmVyc2VfY29tcGF0aWJsZV9zdG9wAAkYdHJhdmVyc2VfY29tcGF0aWJsZV9raWxsAAoRdHJhdmVyc2Vfc2h1dGRvd24ACwqXAQwGAEH0zgALBQBBwAALAgALFQAgACABNgIAIABBBGogAjYCAEEACwsAIAJBgARBEhADCwsAIAJBoARBJxADCxsAIwBFBH9BASQAIABB4ARBNhADGkEBBUEACwsLACACQaAEQScQAwsLACACQaAEQScQAwsLACACQaAEQScQAwsLACACQaAEQScQAwsLACAAQcAFQRQQAwsLnAEEAEGABAsSeyJzdGF0dXMiOiJyZWFkeSJ9AEGgBAsneyJzZXNzaW9uX2lkIjoiczEiLCJzdGF0dXMiOiJhY2NlcHRlZCJ9AEHgBAs2eyJzZXF1ZW5jZSI6MSwidGFyZ2V0X2lkIjoiZGVtbyIsInN0YXR1cyI6ImNvbXBsZXRlZCJ9AEHABQsUeyJzdGF0dXMiOiJzdG9wcGVkIn0=";

    [Fact]
    public void RealNativeArtifactRunsWithoutASidecar()
    {
        var root = Environment.GetEnvironmentVariable("TRAVERSE_NATIVE_ARTIFACT_ROOT");
        if (string.IsNullOrWhiteSpace(root)) return;
        var bytes = File.ReadAllBytes(Path.Join(root, "runtime", "runtime.wasm"));
        // The default fuel budget is sized for a trivial fixture guest. The
        // real `runtime.wasm` interprets genuine Rust code (JSON parsing,
        // heap allocation, a nested wasmi engine) on `init`/`submit`, which
        // costs far more simulated fuel than a few store instructions.
        using var bridge = new WasmtimeRuntimeBridge(new TraverseBundle(root, Digest(bytes)), fuelPerCall: 50_000_000);
        var client = new WasmtimeBridgeClient(bridge);

        // The real `runtime-wasm-bridge/1.0.0` guest (crates/traverse-runtime-wasm)
        // hosts a *nested* capability itself, so `Initialize`'s payload is not
        // bare JSON: a 4-byte little-endian header length, that many bytes of
        // JSON metadata, then the raw nested-capability WASM artifact (spec
        // 1402 FR-003/FR-011). `NestedConformanceCapabilityFixture` echoes
        // stdin to stdout, then emits one declared domain event — matching
        // `crates/traverse-runtime/tests/native_bridge_conformance.rs`'s
        // fixture exactly, so all host profiles exercise the same lifecycle
        // transcript.
        var nestedCapability = Convert.FromBase64String(NestedConformanceCapabilityFixture);
        var header = "{\"capability_id\":\"dotnet.conformance.echo\",\"capability_version\":\"1.0.0\","
            + "\"service_type\":\"subscribable\","
            + "\"emits\":[{\"event_id\":\"conformance.echoed\",\"version\":\"1.0.0\"}],"
            + "\"host_placement_target\":\"local\",\"permitted_targets\":[\"local\"]}";
        var headerBytes = System.Text.Encoding.UTF8.GetBytes(header);
        var initPayload = new byte[4 + headerBytes.Length + nestedCapability.Length];
        BinaryPrimitives.WriteUInt32LittleEndian(initPayload, (uint)headerBytes.Length);
        headerBytes.CopyTo(initPayload, 4);
        nestedCapability.CopyTo(initPayload, 4 + headerBytes.Length);

        using var initResponse = JsonDocument.Parse(client.Initialize(initPayload));
        Assert.Equal("ready", initResponse.RootElement.GetProperty("status").GetString());

        using var submitResponse = JsonDocument.Parse(client.Submit("{\"hello\":\"dotnet-conformance\"}"u8));
        Assert.Equal("accepted", submitResponse.RootElement.GetProperty("status").GetString());

        var eventTypes = new List<string>();
        while (client.NextEvent() is { } eventBytes)
        {
            using var eventJson = JsonDocument.Parse(eventBytes);
            eventTypes.Add(eventJson.RootElement.GetProperty("type").GetString()!);
        }
        Assert.Equal(["capability_invoked", "conformance.echoed", "capability_result"], eventTypes);

        Assert.Equal("{\"status\":\"stopped\"}", Text(client.Shutdown()));
    }

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

    [Fact]
    public void RuntimeEmbedderMapsRuntimeOwnedResultsIntoPublicTypes()
    {
        using var bridge = new WasmtimeRuntimeBridge(FixtureBundle(fixture: TypedClientFixture).Bundle);
        var runtime = new RuntimeTraverseEmbedder(new WasmtimeBridgeClient(bridge));
        runtime.Initialize("{}");

        Assert.Equal(
            new TraverseSubmissionResult("s1", "accepted"),
            runtime.Submit(new TraverseSubmission("demo", "{}")));
        Assert.Equal(
            [new TraverseRuntimeEvent(1, "demo", "completed")],
            runtime.Subscribe());
        Assert.Equal("{\"status\":\"stopped\"}", runtime.Shutdown());
    }

    [Fact]
    public void TestDoubleExposesScriptedTargetOutputPublicly()
    {
        var harness = new InMemoryTraverseEmbedder().WithTargetOutput("{\"answer\":42}");
        harness.Initialize(new TraverseBundle("assets/traverse", "sha256:test"));
        harness.Submit(new TraverseSubmission("demo.target", "{}"));
        Assert.Equal(new TraverseRuntimeEvent(1, "demo.target", "accepted", EventType: "capability_result", SessionId: "dotnet-session-1", Output: "{\"answer\":42}"), harness.Subscribe().Single());
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
