using System.Buffers.Binary;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json.Nodes;

namespace Traverse.Embedder;

/// <summary>
/// Spec 138 exact-ref model failure: stable <see cref="Code"/>, refining
/// <see cref="Reason"/>, and rights <see cref="Detail"/>.
/// </summary>
public sealed class ExactModelException(
    string code,
    string? reason,
    string message,
    ModelRightsDenialDetail? detail = null) : Exception(message)
{
    public string Code { get; } = code;

    public string? Reason { get; } = reason;

    public ModelRightsDenialDetail? Detail { get; } = detail;
}

/// <summary>Why a rights check failed (Spec 138 0.8.0); identity is null only before a package is known.</summary>
public sealed record ModelRightsDenialDetail(
    string? ModelId,
    string? Version,
    string? Digest,
    string Field,
    string Expected,
    string Actual,
    string? EffectiveUsage);

/// <summary>Signed provenance of a derivative package (<c>rights.derivation</c>).</summary>
public sealed record ModelDerivation(
    string Kind,
    string SourceDigest,
    string SourceLicenseId,
    string SourceCommercialUse,
    string SourceUrl);

/// <summary>Signed rights of a registered package, for host/UI display.</summary>
public sealed record ModelRights(
    string LicenseId,
    string Attribution,
    string Redistribution,
    string CommercialUse,
    string SourceUrl,
    ModelDerivation? Derivation);

/// <summary>Verified rights record (Spec 138 FR-040). <c>Status</c> is <c>revoked</c> only on a host query.</summary>
public sealed record ModelRightsRecord(
    string ModelId,
    string Version,
    string Digest,
    ModelRights Rights,
    string Status,
    string? StatusReason,
    string EffectiveUsage);

/// <summary>Host-owned package status: <c>deprecated</c> runs but is flagged; <c>revoked</c> fails closed.</summary>
public sealed record PackageStatusEntry(string Status, string Reason);

/// <summary>Application <c>exact_model_dependencies</c> pin (Spec 138).</summary>
public sealed record ExactModelPin(
    string ModelId,
    string Version,
    string Digest,
    string LicenseId,
    string CommercialUse,
    bool OfflineAllowed = true,
    string Target = "wasm-cpu",
    string? KeyId = null)
{
    internal JsonObject ToJson()
    {
        var pin = new JsonObject
        {
            ["model_id"] = ModelId,
            ["version"] = Version,
            ["digest"] = Digest,
            ["offline_allowed"] = OfflineAllowed,
            ["target"] = Target,
            ["rights"] = new JsonObject { ["license_id"] = LicenseId, ["commercial_use"] = CommercialUse },
        };
        if (KeyId is not null)
        {
            pin["key_id"] = KeyId;
        }

        return pin;
    }

    /// <summary>Parse a pin from its app-manifest JSON.</summary>
    public static ExactModelPin FromJson(string json)
    {
        var value = JsonNode.Parse(json) as JsonObject
            ?? throw new ExactModelException("invalid_input", null, "pin is not a JSON object");
        var rights = value.Obj("rights");
        return new ExactModelPin(
            value.Str("model_id"),
            value.Str("version"),
            value.Str("digest"),
            rights.Str("license_id"),
            rights.Str("commercial_use"),
            value["offline_allowed"]?.GetValue<bool>() ?? true,
            value.OptStr("target") ?? "wasm-cpu",
            value.OptStr("key_id"));
    }
}

/// <summary>
/// Host ceilings a package's declared limits must fit within (Decision 104).
/// The defaults are the native desktop defaults (Decision 111).
/// </summary>
public sealed record ExactModelHostLimits(
    ulong MaxPackageBytes = 256UL * 1024 * 1024,
    ulong MaxMemoryBytes = 1024UL * 1024 * 1024,
    ulong MaxFuel = 50_000_000_000UL,
    ulong MaxSnapshotBytes = 512UL * 1024 * 1024);

/// <summary>Typed <c>model.execute</c> result with identity, placement, usage, and rights evidence.</summary>
public sealed record ExactModelExecution(
    string OutputRef,
    string Placement,
    string Target,
    string ModelId,
    string Version,
    string Digest,
    string DataClassification,
    int InputBytes,
    int OutputBytes,
    double DurationMs,
    ModelRightsRecord? ModelEvidence);

/// <summary>
/// The P/Invoke boundary (Decision 111, ADR-0081): one framed call into
/// <c>traverse-dotnet-host</c>, plus the matching free. .NET's default probing
/// finds the library next to the app or under <c>runtimes/&lt;rid&gt;/native/</c>.
/// </summary>
internal static unsafe partial class ExactModelNative
{
    private const string Library = "traverse_dotnet_host";

    [LibraryImport(Library, EntryPoint = "traverse_dotnet_host_model_call")]
    private static partial byte* ModelCall(ulong handle, byte* request, nuint requestLength, out nuint responseLength);

    [LibraryImport(Library, EntryPoint = "traverse_dotnet_host_free")]
    private static partial void Free(byte* response, nuint length);

    /// <summary>Send one request frame; returns the response frame.</summary>
    internal static byte[] Call(ulong handle, byte[] request)
    {
        fixed (byte* bytes = request)
        {
            var response = ModelCall(handle, bytes, (nuint)request.Length, out var length);
            try
            {
                return new ReadOnlySpan<byte>(response, checked((int)length)).ToArray();
            }
            finally
            {
                Free(response, length);
            }
        }
    }
}

/// <summary>
/// .NET Spec 138 exact-ref model host (Decision 111). Verification, rights,
/// policy, and the <c>wasmi</c> guest run in the shared Rust
/// <c>traverse-model-host-frame</c> behind the P/Invoke shim; trust stays
/// host-owned. If the native library cannot load, every call fails closed
/// with <c>model_unavailable</c> / <c>engine_unavailable</c>.
/// </summary>
public sealed class ExactModelHost : IDisposable
{
    private readonly Func<ulong, byte[], byte[]> transport;
    private readonly ulong handle;
    private int disposed;

    /// <param name="pins">The app manifest <c>exact_model_dependencies</c>.</param>
    /// <param name="trustedPublicKeysHex">Host-owned trusted Ed25519 public keys (hex, raw 32 bytes).</param>
    /// <param name="modelUsage">
    /// The app manifest <c>model_usage</c> (<c>commercial</c> | <c>non_commercial</c>); registration
    /// fails closed with <c>usage_undeclared</c> when it is null.
    /// </param>
    /// <param name="hostRequiresCommercial">Host tightening; a host can never relax an app's usage.</param>
    /// <param name="limits">Host ceilings; desktop defaults when omitted.</param>
    public ExactModelHost(
        IReadOnlyList<ExactModelPin> pins,
        IReadOnlyList<string> trustedPublicKeysHex,
        string? modelUsage,
        bool hostRequiresCommercial = false,
        ExactModelHostLimits? limits = null)
        : this(pins, trustedPublicKeysHex, modelUsage, hostRequiresCommercial, limits, ExactModelNative.Call)
    {
    }

    internal ExactModelHost(
        IReadOnlyList<ExactModelPin> pins,
        IReadOnlyList<string> trustedPublicKeysHex,
        string? modelUsage,
        bool hostRequiresCommercial,
        ExactModelHostLimits? limits,
        Func<ulong, byte[], byte[]> transport)
    {
        this.transport = transport;
        limits ??= new ExactModelHostLimits();
        var header = new JsonObject
        {
            ["op"] = "create",
            ["pins"] = new JsonArray(pins.Select(pin => (JsonNode)pin.ToJson()).ToArray()),
            ["trusted_public_keys_hex"] = new JsonArray(trustedPublicKeysHex.Select(key => (JsonNode)key).ToArray()),
            ["limits"] = new JsonObject
            {
                ["max_package_bytes"] = limits.MaxPackageBytes,
                ["max_memory_bytes"] = limits.MaxMemoryBytes,
                ["max_fuel"] = limits.MaxFuel,
                ["max_snapshot_bytes"] = limits.MaxSnapshotBytes,
            },
            ["host_requires_commercial"] = hostRequiresCommercial,
        };
        if (modelUsage is not null)
        {
            header["model_usage"] = modelUsage;
        }

        handle = Call(0, header).Header["handle"]?.GetValue<ulong>()
            ?? throw new ExactModelException("unavailable", null, "model host was not created");
    }

    /// <summary>Verify and admit a signed package; returns the pin digest it is cached under.</summary>
    public Task<string> RegisterPackageAsync(byte[] manifest, byte[] wasm, byte[] signature) =>
        Task.Run(() => Call(
            handle,
            new JsonObject { ["op"] = "register" },
            ("manifest", manifest),
            ("wasm", wasm),
            ("signature", signature)).Header.OptStr("digest")
            ?? throw new ExactModelException("model_incompatible", null, "registration returned no digest"));

    /// <summary>Stage input bytes; returns a single-consume opaque <c>input_ref</c>.</summary>
    public string StageModelInput(byte[] bytes, int maxBytes) =>
        Call(handle, new JsonObject { ["op"] = "stage_input", ["max_bytes"] = maxBytes }, ("input", bytes))
            .Header.OptStr("input_ref")
        ?? throw new ExactModelException("invalid_input", null, "staging returned no input_ref");

    /// <summary>Read output bytes by <c>output_ref</c> (size-capped).</summary>
    public byte[] ReadModelOutput(string outputRef, int maxBytes) =>
        Call(handle, new JsonObject { ["op"] = "read_output", ["output_ref"] = outputRef, ["max_bytes"] = maxBytes })
            .Segment("output") ?? [];

    /// <summary>Signed rights of a registered package, or null when unknown.</summary>
    public ModelRights? ModelRights(string digest) =>
        Call(handle, new JsonObject { ["op"] = "rights", ["digest"] = digest }).Header["rights"] is JsonObject rights
            ? ParseRights(rights)
            : null;

    /// <summary>Verified rights record (rights, status, effective usage), or null when unknown or undeclared.</summary>
    public ModelRightsRecord? ModelRightsRecord(string digest) =>
        Call(handle, new JsonObject { ["op"] = "rights_record", ["digest"] = digest }).Header["record"] is JsonObject record
            ? ParseRecord(record)
            : null;

    /// <summary>Replace the host-owned package status map; a revocation blocks the next call.</summary>
    public void SetPackageStatus(IReadOnlyDictionary<string, PackageStatusEntry> entries)
    {
        var map = new JsonObject();
        foreach (var (digest, entry) in entries)
        {
            map[digest] = new JsonObject { ["status"] = entry.Status, ["reason"] = entry.Reason };
        }

        Call(handle, new JsonObject { ["op"] = "set_package_status", ["entries"] = map });
    }

    /// <summary>Drop a staged input or output ref.</summary>
    public void DropRef(string reference) =>
        Call(handle, new JsonObject { ["op"] = "drop_ref", ["ref"] = reference });

    /// <summary>
    /// Execute a pinned model. Cancelling <paramref name="cancellationToken"/> interrupts the
    /// running inference mid-run (<c>cancelled</c>); <paramref name="timeoutMs"/> bounds it (<c>timeout</c>).
    /// </summary>
    public Task<ExactModelExecution> ExecuteAsync(
        string modelId,
        string version,
        string digest,
        string inputRef,
        string policyRef,
        string dataClassification,
        string inputSchemaRef,
        string inputSchemaVersion,
        int maxOutputBytes,
        IReadOnlyList<string> allowedClassifications,
        int? timeoutMs = null,
        CancellationToken cancellationToken = default)
    {
        var payload = new JsonObject
        {
            ["model_ref"] = new JsonObject { ["model_id"] = modelId, ["version"] = version, ["digest"] = digest },
            ["input_ref"] = inputRef,
            ["policy_ref"] = policyRef,
            ["data_classification"] = dataClassification,
            ["input_schema_ref"] = inputSchemaRef,
            ["input_schema_version"] = inputSchemaVersion,
            ["max_output_bytes"] = maxOutputBytes,
        };
        if (timeoutMs is not null)
        {
            payload["timeout_ms"] = timeoutMs;
        }

        return ExecuteAsync(payload, allowedClassifications, cancellationToken);
    }

    /// <summary>Execute a raw Spec 137 <c>model.execute</c> payload (used by app-command routing).</summary>
    public async Task<ExactModelExecution> ExecuteAsync(
        JsonObject payload,
        IReadOnlyList<string> allowedClassifications,
        CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested)
        {
            throw new ExactModelException("cancelled", null, "model.execute cancelled before invoke");
        }

        var executionId = Guid.NewGuid().ToString();
        var header = new JsonObject
        {
            ["op"] = "execute",
            ["execution_id"] = executionId,
            ["allowed_classifications"] = new JsonArray(allowedClassifications.Select(c => (JsonNode)c).ToArray()),
            ["payload"] = payload.DeepClone(),
        };
        var running = Task.Run(() => Call(handle, header), CancellationToken.None);
        // Flip the native cancel flag for exactly this execution; the blocked call returns `cancelled`.
        using (cancellationToken.Register(() => SendCancel(executionId)))
        {
            var response = await running.ConfigureAwait(false);
            var trace = response.Header["trace"] as JsonObject;
            var usage = trace?["usage"] as JsonObject;
            var model = response.Header["model_ref"] as JsonObject;
            return new ExactModelExecution(
                response.Header.OptStr("output_ref") ?? string.Empty,
                response.Header.OptStr("placement") ?? string.Empty,
                response.Header.OptStr("target") ?? string.Empty,
                model?.OptStr("model_id") ?? string.Empty,
                model?.OptStr("version") ?? string.Empty,
                model?.OptStr("digest") ?? string.Empty,
                trace?.OptStr("data_classification") ?? string.Empty,
                usage?["input_bytes"]?.GetValue<int>() ?? 0,
                usage?["output_bytes"]?.GetValue<int>() ?? 0,
                usage?["duration_ms"]?.GetValue<double>() ?? 0,
                response.Header["model_evidence"] is JsonObject evidence ? ParseRecord(evidence) : null);
        }
    }

    private void SendCancel(string executionId)
    {
        try
        {
            Call(handle, new JsonObject { ["op"] = "cancel", ["execution_id"] = executionId });
        }
        catch (ExactModelException)
        {
            // The host is gone; the running call fails on its own.
        }
    }

    /// <summary>Register this host as the adapter for a manifest command routed to <c>model.execute</c>.</summary>
    public IDisposable Install(RuntimeTraverseEmbedder on, string command) =>
        on.RegisterHostConnectorAdapter(command, ModelExecuteAdapter);

    /// <summary>
    /// The host-connector adapter <see cref="Install"/> registers. The command payload is the Spec 137
    /// <c>model.execute</c> payload plus <c>allowed_classifications</c>. Success carries
    /// <c>{output_ref, placement, model_id, version, digest, model_evidence}</c>; failures carry
    /// <c>reason</c> and the rights <c>detail</c>.
    /// </summary>
    public async Task<HostConnectorResult> ModelExecuteAdapter(HostConnectorRequest request, CancellationToken cancellationToken)
    {
        JsonObject? payload;
        try
        {
            payload = JsonNode.Parse(request.PayloadJson) as JsonObject;
        }
        catch (System.Text.Json.JsonException)
        {
            payload = null;
        }

        if (payload is null)
        {
            return Failed("invalid_input", null, "model.execute payload is not a JSON object", null);
        }

        var allowed = (payload["allowed_classifications"] as JsonArray)?
            .Select(node => node?.GetValue<string>())
            .OfType<string>()
            .ToList() ?? [];
        payload.Remove("allowed_classifications");
        try
        {
            var result = await ExecuteAsync(payload, allowed, cancellationToken).ConfigureAwait(false);
            var raw = new JsonObject
            {
                ["output_ref"] = result.OutputRef,
                ["placement"] = result.Placement,
                ["model_id"] = result.ModelId,
                ["version"] = result.Version,
                ["digest"] = result.Digest,
            };
            if (result.ModelEvidence is not null)
            {
                raw["model_evidence"] = RecordJson(result.ModelEvidence);
            }

            return new HostConnectorResult("succeeded", raw.ToJsonString());
        }
        catch (ExactModelException error)
        {
            var resultClass = error.Code is "cancelled" or "timeout" ? error.Code : "failed";
            return Failed(error.Code, error.Reason, error.Message, error.Detail, resultClass);
        }
    }

    public void Dispose()
    {
        if (Interlocked.Exchange(ref disposed, 1) == 0)
        {
            try
            {
                Call(handle, new JsonObject { ["op"] = "destroy" });
            }
            catch (ExactModelException)
            {
                // Already gone, or the engine never loaded: nothing to release.
            }
        }
    }

    /// <summary>The rights record as the shared conformance JSON (Spec 138 FR-040).</summary>
    public static JsonObject RecordJson(ModelRightsRecord record)
    {
        var rights = new JsonObject
        {
            ["license_id"] = record.Rights.LicenseId,
            ["attribution"] = record.Rights.Attribution,
            ["redistribution"] = record.Rights.Redistribution,
            ["commercial_use"] = record.Rights.CommercialUse,
            ["source_url"] = record.Rights.SourceUrl,
        };
        if (record.Rights.Derivation is { } derivation)
        {
            rights["derivation"] = new JsonObject
            {
                ["kind"] = derivation.Kind,
                ["source_digest"] = derivation.SourceDigest,
                ["source_license_id"] = derivation.SourceLicenseId,
                ["source_commercial_use"] = derivation.SourceCommercialUse,
                ["source_url"] = derivation.SourceUrl,
            };
        }

        var json = new JsonObject
        {
            ["model_id"] = record.ModelId,
            ["version"] = record.Version,
            ["digest"] = record.Digest,
            ["rights"] = rights,
            ["status"] = record.Status,
        };
        if (record.StatusReason is not null)
        {
            json["status_reason"] = record.StatusReason;
        }

        json["effective_usage"] = record.EffectiveUsage;
        return json;
    }

    internal sealed class Response(JsonObject header, byte[] payload)
    {
        public JsonObject Header { get; } = header;

        public byte[]? Segment(string name)
        {
            if ((Header["segments"] as JsonObject)?[name] is not JsonArray { Count: 2 } range)
            {
                return null;
            }

            var start = range[0]?.GetValue<int>() ?? -1;
            var length = range[1]?.GetValue<int>() ?? -1;
            return start < 0 || length < 0 || start + length > payload.Length
                ? null
                : payload.AsSpan(start, length).ToArray();
        }
    }

    internal static byte[] Encode(JsonObject header, params (string Name, byte[] Bytes)[] segments)
    {
        var full = (JsonObject)header.DeepClone();
        var body = new List<byte>();
        if (segments.Length > 0)
        {
            var offsets = new JsonObject();
            foreach (var (name, bytes) in segments)
            {
                offsets[name] = new JsonArray(body.Count, bytes.Length);
                body.AddRange(bytes);
            }

            full["segments"] = offsets;
        }

        var headerBytes = Encoding.UTF8.GetBytes(full.ToJsonString());
        var frame = new byte[4 + headerBytes.Length + body.Count];
        BinaryPrimitives.WriteUInt32LittleEndian(frame, (uint)headerBytes.Length);
        headerBytes.CopyTo(frame, 4);
        body.CopyTo(frame, 4 + headerBytes.Length);
        return frame;
    }

    internal static Response Decode(byte[] frame)
    {
        if (frame.Length < 4)
        {
            throw InvalidResponse();
        }

        var length = BinaryPrimitives.ReadUInt32LittleEndian(frame);
        if (length > (uint)(frame.Length - 4))
        {
            throw InvalidResponse();
        }

        JsonObject? header;
        try
        {
            header = JsonNode.Parse(frame.AsSpan(4, (int)length)) as JsonObject;
        }
        catch (System.Text.Json.JsonException)
        {
            header = null;
        }

        return new Response(header ?? throw InvalidResponse(), frame[(4 + (int)length)..]);
    }

    private static ExactModelException InvalidResponse() =>
        new("unavailable", null, "malformed model host response");

    private Response Call(ulong target, JsonObject header, params (string Name, byte[] Bytes)[] segments)
    {
        byte[] raw;
        try
        {
            raw = transport(target, Encode(header, segments));
        }
        catch (Exception error) when (error is DllNotFoundException or EntryPointNotFoundException or BadImageFormatException)
        {
            // Fail closed when the native engine cannot load (Spec 138 FR-047, Decision 111).
            throw new ExactModelException(
                "model_unavailable",
                "engine_unavailable",
                $"the native model engine (traverse_dotnet_host) could not be loaded: {error.Message}");
        }

        var response = Decode(raw);
        if (response.Header["ok"]?.GetValue<bool>() == false)
        {
            var error = response.Header["error"] as JsonObject;
            throw new ExactModelException(
                error?.OptStr("code") ?? "unavailable",
                error?.OptStr("reason"),
                error?.OptStr("message") ?? string.Empty,
                error?["detail"] is JsonObject detail ? ParseDenial(detail) : null);
        }

        return response;
    }

    private static HostConnectorResult Failed(
        string code,
        string? reason,
        string message,
        ModelRightsDenialDetail? detail,
        string resultClass = "failed")
    {
        var body = new JsonObject { ["error_code"] = code, ["message"] = message };
        if (reason is not null)
        {
            body["reason"] = reason;
        }

        if (detail is not null)
        {
            var json = new JsonObject();
            if (detail.ModelId is not null) json["model_id"] = detail.ModelId;
            if (detail.Version is not null) json["version"] = detail.Version;
            if (detail.Digest is not null) json["digest"] = detail.Digest;
            json["field"] = detail.Field;
            json["expected"] = detail.Expected;
            json["actual"] = detail.Actual;
            if (detail.EffectiveUsage is not null) json["effective_usage"] = detail.EffectiveUsage;
            body["detail"] = json;
        }

        return new HostConnectorResult(resultClass, body.ToJsonString());
    }

    private static ModelRights ParseRights(JsonObject value) => new(
        value.Str("license_id"),
        value.Str("attribution"),
        value.Str("redistribution"),
        value.Str("commercial_use"),
        value.Str("source_url"),
        value["derivation"] is JsonObject derivation
            ? new ModelDerivation(
                derivation.Str("kind"),
                derivation.Str("source_digest"),
                derivation.Str("source_license_id"),
                derivation.Str("source_commercial_use"),
                derivation.Str("source_url"))
            : null);

    private static ModelRightsRecord ParseRecord(JsonObject value) => new(
        value.Str("model_id"),
        value.Str("version"),
        value.Str("digest"),
        ParseRights(value.Obj("rights")),
        value.Str("status"),
        value.OptStr("status_reason"),
        value.Str("effective_usage"));

    private static ModelRightsDenialDetail ParseDenial(JsonObject value) => new(
        value.OptStr("model_id"),
        value.OptStr("version"),
        value.OptStr("digest"),
        value.Str("field"),
        value.Str("expected"),
        value.Str("actual"),
        value.OptStr("effective_usage"));
}

internal static class ExactModelJson
{
    internal static string? OptStr(this JsonObject value, string name) =>
        value[name] is JsonValue node && node.TryGetValue<string>(out var text) ? text : null;

    internal static string Str(this JsonObject value, string name) =>
        value.OptStr(name) ?? throw new ExactModelException("unavailable", null, $"model host response missing {name}");

    internal static JsonObject Obj(this JsonObject value, string name) =>
        value[name] as JsonObject ?? throw new ExactModelException("unavailable", null, $"model host response missing {name}");
}
