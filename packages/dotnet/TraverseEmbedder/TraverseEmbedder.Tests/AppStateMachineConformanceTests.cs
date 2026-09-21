using System.Buffers.Binary;
using System.Collections.Concurrent;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Traverse.Embedder;
using Xunit;

namespace TraverseEmbedder.Tests;

/// <summary>
/// Cross-host ordered-event conformance (Spec 139 / Spec 140 FR-013, #1502). Runs
/// <c>fixtures/cross-host/app-state-machine-events-v1</c> against the real <c>runtime.wasm</c> and
/// compares each step with <c>golden.json</c>, which the Rust reference generated. Skipped unless
/// TRAVERSE_NATIVE_ARTIFACT_ROOT points at a directory containing <c>runtime/runtime.wasm</c>,
/// exactly like the other real-artifact tests.
/// </summary>
public sealed class AppStateMachineConformanceTests
{
    private const string FixtureDirectory = "fixtures/cross-host/app-state-machine-events-v1";

    /// <summary>Normalizes runtime-assigned ids to $S&lt;n&gt; / $C&lt;n&gt; by first appearance.</summary>
    private sealed class Placeholders
    {
        public List<string> Sessions { get; } = [];
        public List<string> Commands { get; } = [];

        private static string Name(List<string> list, string prefix, string id)
        {
            var index = list.IndexOf(id);
            if (index < 0)
            {
                list.Add(id);
                index = list.Count - 1;
            }
            return $"{prefix}{index + 1}";
        }

        public JsonNode? Normalize(JsonNode? node) => node switch
        {
            JsonObject obj => new JsonObject(obj.Select(pair => KeyValuePair.Create(pair.Key, pair.Key switch
            {
                "session_id" when pair.Value is JsonValue value && value.TryGetValue<string>(out var id) =>
                    (JsonNode?)Name(Sessions, "$S", id),
                "command_id" when pair.Value is JsonValue value && value.TryGetValue<string>(out var id) =>
                    (JsonNode?)Name(Commands, "$C", id),
                _ => Normalize(pair.Value),
            }))),
            JsonArray array => new JsonArray(array.Select(item => Normalize(item)).ToArray()),
            null => null,
            _ => node.DeepClone(),
        };

        public static string Resolve(List<string> list, string placeholder, string prefix) =>
            list[int.Parse(placeholder[prefix.Length..]) - 1];
    }

    private static string? ArtifactRoot()
    {
        var root = Environment.GetEnvironmentVariable("TRAVERSE_NATIVE_ARTIFACT_ROOT");
        return string.IsNullOrWhiteSpace(root) ? null : root;
    }

    private static string FixturePath(string file)
    {
        for (var directory = new DirectoryInfo(AppContext.BaseDirectory); directory is not null; directory = directory.Parent)
        {
            var candidate = Path.Join(directory.FullName, FixtureDirectory, file);
            if (File.Exists(candidate)) return candidate;
        }
        throw new FileNotFoundException($"{FixtureDirectory}/{file} not found above the test binary");
    }

    private static JsonNode Load(string file) => JsonNode.Parse(File.ReadAllText(FixturePath(file)))!;

    private static (TraverseBundle Bundle, byte[] Init) Prepare(string root, JsonNode fixture)
    {
        var runtime = File.ReadAllBytes(Path.Join(root, "runtime", "runtime.wasm"));
        var digest = "sha256:" + Convert.ToHexString(SHA256.HashData(runtime)).ToLowerInvariant();
        var header = Encoding.UTF8.GetBytes(fixture["init_header"]!.ToJsonString());
        var init = new byte[4 + header.Length];
        BinaryPrimitives.WriteUInt32LittleEndian(init, (uint)header.Length);
        header.CopyTo(init, 4);
        return (new TraverseBundle(root, digest), init);
    }

    private static WasmtimeBridgeClient NewClient(TraverseBundle bundle) =>
        new(new WasmtimeRuntimeBridge(bundle, fuelPerCall: 50_000_000));

    private static JsonNode? TryParse(string text)
    {
        try { return JsonNode.Parse(text); }
        catch (JsonException) { return null; }
    }

    /// <summary>Runs one scenario at the bridge-client level; returns per-step transcripts.</summary>
    private static JsonArray RunScenario(WasmtimeBridgeClient client, byte[] init, JsonNode scenario)
    {
        Assert.Equal("ready", JsonNode.Parse(client.Initialize(init))!["status"]!.GetValue<string>());
        var names = new Placeholders();
        var pending = new List<(string Session, string Command)>();
        var transcript = new JsonArray();

        foreach (var (step, index) in scenario["steps"]!.AsArray().Select((step, index) => (step!.AsObject(), index)))
        {
            var (kind, spec) = step.Select(pair => (pair.Key, pair.Value!)).Single();
            (string Session, string Command) TargetWait() => spec["command"] is JsonNode placeholder
                ? pending.First(wait => wait.Command == Placeholders.Resolve(names.Commands, placeholder.GetValue<string>(), "$C"))
                : pending[^1];
            JsonObject request;
            switch (kind)
            {
                case "submit":
                    request = new JsonObject
                    {
                        ["kind"] = "app_command",
                        ["command"] = spec["command"]!.DeepClone(),
                        ["payload"] = spec["payload"]!.DeepClone(),
                    };
                    if (spec["session"] is JsonNode session)
                    {
                        request["session_id"] = Placeholders.Resolve(names.Sessions, session.GetValue<string>(), "$S");
                    }
                    break;
                case "complete":
                    var wait = TargetWait();
                    request = new JsonObject
                    {
                        ["kind"] = "host_connector_result",
                        ["command_id"] = wait.Command,
                        ["session_id"] = wait.Session,
                        ["result_class"] = spec["result_class"]!.DeepClone(),
                        ["payload"] = spec["payload"]!.DeepClone(),
                    };
                    break;
                case "fire_deadline":
                    var due = TargetWait();
                    request = new JsonObject { ["kind"] = "deadline_fired", ["command_id"] = due.Command, ["session_id"] = due.Session };
                    break;
                default:
                    throw new InvalidOperationException($"unknown step kind {kind}");
            }

            var entry = new JsonObject { ["step"] = index, ["kind"] = kind };
            JsonNode? response;
            try
            {
                response = JsonNode.Parse(client.Submit(Encoding.UTF8.GetBytes(request.ToJsonString())));
                entry["guest_status"] = 0;
            }
            catch (TraverseBridgeException error)
            {
                // A rejected submit throws; the guest still wrote the response body as the message.
                entry["guest_status"] = error.Status;
                response = TryParse(error.Message);
            }
            foreach (var item in response?["pending_host_connector"]?.AsArray() ?? [])
            {
                pending.Add((item!["session_id"]!.GetValue<string>(), item["command_id"]!.GetValue<string>()));
            }
            entry["response"] = names.Normalize(response);
            var events = new JsonArray();
            while (client.NextEvent() is { } bytes) events.Add(JsonNode.Parse(bytes));
            entry["events"] = names.Normalize(events);
            transcript.Add(entry);
        }
        return transcript;
    }

    [Fact]
    public void DotNetHostReproducesTheGoldenOrderedEventLog()
    {
        if (ArtifactRoot() is not { } root) return;
        var fixture = Load("fixture.json");
        var golden = Load("golden.json")["scenarios"]!;
        var (bundle, init) = Prepare(root, fixture);
        var divergences = new List<string>();

        foreach (var scenario in fixture["scenarios"]!.AsArray())
        {
            var id = scenario!["id"]!.GetValue<string>();
            var actual = RunScenario(NewClient(bundle), init, scenario);
            var want = golden[id]!.AsArray();
            if (actual.Count != want.Count)
            {
                divergences.Add($"scenario {id}: {actual.Count} steps, expected {want.Count}");
                continue;
            }
            for (var index = 0; index < want.Count; index++)
            {
                var step = index;
                foreach (var field in new[] { "kind", "guest_status", "response" }
                             .Where(field => !JsonNode.DeepEquals(actual[step]![field], want[step]![field])))
                {
                    divergences.Add($"scenario {id} step {step} {field}: expected {want[step]![field]?.ToJsonString()}, got {actual[step]![field]?.ToJsonString()}");
                }
                var got = actual[index]!["events"]!.AsArray();
                var wanted = want[index]!["events"]!.AsArray();
                for (var eventIndex = 0; eventIndex < Math.Max(got.Count, wanted.Count); eventIndex++)
                {
                    var g = eventIndex < got.Count ? got[eventIndex] : null;
                    var w = eventIndex < wanted.Count ? wanted[eventIndex] : null;
                    if (!JsonNode.DeepEquals(g, w))
                    {
                        divergences.Add($"scenario {id} step {index} event {eventIndex}: expected {w?.ToJsonString()}, got {g?.ToJsonString()}");
                    }
                }
            }
        }
        Assert.True(divergences.Count == 0, string.Join("\n", divergences));
    }

    // ---- Public API level ----

    private sealed class ScriptedTimer : ITraverseTimer
    {
        private readonly List<Action> callbacks = [];

        public IDisposable Schedule(TimeSpan delay, Action callback)
        {
            lock (callbacks) callbacks.Add(callback);
            return new Noop();
        }

        public bool FireLatest()
        {
            Action? callback;
            lock (callbacks) callback = callbacks.Count == 0 ? null : callbacks[^1];
            if (callback is null) return false;
            callback();
            return true;
        }

        private sealed class Noop : IDisposable
        {
            public void Dispose() { }
        }
    }

    /// <summary>
    /// Raw terminal injection (an explicit <c>command</c> target) cannot be expressed through
    /// adapters and the timer port, so those scenarios run at the bridge level only.
    /// </summary>
    private static bool Expressible(JsonNode? scenario) => scenario!["steps"]!.AsArray().All(step =>
        step!["complete"]?["command"] is null && step["fire_deadline"]?["command"] is null);

    private sealed record WaitAction(string? ResultClass, string Payload);

    [Fact]
    public async Task DotNetPublicSubscribeDeliversTheGoldenAppEventsInOrder()
    {
        if (ArtifactRoot() is not { } root) return;
        var fixture = Load("fixture.json");
        var golden = Load("golden.json")["scenarios"]!;
        var (bundle, init) = Prepare(root, fixture);
        var hostCommands = fixture["init_header"]!["state_machine"]!["states"]!.AsArray()
            .Select(state => state!["invoke"]?["host_connector"]?.GetValue<string>())
            .OfType<string>().Distinct().ToArray();
        var divergences = new List<string>();
        var exercised = 0;

        foreach (var scenario in fixture["scenarios"]!.AsArray().Where(Expressible))
        {
            exercised++;
            var id = scenario!["id"]!.GetValue<string>();
            var steps = scenario["steps"]!.AsArray();
            var goldenSteps = golden[id]!.AsArray();

            // One action per staged wait, in order: a host result, or "let the deadline win".
            var actions = new ConcurrentQueue<WaitAction>(steps.Select(step => step!["complete"] is JsonNode complete
                ? new WaitAction(complete["result_class"]!.GetValue<string>(), complete["payload"]!.ToJsonString())
                : step["fire_deadline"] is not null ? new WaitAction(null, "{}") : null).OfType<WaitAction>());
            var timer = new ScriptedTimer();
            var client = NewClient(bundle);
            client.Initialize(init);
            var embedder = new RuntimeTraverseEmbedder(client, timer);
            foreach (var command in hostCommands)
            {
                embedder.RegisterHostConnectorAdapter(command, async (_, token) =>
                {
                    if (actions.TryDequeue(out var next) && next.ResultClass is { } resultClass)
                    {
                        return new HostConnectorResult(resultClass, next.Payload);
                    }
                    await Task.Delay(Timeout.Infinite, token); // the deadline fires instead
                    throw new InvalidOperationException("unreachable");
                });
            }

            var names = new Placeholders();
            string? sessionId = null;
            var received = new List<TraverseRuntimeEvent>();
            var expectedTotal = 0;
            for (var index = 0; index < steps.Count; index++)
            {
                var step = steps[index]!;
                expectedTotal += goldenSteps[index]!["events"]!.AsArray().Count;
                if (step["submit"] is JsonNode spec)
                {
                    var result = embedder.Submit(new TraverseAppCommand(
                        spec["command"]!.GetValue<string>(), spec["payload"]!.ToJsonString(),
                        spec["session"] is null ? null : sessionId));
                    sessionId ??= result.SessionId;
                    var goldenResponse = goldenSteps[index]!["response"]!;
                    if (goldenResponse["status"]!.GetValue<string>() == "rejected")
                    {
                        Assert.Equal("rejected", result.Status);
                        Assert.Equal(goldenResponse["error"]!.GetValue<string>(), result.Error);
                    }
                    else
                    {
                        Assert.Equal("accepted", result.Status);
                    }
                }
                else if (step["fire_deadline"] is not null)
                {
                    Assert.True(timer.FireLatest(), $"{id} step {index}: no deadline was registered");
                }
                for (var attempt = 0; attempt < 500 && received.Count < expectedTotal; attempt++)
                {
                    received.AddRange(embedder.Subscribe());
                    if (received.Count < expectedTotal) await Task.Delay(10);
                }
                // Adapters complete asynchronously, so a step may already include later events.
                if (received.Count < expectedTotal)
                {
                    divergences.Add($"scenario {id} step {index}: only {received.Count} of {expectedTotal} events arrived");
                }
            }
            await Task.Delay(50); // let any surplus event arrive before checking the exact total
            received.AddRange(embedder.Subscribe());
            if (received.Count != expectedTotal)
            {
                divergences.Add($"scenario {id}: {received.Count} events, expected {expectedTotal}");
            }
            embedder.Shutdown();

            var wanted = goldenSteps.SelectMany(step => step!["events"]!.AsArray()).ToArray();
            for (var eventIndex = 0; eventIndex < Math.Min(received.Count, wanted.Length); eventIndex++)
            {
                var got = received[eventIndex];
                var actual = names.Normalize(new JsonObject
                {
                    ["type"] = got.EventType,
                    ["session_id"] = got.SessionId,
                    ["data"] = JsonNode.Parse(got.Output ?? "{}"),
                });
                if (!JsonNode.DeepEquals(actual, wanted[eventIndex]))
                {
                    divergences.Add($"scenario {id} event {eventIndex}: expected {wanted[eventIndex]!.ToJsonString()}, got {actual!.ToJsonString()}");
                }
            }
            Assert.Equal(Enumerable.Range(1, received.Count), received.Select(@event => @event.Sequence));
        }
        Assert.True(exercised >= 7, "expected the adapter-expressible scenarios to run");
        Assert.True(divergences.Count == 0, string.Join("\n", divergences));
    }
}
