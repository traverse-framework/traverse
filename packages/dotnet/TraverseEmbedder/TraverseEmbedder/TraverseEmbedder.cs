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

/// <summary>Ordered runtime-shaped event exposed by the conformance harness.</summary>
public sealed record TraverseRuntimeEvent(int Sequence, string TargetId, string Status);

/// <summary>
/// Deterministic conformance test double. It never evaluates application
/// business logic and never starts a Traverse sidecar process.
/// </summary>
public sealed class InMemoryTraverseEmbedder
{
    private TraverseBundle? bundle;
    private int submissionSequence;
    private readonly List<TraverseRuntimeEvent> events = [];

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
        events.Clear();
    }

    public TraverseSubmissionResult Submit(TraverseSubmission submission)
    {
        if (bundle is null) throw new InvalidOperationException("embedder is not initialized");
        submission.Validate();
        submissionSequence++;
        var result = new TraverseSubmissionResult($"dotnet-session-{submissionSequence}", "accepted");
        events.Add(new TraverseRuntimeEvent(submissionSequence, submission.TargetId, result.Status));
        return result;
    }

    /// <summary>Returns ordered runtime-shaped events emitted after a sequence cursor.</summary>
    public IReadOnlyList<TraverseRuntimeEvent> Subscribe(int afterSequence = 0)
    {
        EnsureInitialized();
        return events.Where(@event => @event.Sequence > afterSequence).ToArray();
    }

    public TraverseSubmissionResult CompatibleStart(string capabilityId, string inputJson) =>
        Submit(new TraverseSubmission(capabilityId, inputJson));

    public void CompatibleStop(string capabilityId, string? instanceId) => EnsureInitialized();

    public void CompatibleKill(string capabilityId, string? instanceId) => EnsureInitialized();

    private void EnsureInitialized()
    {
        if (bundle is null) throw new InvalidOperationException("embedder is not initialized");
    }
}
