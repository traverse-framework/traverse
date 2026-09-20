using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Traverse.Embedder;

/// <summary>Spec 139 / embedder-api 1.1.0 app-command submit envelope.</summary>
public sealed record TraverseAppCommand(string Command, string PayloadJson = "{}", string? SessionId = null)
{
    public void Validate()
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(Command);
        ArgumentException.ThrowIfNullOrWhiteSpace(PayloadJson);
    }
}

/// <summary>Host-provided monotonic timer port used for Spec 139 dual deadlines.</summary>
public interface ITraverseTimer
{
    /// <summary>Runs <paramref name="callback"/> once after <paramref name="delay"/>; disposing cancels it.</summary>
    IDisposable Schedule(TimeSpan delay, Action callback);
}

/// <summary>Default timer port backed by <see cref="System.Threading.Timer"/>.</summary>
public sealed class SystemTraverseTimer : ITraverseTimer
{
    public IDisposable Schedule(TimeSpan delay, Action callback)
    {
        Timer? timer = null;
        timer = new Timer(_ =>
        {
            timer?.Dispose();
            callback();
        }, null, delay < TimeSpan.Zero ? TimeSpan.Zero : delay, Timeout.InfiniteTimeSpan);
        return timer;
    }
}

/// <summary>A runtime-staged host-connector wait handed to a registered adapter.</summary>
public sealed record HostConnectorRequest(string Command, string CommandId, string SessionId, string PayloadJson);

/// <summary>
/// Adapter outcome. <see cref="ResultClass"/> is one of <c>succeeded</c>, <c>failed</c>,
/// <c>cancelled</c>, or <c>timeout</c>.
/// </summary>
public sealed record HostConnectorResult(string ResultClass, string PayloadJson = "{}");

/// <summary>
/// Host-side authority for one manifest command (Spec 140 WIT semantics). It never
/// runs inside runtime.wasm; the runtime only receives the correlated terminal.
/// </summary>
public delegate Task<HostConnectorResult> HostConnectorAdapter(
    HostConnectorRequest request,
    CancellationToken cancellationToken);

/// <summary>
/// Drives Spec 139 app commands. State-machine logic stays in runtime.wasm; this
/// type only submits envelopes, runs registered adapters for staged host-connector
/// waits, and registers the host half of the dual deadline. The first correlated
/// terminal wins; duplicates are ignored by the runtime.
/// </summary>
internal sealed class AppCommandCoordinator
{
    private readonly Func<byte[], byte[]> submit;
    private readonly ITraverseTimer timer;
    private readonly object gate = new();
    private readonly Dictionary<string, HostConnectorAdapter> adapters = [];
    private readonly List<IDisposable> deadlines = [];
    private readonly CancellationTokenSource shutdown = new();
    private bool stopped;

    public AppCommandCoordinator(Func<byte[], byte[]> submit, ITraverseTimer timer)
    {
        this.submit = submit;
        this.timer = timer;
    }

    public TraverseSubmissionResult Submit(TraverseAppCommand command)
    {
        command.Validate();
        var envelope = new JsonObject
        {
            ["kind"] = "app_command",
            ["command"] = command.Command,
            ["payload"] = JsonNode.Parse(command.PayloadJson),
        };
        if (command.SessionId is not null)
        {
            envelope["session_id"] = command.SessionId;
        }
        return Dispatch(envelope);
    }

    public IDisposable Register(string command, HostConnectorAdapter adapter)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(command);
        ArgumentNullException.ThrowIfNull(adapter);
        lock (gate)
        {
            adapters[command] = adapter;
        }
        return new Registration(() =>
        {
            lock (gate)
            {
                if (adapters.TryGetValue(command, out var current) && current == adapter)
                {
                    adapters.Remove(command);
                }
            }
        });
    }

    /// <summary>Cancels adapters and deadlines; late completions are dropped.</summary>
    public void Stop()
    {
        List<IDisposable> pending;
        lock (gate)
        {
            stopped = true;
            pending = [.. deadlines];
            deadlines.Clear();
        }
        shutdown.Cancel();
        foreach (var deadline in pending)
        {
            deadline.Dispose();
        }
    }

    private TraverseSubmissionResult Dispatch(JsonObject envelope)
    {
        using var response = JsonDocument.Parse(submit(Encoding.UTF8.GetBytes(envelope.ToJsonString())));
        var root = response.RootElement;
        var sessionId = RequiredString(root, "session_id");
        var status = RequiredString(root, "status");
        if (status != "accepted")
        {
            return new TraverseSubmissionResult(sessionId, status, OptionalString(root, "error"));
        }
        ScheduleDeadlines(root);
        StartHostConnectors(root, sessionId);
        return new TraverseSubmissionResult(sessionId, status);
    }

    private void ScheduleDeadlines(JsonElement response)
    {
        foreach (var deadline in Items(response, "pending_deadlines"))
        {
            if (OptionalString(deadline, "command_id") is not { } commandId ||
                OptionalString(deadline, "session_id") is not { } sessionId ||
                !deadline.TryGetProperty("deadline_ms", out var delay) ||
                delay.ValueKind != JsonValueKind.Number ||
                !delay.TryGetDouble(out var delayMs) ||
                !double.IsFinite(delayMs))
            {
                continue;
            }
            lock (gate)
            {
                if (stopped)
                {
                    return;
                }
                deadlines.Add(timer.Schedule(
                    TimeSpan.FromMilliseconds(Math.Max(0, delayMs)),
                    () => Terminal(new JsonObject
                    {
                        ["kind"] = "deadline_fired",
                        ["command_id"] = commandId,
                        ["session_id"] = sessionId,
                    })));
            }
        }
    }

    private void StartHostConnectors(JsonElement response, string fallbackSessionId)
    {
        foreach (var pending in Items(response, "pending_host_connector"))
        {
            if (OptionalString(pending, "command_id") is not { } commandId)
            {
                continue;
            }
            var sessionId = OptionalString(pending, "session_id") ?? fallbackSessionId;
            var command = OptionalString(pending, "command");
            HostConnectorAdapter? adapter = null;
            lock (gate)
            {
                if (command is not null)
                {
                    adapters.TryGetValue(command, out adapter);
                }
            }
            if (command is null || adapter is null)
            {
                Complete(commandId, sessionId, new HostConnectorResult("failed", "{\"code\":\"target_incompatible\"}"));
                continue;
            }
            var payload = pending.TryGetProperty("payload", out var value) ? value.GetRawText() : "{}";
            var request = new HostConnectorRequest(command, commandId, sessionId, payload);
            _ = RunAdapter(adapter, request);
        }
    }

    private async Task RunAdapter(HostConnectorAdapter adapter, HostConnectorRequest request)
    {
        HostConnectorResult result;
        try
        {
            result = await Task.Run(() => adapter(request, shutdown.Token)).ConfigureAwait(false);
        }
        catch (Exception)
        {
            result = new HostConnectorResult("failed", "{\"code\":\"execution_failed\"}");
        }
        Complete(request.CommandId, request.SessionId, result);
    }

    private void Complete(string commandId, string sessionId, HostConnectorResult result) =>
        Terminal(new JsonObject
        {
            ["kind"] = "host_connector_result",
            ["command_id"] = commandId,
            ["session_id"] = sessionId,
            ["result_class"] = result.ResultClass,
            ["payload"] = JsonNode.Parse(result.PayloadJson),
        });

    private void Terminal(JsonObject envelope)
    {
        lock (gate)
        {
            if (stopped)
            {
                return;
            }
        }
        try
        {
            Dispatch(envelope);
        }
        catch (Exception)
        {
            // Shutdown and first-terminal-wins are runtime-owned; there is no
            // host retry path for a terminal the runtime can no longer accept.
        }
    }

    private static IEnumerable<JsonElement> Items(JsonElement response, string name) =>
        response.TryGetProperty(name, out var array) && array.ValueKind == JsonValueKind.Array
            ? array.EnumerateArray().Where(item => item.ValueKind == JsonValueKind.Object).ToArray()
            : [];

    private static string RequiredString(JsonElement value, string name) =>
        OptionalString(value, name) ?? throw new TraverseBridgeException(-2, $"bridge result is missing {name}");

    private static string? OptionalString(JsonElement value, string name) =>
        value.TryGetProperty(name, out var property) && property.ValueKind == JsonValueKind.String
            ? property.GetString()
            : null;

    private sealed class Registration(Action dispose) : IDisposable
    {
        public void Dispose() => dispose();
    }
}
