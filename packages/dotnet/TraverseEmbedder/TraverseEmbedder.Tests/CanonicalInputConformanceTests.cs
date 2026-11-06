using System.Buffers.Binary;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Traverse.Embedder;
using Xunit;

namespace TraverseEmbedder.Tests;

// Cross-host canonical guest input (Spec 1722 FR-006, #1795).
//
// Runs fixtures/cross-host/canonical-input-v1 through the real `runtime.wasm` in
// TRAVERSE_NATIVE_ARTIFACT_ROOT, once as raw bytes on the bridge client and once through
// RuntimeTraverseEmbedder. The stdin-echo guest returns the bytes it read, which must match
// the Rust reference on every host.
public sealed class CanonicalInputConformanceTests
{
    private const string CapabilityId = "conformance.stdin-echo";

    private sealed record Vector(
        string Name,
        string Input,
        string? ExpectedStdin,
        string? ExpectedSha256,
        string? ExpectedError);

    // The guest's echoed stdin, or the failure message from submit or the result event.
    private sealed record Outcome(string? Stdin, string? Failure);

    private static string FixtureDir()
    {
        for (var dir = new DirectoryInfo(AppContext.BaseDirectory); dir is not null; dir = dir.Parent)
        {
            var candidate = Path.Join(dir.FullName, "fixtures", "cross-host", "canonical-input-v1");
            if (Directory.Exists(candidate)) return candidate;
        }
        throw new DirectoryNotFoundException("fixtures/cross-host/canonical-input-v1");
    }

    private static IEnumerable<Vector> Vectors()
    {
        using var fixture = JsonDocument.Parse(File.ReadAllText(Path.Join(FixtureDir(), "vectors.json")));
        return fixture.RootElement.GetProperty("vectors").EnumerateArray().Select(entry => new Vector(
            entry.GetProperty("name").GetString()!,
            entry.GetProperty("input").GetString()!,
            Optional(entry, "expected_stdin"),
            Optional(entry, "expected_sha256"),
            Optional(entry, "expected_error"))).ToList();
    }

    private static string? Optional(JsonElement entry, string name) =>
        entry.TryGetProperty(name, out var value) ? value.GetString() : null;

    private static string Sha256(byte[] bytes) => Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant();

    // A fresh real `runtime.wasm` with stdin-echo as a stateless capability, or null without an artifact root.
    private static (WasmtimeRuntimeBridge Bridge, WasmtimeBridgeClient Client)? StdinEchoClient()
    {
        var root = Environment.GetEnvironmentVariable("TRAVERSE_NATIVE_ARTIFACT_ROOT");
        if (string.IsNullOrWhiteSpace(root)) return null;
        var runtime = File.ReadAllBytes(Path.Join(root, "runtime", "runtime.wasm"));
        var bridge = new WasmtimeRuntimeBridge(new TraverseBundle(root, "sha256:" + Sha256(runtime)), fuelPerCall: 50_000_000);
        var client = new WasmtimeBridgeClient(bridge);
        var header = Encoding.UTF8.GetBytes(
            "{\"capability_id\":\"" + CapabilityId + "\",\"capability_version\":\"1.0.0\","
            + "\"service_type\":\"stateless\",\"emits\":[],"
            + "\"host_placement_target\":\"local\",\"permitted_targets\":[\"local\"]}");
        var guest = File.ReadAllBytes(Path.Join(FixtureDir(), "stdin-echo.wasm"));
        var initPayload = new byte[4 + header.Length + guest.Length];
        BinaryPrimitives.WriteUInt32LittleEndian(initPayload, (uint)header.Length);
        header.CopyTo(initPayload, 4);
        guest.CopyTo(initPayload, 4 + header.Length);
        using var ready = JsonDocument.Parse(client.Initialize(initPayload));
        Assert.Equal("ready", ready.RootElement.GetProperty("status").GetString());
        return (bridge, client);
    }

    // `submit` returns the rejection text when the runtime didn't accept.
    private static Outcome Run(WasmtimeBridgeClient client, Func<string?> submit)
    {
        try
        {
            if (submit() is { } rejection) return new Outcome(null, rejection);
            while (client.NextEvent() is { } bytes)
            {
                using var item = JsonDocument.Parse(bytes);
                if (item.RootElement.GetProperty("type").GetString() != "capability_result") continue;
                var data = item.RootElement.GetProperty("data");
                return data.GetProperty("status").GetString() == "completed"
                    ? new Outcome(data.GetProperty("output").GetProperty("stdin").GetString(), null)
                    : new Outcome(null, data.TryGetProperty("error", out var error) ? error.ToString() : data.ToString());
            }
            return new Outcome(null, "no capability_result event");
        }
        catch (TraverseBridgeException error)
        {
            // A rejected submit: the message is the runtime's response, with its code.
            return new Outcome(null, error.Message);
        }
    }

    private static void Check(string host, Vector vector, Outcome outcome)
    {
        if (vector.ExpectedError is { } code)
        {
            Assert.True(outcome.Failure?.Contains(code, StringComparison.Ordinal) == true,
                $"{host}/{vector.Name}: expected {code}, got {outcome}");
            return;
        }
        Assert.True(outcome.Stdin is not null, $"{host}/{vector.Name}: expected output, got {outcome}");
        Assert.Equal(vector.ExpectedStdin, outcome.Stdin);
        Assert.Equal(vector.ExpectedSha256, Sha256(Encoding.UTF8.GetBytes(outcome.Stdin!)));
    }

    [Fact]
    public void CanonicalInputVectorsReachTheGuestAsRawBytes()
    {
        foreach (var vector in Vectors())
        {
            if (StdinEchoClient() is not var (bridge, client)) return;
            using var ownedBridge = bridge;
            Check("dotnet raw", vector, Run(client, () =>
            {
                var response = client.Submit(Encoding.UTF8.GetBytes(vector.Input));
                using var result = JsonDocument.Parse(response);
                return result.RootElement.GetProperty("status").GetString() == "accepted"
                    ? null
                    : Encoding.UTF8.GetString(response);
            }));
        }
    }

    [Fact]
    public void CanonicalInputVectorsReachTheGuestThroughTheEmbedder()
    {
        // TraverseSubmission carries the caller's JSON text unchanged, so bytes-only vectors
        // (duplicate member names) run here too.
        foreach (var vector in Vectors())
        {
            if (StdinEchoClient() is not var (bridge, client)) return;
            using var ownedBridge = bridge;
            var runtime = new RuntimeTraverseEmbedder(client);
            Check("dotnet embedder", vector, Run(client, () =>
            {
                var result = runtime.Submit(new TraverseSubmission(CapabilityId, vector.Input));
                return result.Status == "accepted" ? null : $"{result.Status}: {result.Error}";
            }));
        }
    }

    // A fresh real `runtime.wasm` initialized with a compatible capability on `windows` (Spec 072 FR-010).
    private static (WasmtimeRuntimeBridge Bridge, WasmtimeBridgeClient Client)? CompatibleClient()
    {
        var root = Environment.GetEnvironmentVariable("TRAVERSE_NATIVE_ARTIFACT_ROOT");
        if (string.IsNullOrWhiteSpace(root)) return null;
        var runtime = File.ReadAllBytes(Path.Join(root, "runtime", "runtime.wasm"));
        var bridge = new WasmtimeRuntimeBridge(new TraverseBundle(root, "sha256:" + Sha256(runtime)), fuelPerCall: 50_000_000);
        var client = new WasmtimeBridgeClient(bridge);
        var header = Encoding.UTF8.GetBytes(
            "{\"capability_id\":\"conformance.camera\",\"capability_version\":\"1.0.0\","
            + "\"execution_mode\":\"compatible\",\"platforms\":[\"windows\"],\"host_platform\":\"windows\"}");
        var initPayload = new byte[4 + header.Length];
        BinaryPrimitives.WriteUInt32LittleEndian(initPayload, (uint)header.Length);
        header.CopyTo(initPayload, 4);
        using var ready = JsonDocument.Parse(client.Initialize(initPayload));
        Assert.Equal("ready", ready.RootElement.GetProperty("status").GetString());
        return (bridge, client);
    }

    [Fact]
    public void CanonicalInputVectorsReachTheWrapperThroughCompatibleStart()
    {
        // Spec 072 FR-012 (Decision 128): the started event's input is the runtime's canonical bytes.
        const string marker = ",\"input\":";
        foreach (var vector in Vectors())
        {
            if (CompatibleClient() is not var (bridge, client)) return;
            using var ownedBridge = bridge;
            var runtime = new RuntimeTraverseEmbedder(client);
            Outcome outcome;
            try
            {
                runtime.CompatibleStart("conformance.camera", vector.Input);
                var output = runtime.Subscribe().First(item => item.EventType == "state_changed").Output!;
                var start = output.IndexOf(marker, StringComparison.Ordinal) + marker.Length;
                outcome = new Outcome(output[start..^1], null);
            }
            catch (TraverseBridgeException error)
            {
                outcome = new Outcome(null, error.Message);
            }
            Check("dotnet compatible", vector, outcome);
        }
    }

    [Fact]
    public void CompatibleLifecycleUsesTheRuntimeRules()
    {
        if (CompatibleClient() is not var (bridge, client)) return;
        using var ownedBridge = bridge;
        var runtime = new RuntimeTraverseEmbedder(client);
        var started = runtime.CompatibleStart("conformance.camera", "{}");
        Assert.Equal("conformance.camera-compatible-1", started.InstanceId);
        Assert.Throws<TraverseBridgeException>(() => runtime.CompatibleStart("conformance.camera", "{}"));
        Assert.Equal("killed", runtime.CompatibleKill("conformance.camera", null).Status);
        Assert.Throws<TraverseBridgeException>(() => runtime.CompatibleKill("conformance.camera", started.InstanceId));
        Assert.Equal(["started", "killed"], runtime.Subscribe().Select(item => item.Status));
    }
}
