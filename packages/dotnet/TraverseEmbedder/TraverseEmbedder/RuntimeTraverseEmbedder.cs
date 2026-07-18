using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Traverse.Embedder;

/// <summary>Typed public embedder backed exclusively by runtime-owned bridge results.</summary>
public sealed class RuntimeTraverseEmbedder
{
    private readonly WasmtimeBridgeClient client;

    public RuntimeTraverseEmbedder(TraverseBundle bundle) : this(
        new WasmtimeBridgeClient(new WasmtimeRuntimeBridge(bundle)))
    {
    }

    public RuntimeTraverseEmbedder(WasmtimeBridgeClient client)
    {
        this.client = client;
    }

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

    public IReadOnlyList<TraverseRuntimeEvent> Subscribe()
    {
        var events = new List<TraverseRuntimeEvent>();
        while (client.NextEvent() is { } bytes)
        {
            using var result = Result(bytes);
            var value = result.RootElement;
            events.Add(new TraverseRuntimeEvent(
                RequiredInt(value, "sequence"),
                RequiredString(value, "target_id"),
                RequiredString(value, "status"),
                OptionalString(value, "instance_id")));
        }
        return events;
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

    public string Shutdown() => Text(client.Shutdown());

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
