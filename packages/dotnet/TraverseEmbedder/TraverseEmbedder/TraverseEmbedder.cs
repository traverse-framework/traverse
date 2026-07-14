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

/// <summary>
/// Deterministic conformance test double. It never evaluates application
/// business logic and never starts a Traverse sidecar process.
/// </summary>
public sealed class InMemoryTraverseEmbedder
{
    private TraverseBundle? bundle;
    private int submissionSequence;

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
    }

    public TraverseSubmissionResult Submit(TraverseSubmission submission)
    {
        if (bundle is null) throw new InvalidOperationException("embedder is not initialized");
        submission.Validate();
        submissionSequence++;
        return new TraverseSubmissionResult($"dotnet-session-{submissionSequence}", "accepted");
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
