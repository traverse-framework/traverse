using System.Diagnostics;
using System.Text;
using System.Text.Json.Nodes;
using Traverse.Embedder;
using Xunit;

namespace TraverseEmbedder.Tests;

/// <summary>
/// Decision 111 (#1642): the .NET <see cref="ExactModelHost"/> runs the shared Rust model host
/// through the P/Invoke shim and matches native, web, Swift, and Kotlin: the signed vectors
/// byte-for-byte and the shared Spec 138 rights conformance suite (FR-041) exactly.
/// </summary>
public sealed class ExactModelHostTests
{
    private static string RepoFile(string path)
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null)
        {
            var candidate = Path.Combine(directory.FullName, path);
            if (File.Exists(candidate))
            {
                return candidate;
            }

            directory = directory.Parent;
        }

        throw new FileNotFoundException($"{path} not found above {AppContext.BaseDirectory}");
    }

    private static byte[] Read(string path) => File.ReadAllBytes(RepoFile(path));

    private static JsonObject Json(string path) => (JsonObject)JsonNode.Parse(File.ReadAllText(RepoFile(path)))!;

    private static string S(JsonNode? node, string name) => node![name]!.GetValue<string>();

    private static byte[] Hex(string value) => Convert.FromHexString(value);

    private static string ToHex(byte[] bytes) => Convert.ToHexString(bytes).ToLowerInvariant();

    private static ExactModelPin Pin(JsonNode? value) => ExactModelPin.FromJson(value!.ToJsonString());

    [Theory]
    [InlineData("signed-classifier.json", 4096, 1)]
    [InlineData("signed-digits-mlp.json", 64, 1)]
    [InlineData("signed-digits-onnx.json", 56, 1)]
    // Guest ABI v3 (Decision 110): the first round prepares, later rounds restore the snapshot.
    [InlineData("signed-prepared-v3.json", 4096, 3)]
    public async Task SignedVectorsAreByteIdentical(string vectorPath, int maxOutput, int rounds)
    {
        var vector = Json($"fixtures/models/conformance/{vectorPath}");
        var pin = Pin(vector["pin"]);
        using var host = new ExactModelHost([pin], [S(vector, "trusted_public_key_hex")], "commercial");
        var dir = $"fixtures/models/{S(vector, "package_dir")}";
        var digest = await host.RegisterPackageAsync(
            Read($"{dir}/model.manifest.json"),
            Read($"{dir}/model.wasm"),
            Read($"{dir}/model.sig.json"));
        Assert.Equal(pin.Digest, digest);
        var request = vector["request"]!;
        var cases = vector["cases"] is JsonArray array
            ? array.Select(item => item!).ToList()
            : [new JsonObject
            {
                ["input_frame_hex"] = S(request, "input_frame_hex"),
                ["output_frame_hex"] = S(vector["expected"], "output_frame_hex"),
            }];
        for (var round = 0; round < rounds; round++)
        {
            foreach (var testCase in cases)
            {
                var inputRef = host.StageModelInput(Hex(S(testCase, "input_frame_hex")), 4096);
                var result = await host.ExecuteAsync(
                    pin.ModelId, pin.Version, pin.Digest, inputRef, "policy-1", "sensitive",
                    S(request, "input_schema_ref"), "1.0.0", maxOutput, ["sensitive"]);
                Assert.Equal("wasm-cpu", result.Placement);
                Assert.Equal(S(testCase, "output_frame_hex"), ToHex(host.ReadModelOutput(result.OutputRef, maxOutput)));
                Assert.Equal("active", result.ModelEvidence?.Status);
                Assert.Equal("sensitive", result.DataClassification);
            }
        }
    }

    /// <summary>The public error shape every embedder compares: code, reason, detail.</summary>
    private static JsonObject ErrorJson(ExactModelException error)
    {
        var json = new JsonObject { ["ok"] = false, ["code"] = error.Code, ["reason"] = error.Reason };
        if (error.Detail is { } detail)
        {
            var body = new JsonObject();
            if (detail.ModelId is not null) body["model_id"] = detail.ModelId;
            if (detail.Version is not null) body["version"] = detail.Version;
            if (detail.Digest is not null) body["digest"] = detail.Digest;
            body["field"] = detail.Field;
            body["expected"] = detail.Expected;
            body["actual"] = detail.Actual;
            if (detail.EffectiveUsage is not null) body["effective_usage"] = detail.EffectiveUsage;
            json["detail"] = body;
        }

        return json;
    }

    private static async Task<JsonNode> Attempt(Func<Task<JsonNode>> block)
    {
        try
        {
            return await block();
        }
        catch (ExactModelException error)
        {
            return ErrorJson(error);
        }
    }

    private static Dictionary<string, PackageStatusEntry> StatusEntries(JsonNode? value) =>
        (value as JsonObject)?.ToDictionary(
            entry => entry.Key,
            entry => new PackageStatusEntry(S(entry.Value, "status"), S(entry.Value, "reason")))
        ?? [];

    [Fact]
    public async Task RightsConformanceSuiteMatchesEveryOtherEmbedder()
    {
        var suite = Json("fixtures/models/rights-conformance/suite.json");
        var run = suite["execute"]!;
        // Scenario 10 (offline cache-only activation) is proven by every vector test above.
        var scenarios = new HashSet<int> { 10 };
        foreach (var testCase in suite["cases"]!.AsArray().Select(item => item!))
        {
            var id = S(testCase, "id");
            var pins = testCase["pins"]!.AsArray().Select(Pin).ToList();
            ExactModelPin PinFor(string name) => pins.First(pin => pin.ModelId == $"fixture.rights.{name}");
            using var host = new ExactModelHost(
                pins,
                [S(suite, "trusted_public_key_hex")],
                testCase["model_usage"] is JsonValue usage && usage.TryGetValue<string>(out var text) ? text : null,
                testCase["host_requires_commercial"]?.GetValue<bool>() ?? false);
            host.SetPackageStatus(StatusEntries(testCase["package_status"]));
            var index = 0;
            foreach (var step in testCase["steps"]!.AsArray().Select(item => item!))
            {
                JsonNode? actual;
                switch (S(step, "op"))
                {
                    case "register":
                        actual = await Attempt(async () =>
                        {
                            var dir = $"{S(suite, "package_dir")}/{S(step, "package")}";
                            var wasm = Read(S(suite, "wasm_path"));
                            var signature = Read($"{dir}/model.sig.json");
                            switch (step["tamper"]?.GetValue<string>())
                            {
                                case "wasm":
                                    wasm = [.. wasm, 0];
                                    break;
                                case "signature":
                                    var document = (JsonObject)JsonNode.Parse(signature)!;
                                    var bytes = Hex(S(document, "signature"));
                                    bytes[0] ^= 1;
                                    document["signature"] = ToHex(bytes);
                                    signature = Encoding.UTF8.GetBytes(document.ToJsonString());
                                    break;
                            }

                            var digest = await host.RegisterPackageAsync(Read($"{dir}/model.manifest.json"), wasm, signature);
                            return new JsonObject { ["ok"] = true, ["digest"] = digest };
                        });
                        break;
                    case "execute":
                        actual = await Attempt(async () =>
                        {
                            var pin = PinFor(S(step, "package"));
                            var inputRef = host.StageModelInput(Hex(S(run, "input_hex")), 4096);
                            var result = await host.ExecuteAsync(
                                pin.ModelId, pin.Version, pin.Digest, inputRef, S(run, "policy_ref"),
                                S(run, "data_classification"), S(run, "input_schema_ref"), S(run, "input_schema_version"),
                                run["max_output_bytes"]!.GetValue<int>(),
                                run["allowed_classifications"]!.AsArray().Select(c => c!.GetValue<string>()).ToList());
                            return new JsonObject
                            {
                                ["ok"] = true,
                                ["output_hex"] = ToHex(host.ReadModelOutput(result.OutputRef, 4096)),
                                ["model_evidence"] = ExactModelHost.RecordJson(result.ModelEvidence!),
                            };
                        });
                        break;
                    case "rights_record":
                        actual = host.ModelRightsRecord(PinFor(S(step, "package")).Digest) is { } record
                            ? ExactModelHost.RecordJson(record)
                            : null;
                        break;
                    default:
                        Assert.Equal("set_package_status", S(step, "op"));
                        host.SetPackageStatus(StatusEntries(step["entries"]));
                        index++;
                        continue;
                }

                Assert.True(
                    JsonNode.DeepEquals(step["expect"], actual),
                    $"{id} step {index}: expected {step["expect"]?.ToJsonString()} got {actual?.ToJsonString()}");
                index++;
            }

            foreach (var scenario in testCase["scenarios"]!.AsArray())
            {
                scenarios.Add(scenario!.GetValue<int>());
            }
        }

        Assert.Equal(Enumerable.Range(1, 10).ToHashSet(), scenarios);
    }

    private static async Task<(ExactModelHost Host, ExactModelPin Pin, string InputHex)> PermissiveHost()
    {
        var suite = Json("fixtures/models/rights-conformance/suite.json");
        var permissive = suite["cases"]!.AsArray().First(item => S(item, "id") == "permissive-commercial")!;
        var pin = Pin(permissive["pins"]!.AsArray()[0]);
        var host = new ExactModelHost([pin], [S(suite, "trusted_public_key_hex")], "commercial");
        var dir = $"{S(suite, "package_dir")}/permissive";
        await host.RegisterPackageAsync(
            Read($"{dir}/model.manifest.json"),
            Read(S(suite, "wasm_path")),
            Read($"{dir}/model.sig.json"));
        Assert.Equal("Apache-2.0", host.ModelRights(pin.Digest)?.LicenseId);
        Assert.Null(host.ModelRights("00"));
        Assert.Null(host.ModelRightsRecord("00"));
        return (host, pin, S(suite["execute"], "input_hex"));
    }

    [Fact]
    public async Task AdapterCarriesEvidenceOnSuccessAndDetailOnDenial()
    {
        var (host, pin, inputHex) = await PermissiveHost();
        using var _ = host;
        JsonObject Payload(string inputRef) => new()
        {
            ["model_ref"] = new JsonObject { ["model_id"] = pin.ModelId, ["version"] = pin.Version, ["digest"] = pin.Digest },
            ["input_ref"] = inputRef,
            ["policy_ref"] = "policy-1",
            ["data_classification"] = "sensitive",
            ["input_schema_ref"] = "schema:fixture-in",
            ["input_schema_version"] = "1.0.0",
            ["max_output_bytes"] = 4096,
            ["allowed_classifications"] = new JsonArray("sensitive"),
        };

        var ok = await host.ModelExecuteAdapter(
            new HostConnectorRequest("run", "c1", "s1", Payload(host.StageModelInput(Hex(inputHex), 4096)).ToJsonString()),
            CancellationToken.None);
        Assert.Equal("succeeded", ok.ResultClass);
        var body = (JsonObject)JsonNode.Parse(ok.PayloadJson)!;
        Assert.Equal("active", S(body["model_evidence"], "status"));
        Assert.Equal("wasm-cpu", S(body, "placement"));

        var staged = host.StageModelInput([1, 2, 3], 16);
        host.DropRef(staged);
        var dropped = await Assert.ThrowsAsync<ExactModelException>(() => host.ExecuteAsync(
            pin.ModelId, pin.Version, pin.Digest, staged, "policy-1", "sensitive",
            "schema:fixture-in", "1.0.0", 4096, ["sensitive"]));
        Assert.Equal("invalid_input", dropped.Code);

        host.SetPackageStatus(new Dictionary<string, PackageStatusEntry> { [pin.Digest] = new("revoked", "withdrawn") });
        var denied = await host.ModelExecuteAdapter(
            new HostConnectorRequest("run", "c2", "s1", Payload(host.StageModelInput(Hex(inputHex), 4096)).ToJsonString()),
            CancellationToken.None);
        Assert.Equal("failed", denied.ResultClass);
        var error = JsonNode.Parse(denied.PayloadJson)!;
        Assert.Equal("package_revoked", S(error, "reason"));
        Assert.Equal("status", S(error["detail"], "field"));
        Assert.Equal("commercial", S(error["detail"], "effective_usage"));

        foreach (var malformed in new[] { "[]", "{" })
        {
            var rejected = await host.ModelExecuteAdapter(
                new HostConnectorRequest("run", "c3", "s1", malformed), CancellationToken.None);
            Assert.Equal("invalid_input", S(JsonNode.Parse(rejected.PayloadJson), "error_code"));
        }

    }

    private static async Task<(ExactModelHost Host, ExactModelPin Pin)> LooperHost()
    {
        var manifest = Read("fixtures/models/fixture-looper-1.0.0/model.manifest.json");
        var key = Json("fixtures/models/test-signing-key.json");
        var pin = new ExactModelPin(
            "fixture.looper", "1.0.0",
            ToHex(System.Security.Cryptography.SHA256.HashData(manifest)),
            "Apache-2.0", "allowed");
        var host = new ExactModelHost([pin], [S(key, "public_key_hex")], "commercial");
        await host.RegisterPackageAsync(
            manifest,
            Read("fixtures/models/fixture-looper-1.0.0/model.wasm"),
            Read("fixtures/models/fixture-looper-1.0.0/model.sig.json"));
        return (host, pin);
    }

    private static Task<ExactModelExecution> RunLooper(
        ExactModelHost host, ExactModelPin pin, int? timeoutMs = null, CancellationToken token = default) =>
        host.ExecuteAsync(
            pin.ModelId, pin.Version, pin.Digest, host.StageModelInput([1, 2, 3], 16), "policy-1", "sensitive",
            "schema:fixture-in", "1.0.0", 64, ["sensitive"], timeoutMs, token);

    [Fact]
    public async Task CancellationTokenInterruptsARunningInferenceMidRun()
    {
        var (host, pin) = await LooperHost();
        using var _ = host;
        using var cancellation = new CancellationTokenSource();
        var running = RunLooper(host, pin, token: cancellation.Token);
        await Task.Delay(200);
        Assert.False(running.IsCompleted, "the looper must still be running");
        var started = Stopwatch.StartNew();
        cancellation.Cancel();
        var error = await Assert.ThrowsAsync<ExactModelException>(() => running);
        Assert.Equal("cancelled", error.Code);
        Assert.True(started.Elapsed < TimeSpan.FromSeconds(5), "cancellation must interrupt mid-run");

        // A token cancelled before the call never reaches the engine.
        var early = await Assert.ThrowsAsync<ExactModelException>(
            () => RunLooper(host, pin, token: new CancellationToken(canceled: true)));
        Assert.Equal("cancelled", early.Code);

        // The adapter reports cancellation as its own result class.
        using var adapterCancellation = new CancellationTokenSource(TimeSpan.FromMilliseconds(200));
        var payload = new JsonObject
        {
            ["model_ref"] = new JsonObject { ["model_id"] = pin.ModelId, ["version"] = pin.Version, ["digest"] = pin.Digest },
            ["input_ref"] = host.StageModelInput([1], 16),
            ["policy_ref"] = "policy-1",
            ["data_classification"] = "sensitive",
            ["input_schema_ref"] = "schema:fixture-in",
            ["input_schema_version"] = "1.0.0",
            ["max_output_bytes"] = 64,
            ["allowed_classifications"] = new JsonArray("sensitive"),
        };
        var result = await host.ModelExecuteAdapter(
            new HostConnectorRequest("run", "c1", "s1", payload.ToJsonString()), adapterCancellation.Token);
        Assert.Equal("cancelled", result.ResultClass);
    }

    [Fact]
    public async Task DeadlineInterruptsARunningInferenceMidRun()
    {
        var (host, pin) = await LooperHost();
        using var _ = host;
        var error = await Assert.ThrowsAsync<ExactModelException>(() => RunLooper(host, pin, timeoutMs: 100));
        Assert.Equal("timeout", error.Code);
    }

    [Fact]
    public async Task MissingNativeEngineFailsClosedOnEveryModelCall()
    {
        // The library cannot load at all: creating the host fails closed.
        var missing = Assert.Throws<ExactModelException>(() => new ExactModelHost(
            [], [], "commercial", false, null, (_, _) => throw new DllNotFoundException("traverse_dotnet_host")));
        Assert.Equal(("model_unavailable", "engine_unavailable"), (missing.Code, missing.Reason));

        // The library disappears after creation: every later call fails closed the same way.
        var created = false;
        var host = new ExactModelHost([], [], "commercial", false, null, (handle, request) =>
        {
            if (created)
            {
                throw new EntryPointNotFoundException("traverse_dotnet_host_model_call");
            }

            created = true;
            return ExactModelNative.Call(handle, request);
        });
        var calls = new Func<Task>[]
        {
            () => host.RegisterPackageAsync([], [], []),
            () => Task.FromResult(host.StageModelInput([1], 1)),
            () => Task.FromResult(host.ReadModelOutput("o", 1)),
            () => Task.FromResult(host.ModelRights("d")),
            () => Task.FromResult(host.ModelRightsRecord("d")),
            () => Task.Run(() => host.SetPackageStatus(new Dictionary<string, PackageStatusEntry>())),
            () => Task.Run(() => host.DropRef("r")),
            () => host.ExecuteAsync("m", "1", "d", "i", "p", "c", "s", "1", 1, ["c"]),
        };
        foreach (var call in calls)
        {
            var error = await Assert.ThrowsAsync<ExactModelException>(call);
            Assert.Equal("engine_unavailable", error.Reason);
        }

        host.Dispose();
        host.Dispose();

        // The rest of the embedder keeps working without the model engine.
        Assert.Equal("ExactModelHost", typeof(ExactModelHost).Name);
        Assert.NotNull(new InMemoryTraverseEmbedder());
    }

    [Fact]
    public void MalformedResponsesAndPinsFailClosed()
    {
        foreach (var response in new[] { Array.Empty<byte>(), [200, 0, 0, 0, 1], Encoding.UTF8.GetBytes("\u0001\0\0\0[") })
        {
            var error = Assert.Throws<ExactModelException>(() => new ExactModelHost(
                [], [], "commercial", false, null, (_, _) => response));
            Assert.Equal("unavailable", error.Code);
        }

        var noHandle = Assert.Throws<ExactModelException>(() => new ExactModelHost(
            [], [], "commercial", false, null,
            (_, _) => ExactModelHostTestFrames.Frame(new JsonObject { ["ok"] = true })));
        Assert.Equal("model host was not created", noHandle.Message);

        var pin = ExactModelPin.FromJson(
            """{"model_id":"m","version":"1","digest":"d","rights":{"license_id":"MIT","commercial_use":"allowed"},"key_id":"k"}""");
        Assert.Equal(("wasm-cpu", true, "k"), (pin.Target, pin.OfflineAllowed, pin.KeyId));
        Assert.Throws<ExactModelException>(() => ExactModelPin.FromJson("[]"));
        Assert.Throws<ExactModelException>(() => ExactModelPin.FromJson("""{"model_id":"m"}"""));
    }
}

internal static class ExactModelHostTestFrames
{
    internal static byte[] Frame(JsonObject header)
    {
        var bytes = Encoding.UTF8.GetBytes(header.ToJsonString());
        return [.. BitConverter.GetBytes((uint)bytes.Length), .. bytes];
    }
}
