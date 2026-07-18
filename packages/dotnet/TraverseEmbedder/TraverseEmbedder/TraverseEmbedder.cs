namespace Traverse.Embedder;

/// <summary>Public .NET surface for Traverse embedder-api/1.0.0.</summary>
public static class TraverseEmbedder
{
    public const string ApiVersion = "1.0.0";
}

public sealed record TraverseBundle(string RootPath, string RuntimeWasmDigest)
{
    public void Validate()
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(RootPath);
        ArgumentException.ThrowIfNullOrWhiteSpace(RuntimeWasmDigest);
    }
}

public sealed record TraverseSubmission(string TargetId, string InputJson)
{
    public void Validate() => ArgumentException.ThrowIfNullOrWhiteSpace(TargetId);
}

public sealed record TraverseSubmissionResult(string SessionId, string Status);

/// <summary>Traceability evidence published with a TraverseEmbedder package release.</summary>
public sealed record TraverseReleaseEvidence(
    string PackageVersion,
    string RuntimeWasmDigest,
    string ConformanceVersion,
    IReadOnlyList<string> SupportedHostVersions)
{
    public void Validate()
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(PackageVersion);
        ArgumentException.ThrowIfNullOrWhiteSpace(RuntimeWasmDigest);
        ArgumentException.ThrowIfNullOrWhiteSpace(ConformanceVersion);
        if (SupportedHostVersions is null || SupportedHostVersions.Count == 0 ||
            SupportedHostVersions.Any(string.IsNullOrWhiteSpace))
        {
            throw new ArgumentException("supported host versions are required", nameof(SupportedHostVersions));
        }
    }
}

/// <summary>Ordered runtime-shaped event exposed by the conformance harness.</summary>
public sealed record TraverseRuntimeEvent(
    int Sequence,
    string TargetId,
    string Status,
    string? InstanceId = null,
    string? EventType = null,
    string? SessionId = null,
    string? ErrorData = null,
    string? Output = null);

public sealed record TraverseCompatibleResult(string? InstanceId, string Status);

/// <summary>
/// Deterministic conformance test double. It never evaluates application
/// business logic and never starts a Traverse sidecar process.
/// </summary>
public sealed class InMemoryTraverseEmbedder
{
    private TraverseBundle? bundle;
    private int submissionSequence;
    private int compatibleSequence;
    private readonly List<TraverseRuntimeEvent> events = [];
    private readonly Dictionary<string, string> compatibleInstances = [];
    private string? targetOutput;

    public InMemoryTraverseEmbedder WithTargetOutput(string output)
    {
        targetOutput = output;
        return this;
    }

    public void Initialize(TraverseBundle value)
    {
        if (bundle is not null) throw new InvalidOperationException("embedder is already initialized");
        value.Validate();
        bundle = value;
    }

    public void Shutdown()
    {
        bundle = null;
        submissionSequence = 0;
        compatibleSequence = 0;
        events.Clear();
        compatibleInstances.Clear();
    }

    public TraverseSubmissionResult Submit(TraverseSubmission submission)
    {
        if (bundle is null) throw new InvalidOperationException("embedder is not initialized");
        submission.Validate();
        submissionSequence++;
        var result = new TraverseSubmissionResult($"dotnet-session-{submissionSequence}", "accepted");
        events.Add(new TraverseRuntimeEvent(submissionSequence, submission.TargetId, result.Status,
            EventType: targetOutput is null ? null : "capability_result",
            SessionId: targetOutput is null ? null : result.SessionId, Output: targetOutput));
        return result;
    }

    /// <summary>Returns ordered runtime-shaped events emitted after a sequence cursor.</summary>
    public IReadOnlyList<TraverseRuntimeEvent> Subscribe(int afterSequence = 0)
    {
        EnsureInitialized();
        return events.Where(@event => @event.Sequence > afterSequence).ToArray();
    }

    public TraverseCompatibleResult CompatibleStart(string capabilityId, string inputJson)
    {
        EnsureInitialized();
        ArgumentException.ThrowIfNullOrWhiteSpace(capabilityId);
        compatibleSequence++;
        var instanceId = $"dotnet-compatible-{compatibleSequence}";
        compatibleInstances[capabilityId] = instanceId;
        AppendCompatibleEvent(capabilityId, instanceId, "started");
        return new TraverseCompatibleResult(instanceId, "started");
    }

    public TraverseCompatibleResult CompatibleStop(string capabilityId, string? instanceId)
    {
        var resolvedInstanceId = CompatibleInstance(capabilityId, instanceId);
        compatibleInstances.Remove(capabilityId);
        AppendCompatibleEvent(capabilityId, resolvedInstanceId, "stopped");
        return new TraverseCompatibleResult(resolvedInstanceId, "stopped");
    }

    public TraverseCompatibleResult CompatibleKill(string capabilityId, string? instanceId)
    {
        var resolvedInstanceId = CompatibleInstance(capabilityId, instanceId);
        compatibleInstances.Remove(capabilityId);
        AppendCompatibleEvent(capabilityId, resolvedInstanceId, "killed");
        return new TraverseCompatibleResult(resolvedInstanceId, "killed");
    }

    private string CompatibleInstance(string capabilityId, string? instanceId)
    {
        EnsureInitialized();
        if (!compatibleInstances.TryGetValue(capabilityId, out var activeInstanceId) ||
            (instanceId is not null && instanceId != activeInstanceId))
        {
            throw new InvalidOperationException("compatible instance is not active");
        }
        return activeInstanceId;
    }

    private void AppendCompatibleEvent(string capabilityId, string instanceId, string status)
    {
        submissionSequence++;
        events.Add(new TraverseRuntimeEvent(submissionSequence, capabilityId, status, instanceId));
    }

    private void EnsureInitialized()
    {
        if (bundle is null) throw new InvalidOperationException("embedder is not initialized");
    }
}
