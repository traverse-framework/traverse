using System.Text;
using System.Text.Json.Nodes;
using Traverse.Embedder;
using Xunit;

namespace TraverseEmbedder.Tests;

public sealed class AppCommandTests
{
    private static readonly TimeSpan Wait = TimeSpan.FromSeconds(5);

    private sealed class FakeBridge
    {
        private readonly object gate = new();
        private readonly List<JsonObject> submitted = [];
        public string AppResponse { get; set; } = "{\"session_id\":\"s1\",\"status\":\"accepted\"}";
        public string TerminalResponse { get; set; } = "{\"session_id\":\"s1\",\"status\":\"accepted\"}";
        public bool ThrowOnTerminal { get; set; }
        public TaskCompletionSource<JsonObject> Terminal { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

        public IReadOnlyList<JsonObject> Submitted
        {
            get { lock (gate) { return [.. submitted]; } }
        }

        public byte[] Submit(byte[] request)
        {
            var envelope = JsonNode.Parse(request)!.AsObject();
            lock (gate)
            {
                submitted.Add(envelope);
            }
            if (envelope["kind"]!.GetValue<string>() == "app_command")
            {
                return Encoding.UTF8.GetBytes(AppResponse);
            }
            Terminal.TrySetResult(envelope);
            if (ThrowOnTerminal)
            {
                throw new TraverseBridgeException(-1, "runtime stopped");
            }
            return Encoding.UTF8.GetBytes(TerminalResponse);
        }
    }

    private sealed class ManualTimer : ITraverseTimer
    {
        public List<(TimeSpan Delay, Action Callback, Cancellation Handle)> Scheduled { get; } = [];

        public IDisposable Schedule(TimeSpan delay, Action callback)
        {
            var handle = new Cancellation();
            Scheduled.Add((delay, callback, handle));
            return handle;
        }

        public sealed class Cancellation : IDisposable
        {
            public bool Disposed { get; private set; }
            public void Dispose() => Disposed = true;
        }
    }

    private static string Pending(string command = "capture_audio", string commandId = "cmd-1") =>
        "{\"session_id\":\"s1\",\"status\":\"accepted\",\"pending_host_connector\":[{\"command\":\""
        + command + "\",\"command_id\":\"" + commandId + "\",\"session_id\":\"s1\",\"payload\":{\"max_bytes\":8}}],"
        + "\"pending_deadlines\":[{\"command_id\":\"" + commandId + "\",\"session_id\":\"s1\",\"deadline_ms\":30000}]}";

    private static AppCommandCoordinator Coordinator(FakeBridge bridge, ITraverseTimer? timer = null) =>
        new(bridge.Submit, timer ?? new ManualTimer());

    [Fact]
    public void SubmitSendsTheAppCommandEnvelopeAndReturnsTheRuntimeSession()
    {
        var bridge = new FakeBridge();
        var result = Coordinator(bridge).Submit(new TraverseAppCommand("start", "{\"a\":1}", "sess-9"));

        Assert.Equal(new TraverseSubmissionResult("s1", "accepted"), result);
        var envelope = Assert.Single(bridge.Submitted);
        Assert.Equal("app_command", envelope["kind"]!.GetValue<string>());
        Assert.Equal("start", envelope["command"]!.GetValue<string>());
        Assert.Equal(1, envelope["payload"]!["a"]!.GetValue<int>());
        Assert.Equal("sess-9", envelope["session_id"]!.GetValue<string>());
    }

    [Fact]
    public void SubmitOmitsSessionIdAndSurfacesRuntimeRejection()
    {
        var bridge = new FakeBridge { AppResponse = "{\"session_id\":\"s2\",\"status\":\"rejected\",\"error\":\"ambiguous\"}" };
        var result = Coordinator(bridge).Submit(new TraverseAppCommand("start"));

        Assert.Equal(new TraverseSubmissionResult("s2", "rejected", "ambiguous"), result);
        Assert.False(bridge.Submitted[0].ContainsKey("session_id"));

        bridge.AppResponse = "{\"session_id\":\"s3\",\"status\":\"rejected\"}";
        Assert.Null(Coordinator(bridge).Submit(new TraverseAppCommand("start")).Error);
    }

    [Fact]
    public void SubmitRejectsInvalidCommandsAndMalformedRuntimeResults()
    {
        var bridge = new FakeBridge();
        var coordinator = Coordinator(bridge);
        Assert.Throws<ArgumentException>(() => coordinator.Submit(new TraverseAppCommand(" ")));
        Assert.Throws<ArgumentException>(() => coordinator.Submit(new TraverseAppCommand("go", " ")));
        Assert.Empty(bridge.Submitted);

        bridge.AppResponse = "{\"status\":\"accepted\"}";
        var error = Assert.Throws<TraverseBridgeException>(() => coordinator.Submit(new TraverseAppCommand("go")));
        Assert.Contains("session_id", error.Message);
    }

    [Fact]
    public async Task RegisteredAdapterCompletesTheHostConnectorWait()
    {
        var bridge = new FakeBridge { AppResponse = Pending() };
        var coordinator = Coordinator(bridge);
        HostConnectorRequest? seen = null;
        coordinator.Register("capture_audio", (request, _) =>
        {
            seen = request;
            return Task.FromResult(new HostConnectorResult("succeeded", "{\"artifact_ref\":\"artifact-1\"}"));
        });

        coordinator.Submit(new TraverseAppCommand("record"));
        var terminal = await bridge.Terminal.Task.WaitAsync(Wait);

        Assert.Equal(new HostConnectorRequest("capture_audio", "cmd-1", "s1", "{\"max_bytes\":8}"), seen);
        Assert.Equal("host_connector_result", terminal["kind"]!.GetValue<string>());
        Assert.Equal("cmd-1", terminal["command_id"]!.GetValue<string>());
        Assert.Equal("s1", terminal["session_id"]!.GetValue<string>());
        Assert.Equal("succeeded", terminal["result_class"]!.GetValue<string>());
        Assert.Equal("artifact-1", terminal["payload"]!["artifact_ref"]!.GetValue<string>());
    }

    [Fact]
    public async Task MissingAdapterFailsTargetIncompatibleWithoutRunningAnything()
    {
        var bridge = new FakeBridge { AppResponse = Pending("unregistered") };
        Coordinator(bridge).Submit(new TraverseAppCommand("record"));
        var terminal = await bridge.Terminal.Task.WaitAsync(Wait);

        Assert.Equal("failed", terminal["result_class"]!.GetValue<string>());
        Assert.Equal("target_incompatible", terminal["payload"]!["code"]!.GetValue<string>());
    }

    [Fact]
    public async Task AdapterFailureBecomesExecutionFailed()
    {
        var bridge = new FakeBridge { AppResponse = Pending() };
        var coordinator = Coordinator(bridge);
        coordinator.Register("capture_audio", (_, _) => throw new InvalidOperationException("/Users/x/device 0x1f"));
        coordinator.Submit(new TraverseAppCommand("record"));
        var terminal = await bridge.Terminal.Task.WaitAsync(Wait);

        Assert.Equal("failed", terminal["result_class"]!.GetValue<string>());
        Assert.Equal("execution_failed", terminal["payload"]!["code"]!.GetValue<string>());
        Assert.DoesNotContain("Users", terminal.ToJsonString());
    }

    [Fact]
    public void DeadlineIsRegisteredOnTheTimerPortAndFiresDeadlineFired()
    {
        var bridge = new FakeBridge { AppResponse = Pending() };
        var timer = new ManualTimer();
        var coordinator = Coordinator(bridge, timer);
        coordinator.Register("capture_audio", (_, token) => new TaskCompletionSource<HostConnectorResult>().Task);
        coordinator.Submit(new TraverseAppCommand("record"));

        var scheduled = Assert.Single(timer.Scheduled);
        Assert.Equal(TimeSpan.FromMilliseconds(30000), scheduled.Delay);
        scheduled.Callback();

        var terminal = bridge.Submitted.Last();
        Assert.Equal("deadline_fired", terminal["kind"]!.GetValue<string>());
        Assert.Equal("cmd-1", terminal["command_id"]!.GetValue<string>());
        Assert.Equal("s1", terminal["session_id"]!.GetValue<string>());
    }

    [Fact]
    public void MalformedPendingEntriesAreSkipped()
    {
        var bridge = new FakeBridge
        {
            AppResponse = "{\"session_id\":\"s1\",\"status\":\"accepted\","
                + "\"pending_host_connector\":[7,{\"command\":\"x\"}],"
                + "\"pending_deadlines\":[{\"command_id\":\"c\"},{\"command_id\":\"c\",\"session_id\":\"s1\"},"
                + "{\"command_id\":\"c\",\"session_id\":\"s1\",\"deadline_ms\":\"soon\"}],"
                + "\"unrelated\":1}",
        };
        var timer = new ManualTimer();
        Coordinator(bridge, timer).Submit(new TraverseAppCommand("record"));

        Assert.Empty(timer.Scheduled);
        Assert.Single(bridge.Submitted);

        bridge.AppResponse = "{\"session_id\":\"s1\",\"status\":\"accepted\",\"pending_host_connector\":\"no\"}";
        Coordinator(bridge, timer).Submit(new TraverseAppCommand("record"));
        Assert.Equal(2, bridge.Submitted.Count);
    }

    [Fact]
    public void NegativeDeadlinesFireImmediately()
    {
        var bridge = new FakeBridge
        {
            AppResponse = "{\"session_id\":\"s1\",\"status\":\"accepted\",\"pending_deadlines\":"
                + "[{\"command_id\":\"c\",\"session_id\":\"s1\",\"deadline_ms\":-5}]}",
        };
        var timer = new ManualTimer();
        Coordinator(bridge, timer).Submit(new TraverseAppCommand("record"));
        Assert.Equal(TimeSpan.Zero, Assert.Single(timer.Scheduled).Delay);
    }

    [Fact]
    public async Task StopCancelsAdaptersDisposesDeadlinesAndDropsLateTerminals()
    {
        var bridge = new FakeBridge { AppResponse = Pending() };
        var timer = new ManualTimer();
        var coordinator = Coordinator(bridge, timer);
        var started = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var release = new TaskCompletionSource<HostConnectorResult>(TaskCreationOptions.RunContinuationsAsynchronously);
        CancellationToken observed = default;
        coordinator.Register("capture_audio", (_, token) =>
        {
            observed = token;
            started.SetResult();
            return release.Task;
        });
        coordinator.Submit(new TraverseAppCommand("record"));
        await started.Task.WaitAsync(Wait);

        coordinator.Stop();
        Assert.True(observed.IsCancellationRequested);
        Assert.True(timer.Scheduled[0].Handle.Disposed);

        release.SetResult(new HostConnectorResult("succeeded"));
        timer.Scheduled[0].Callback();
        await Task.Delay(50);
        Assert.Single(bridge.Submitted);

        // Deadlines staged after stop are never registered.
        coordinator.Submit(new TraverseAppCommand("again"));
        Assert.Single(timer.Scheduled);
    }

    [Fact]
    public async Task TerminalSubmitFailureIsSwallowedBecauseShutdownIsRuntimeOwned()
    {
        var bridge = new FakeBridge { AppResponse = Pending(), ThrowOnTerminal = true };
        var coordinator = Coordinator(bridge);
        coordinator.Register("capture_audio", (_, _) => Task.FromResult(new HostConnectorResult("cancelled")));
        coordinator.Submit(new TraverseAppCommand("record"));
        var terminal = await bridge.Terminal.Task.WaitAsync(Wait);
        Assert.Equal("cancelled", terminal["result_class"]!.GetValue<string>());
    }

    [Fact]
    public async Task DisposingARegistrationRemovesOnlyTheSameAdapter()
    {
        var bridge = new FakeBridge { AppResponse = Pending() };
        var coordinator = Coordinator(bridge);
        HostConnectorAdapter first = (_, _) => Task.FromResult(new HostConnectorResult("succeeded"));
        HostConnectorAdapter second = (_, _) => Task.FromResult(new HostConnectorResult("failed"));
        var firstRegistration = coordinator.Register("capture_audio", first);
        coordinator.Register("capture_audio", second);

        firstRegistration.Dispose();
        coordinator.Submit(new TraverseAppCommand("record"));
        Assert.Equal("failed", (await bridge.Terminal.Task.WaitAsync(Wait))["result_class"]!.GetValue<string>());

        Assert.Throws<ArgumentException>(() => coordinator.Register(" ", first));
        Assert.Throws<ArgumentNullException>(() => coordinator.Register("x", null!));
    }

    [Fact]
    public async Task RemovedAdapterFallsBackToTargetIncompatible()
    {
        var bridge = new FakeBridge { AppResponse = Pending() };
        var coordinator = Coordinator(bridge);
        coordinator.Register("capture_audio", (_, _) => Task.FromResult(new HostConnectorResult("succeeded"))).Dispose();
        coordinator.Submit(new TraverseAppCommand("record"));
        var terminal = await bridge.Terminal.Task.WaitAsync(Wait);
        Assert.Equal("target_incompatible", terminal["payload"]!["code"]!.GetValue<string>());
    }

    [Fact]
    public async Task SystemTimerFiresOnceAndCanBeCancelled()
    {
        var timer = new SystemTraverseTimer();
        var fired = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        timer.Schedule(TimeSpan.FromMilliseconds(-1), () => fired.SetResult());
        await fired.Task.WaitAsync(Wait);

        var cancelled = false;
        timer.Schedule(TimeSpan.FromMilliseconds(200), () => cancelled = true).Dispose();
        await Task.Delay(400);
        Assert.False(cancelled);
    }

    [Fact]
    public void HarnessAcceptsAppCommandsAndAdvertisesApi110()
    {
        Assert.Equal("1.1.0", Traverse.Embedder.TraverseEmbedder.ApiVersion);
        var harness = new InMemoryTraverseEmbedder();
        Assert.Throws<InvalidOperationException>(() => harness.Submit(new TraverseAppCommand("go")));
        harness.Initialize(new TraverseBundle("assets/traverse", "sha256:test"));
        Assert.Throws<ArgumentException>(() => harness.Submit(new TraverseAppCommand(" ")));

        var generated = harness.Submit(new TraverseAppCommand("go", "{\"k\":1}"));
        var pinned = harness.Submit(new TraverseAppCommand("go", SessionId: "sess-7"));

        Assert.Equal(new TraverseSubmissionResult("dotnet-session-1", "accepted"), generated);
        Assert.Equal("sess-7", pinned.SessionId);
        Assert.Equal(
            new TraverseRuntimeEvent(1, "app_command", "accepted", EventType: "state_changed",
                SessionId: "dotnet-session-1", Output: "{\"k\":1}"),
            harness.Subscribe().First());
    }
}
