using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Traverse.Embedder;

/// <summary>Typed public embedder backed exclusively by runtime-owned bridge results.</summary>
public sealed class RuntimeTraverseEmbedder
{
    private readonly WasmtimeBridgeClient client;
    private readonly AppCommandCoordinator appCommands;

    public RuntimeTraverseEmbedder(TraverseBundle bundle) : this(
        new WasmtimeBridgeClient(new WasmtimeRuntimeBridge(bundle)))
    {
    }

    public RuntimeTraverseEmbedder(WasmtimeBridgeClient client, ITraverseTimer? timer = null)
    {
        this.client = client;
        appCommands = new AppCommandCoordinator(
            request => client.Submit(request), timer ?? new SystemTraverseTimer());
    }

    /// <summary>
    /// Spec 139 <c>app_command</c> submit. The state machine runs in runtime.wasm;
    /// registered adapters and the timer port complete host-connector waits.
    /// </summary>
    public TraverseSubmissionResult Submit(TraverseAppCommand command) => appCommands.Submit(command);

    /// <summary>
    /// Registers the host authority for a manifest command. Disposing removes it
    /// only if it is still the registered adapter.
    /// </summary>
    public IDisposable RegisterHostConnectorAdapter(string command, HostConnectorAdapter adapter) =>
        appCommands.Register(command, adapter);

    public string Initialize(string configJson) => Text(client.Initialize(Encoding.UTF8.GetBytes(configJson)));

    public TraverseSubmissionResult Submit(TraverseSubmission submission)
    {
        submission.Validate();
        var request = new JsonObject
        {
            ["target_id"] = submission.TargetId,
            ["input"] = JsonNode.Parse(submission.InputJson),
        };
        using var result = Result(client.Submit(Encoding.UTF8.GetBytes(request.ToJsonString())));
        return new TraverseSubmissionResult(
            RequiredString(result.RootElement, "session_id"),
            RequiredString(result.RootElement, "status"));
    }

    /// <summary>
    /// Drains ordered runtime events. Legacy bridge events (<c>sequence</c>, <c>target_id</c>,
    /// <c>status</c>) are parsed as before. Spec 139 app lifecycle events (<c>type</c>,
    /// <c>session_id</c>, <c>data</c>) map to <c>EventType</c>, <c>SessionId</c>, and
    /// <c>Output</c> (also <c>ErrorData</c> for <c>error</c>), numbered in arrival order, so
    /// state-machine events are observable on every embedder (Spec 139 FR-004).
    /// </summary>
    public IReadOnlyList<TraverseRuntimeEvent> Subscribe()
    {
        var events = new List<TraverseRuntimeEvent>();
        while (client.NextEvent() is { } bytes)
        {
            using var result = Result(bytes);
            var value = result.RootElement;
            if (!value.TryGetProperty("sequence", out _) && OptionalString(value, "type") is { } type)
            {
                events.Add(MapLifecycleEvent(type, value, Interlocked.Increment(ref eventSequence)));
                continue;
            }
            events.Add(new TraverseRuntimeEvent(
                RequiredInt(value, "sequence"),
                RequiredString(value, "target_id"),
                RequiredString(value, "status"),
                OptionalString(value, "instance_id")));
        }
        return events;
    }

    private static readonly HashSet<string> LifecycleEventTypes =
    [
        "state_changed", "capability_invoked", "capability_result", "capability_event",
        "capability_succeeded", "capability_failed", "host_connector_succeeded",
        "host_connector_failed", "host_connector_cancelled", "host_connector_timeout",
        "error", "heartbeat",
    ];

    private int eventSequence;

    internal static TraverseRuntimeEvent MapLifecycleEvent(string type, JsonElement value, int sequence)
    {
        var data = value.TryGetProperty("data", out var payload) ? payload.GetRawText() : "{}";
        var eventType = LifecycleEventTypes.Contains(type) ? type : "error";
        return new TraverseRuntimeEvent(
            sequence,
            "app_command",
            "emitted",
            EventType: eventType,
            SessionId: OptionalString(value, "session_id"),
            ErrorData: eventType == "error" ? data : null,
            Output: data);
    }

    public string Cancel(string sessionId)
    {
        var request = new JsonObject { ["session_id"] = sessionId };
        return Text(client.Cancel(Encoding.UTF8.GetBytes(request.ToJsonString())));
    }

    public TraverseCompatibleResult CompatibleStart(string capabilityId, string inputJson)
    {
        var request = new JsonObject
        {
            ["capability_id"] = capabilityId,
            ["input"] = JsonNode.Parse(inputJson),
        };
        return CompatibleResult(client.CompatibleStart(Encoding.UTF8.GetBytes(request.ToJsonString())));
    }

    public TraverseCompatibleResult CompatibleStop(string capabilityId, string? instanceId) =>
        CompatibleResult(client.CompatibleStop(Encoding.UTF8.GetBytes(CompatibleRequest(capabilityId, instanceId))));

    public TraverseCompatibleResult CompatibleKill(string capabilityId, string? instanceId) =>
        CompatibleResult(client.CompatibleKill(Encoding.UTF8.GetBytes(CompatibleRequest(capabilityId, instanceId))));

    public string Shutdown()
    {
        appCommands.Stop();
        return Text(client.Shutdown());
    }

    private static string CompatibleRequest(string capabilityId, string? instanceId) =>
        new JsonObject { ["capability_id"] = capabilityId, ["instance_id"] = instanceId }.ToJsonString();

    private static TraverseCompatibleResult CompatibleResult(byte[] bytes)
    {
        using var result = Result(bytes);
        return new TraverseCompatibleResult(
            OptionalString(result.RootElement, "instance_id"),
            RequiredString(result.RootElement, "status"));
    }

    private static JsonDocument Result(byte[] bytes)
    {
        try
        {
            return JsonDocument.Parse(bytes);
        }
        catch (JsonException error)
        {
            throw new TraverseBridgeException(-2, $"bridge_invalid_json: {error.Message}");
        }
    }

    private static string RequiredString(JsonElement value, string name) =>
        value.TryGetProperty(name, out var property) && property.ValueKind == JsonValueKind.String
            ? property.GetString()!
            : throw new TraverseBridgeException(-2, $"bridge result is missing {name}");

    private static string? OptionalString(JsonElement value, string name) =>
        !value.TryGetProperty(name, out var property) || property.ValueKind == JsonValueKind.Null
            ? null
            : property.GetString();

    private static int RequiredInt(JsonElement value, string name) =>
        value.TryGetProperty(name, out var property) && property.TryGetInt32(out var result)
            ? result
            : throw new TraverseBridgeException(-2, $"bridge result is missing {name}");

    private static string Text(byte[] bytes) => Encoding.UTF8.GetString(bytes);
}
