using System.Security.Cryptography;
using Wasmtime;

namespace Traverse.Embedder;

/// <summary>Fail-closed loader for runtime-wasm-bridge/1.1.0.</summary>
public sealed class WasmtimeRuntimeBridge : IDisposable
{
    public const int AbiVersion = 10_100;
    public const long DefaultMaximumArtifactBytes = 32L * 1024L * 1024L;
    public const long DefaultMaximumMemoryBytes = 32L * 1024L * 1024L;
    public const ulong DefaultFuelPerCall = 10_000_000;
    public static readonly TimeSpan DefaultMaximumCallDuration = TimeSpan.FromSeconds(30);

    private readonly Engine engine;
    private readonly Module module;
    private readonly Linker linker;
    private readonly Store store;
    private readonly ulong fuelPerCall;
    private readonly TimeSpan maximumCallDuration;
    private int executionGeneration;

    public string RuntimePath { get; }
    public string RuntimeWasmDigest { get; }
    internal Instance Instance { get; }

    public WasmtimeRuntimeBridge(
        TraverseBundle bundle,
        long maximumArtifactBytes = DefaultMaximumArtifactBytes,
        long maximumMemoryBytes = DefaultMaximumMemoryBytes,
        ulong fuelPerCall = DefaultFuelPerCall,
        TimeSpan? maximumCallDuration = null)
    {
        bundle.Validate();
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(maximumArtifactBytes);
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(maximumMemoryBytes);
        ArgumentOutOfRangeException.ThrowIfZero(fuelPerCall);
        this.maximumCallDuration = maximumCallDuration ?? DefaultMaximumCallDuration;
        if (this.maximumCallDuration <= TimeSpan.Zero)
        {
            throw new ArgumentOutOfRangeException(nameof(maximumCallDuration));
        }
        this.fuelPerCall = fuelPerCall;

        var bundleRoot = Path.GetFullPath(bundle.RootPath);
        var runtimePath = Path.GetFullPath(Path.Join(bundleRoot, "runtime", "runtime.wasm"));
        var rootPrefix = Path.EndsInDirectorySeparator(bundleRoot)
            ? bundleRoot
            : bundleRoot + Path.DirectorySeparatorChar;
        if (!runtimePath.StartsWith(rootPrefix, StringComparison.Ordinal))
        {
            throw new TraverseBundleException("runtime/runtime.wasm escapes the bundle root");
        }
        RuntimePath = runtimePath;
        if (!File.Exists(RuntimePath))
        {
            throw new TraverseBundleException("runtime/runtime.wasm is unavailable");
        }

        var file = new FileInfo(RuntimePath);
        if (file.Length > maximumArtifactBytes)
        {
            throw new TraverseBundleException("runtime/runtime.wasm exceeds the configured size limit");
        }

        var bytes = File.ReadAllBytes(RuntimePath);
        RuntimeWasmDigest = "sha256:" + Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant();
        if (!string.Equals(NormalizeDigest(bundle.RuntimeWasmDigest), RuntimeWasmDigest, StringComparison.Ordinal))
        {
            throw new TraverseBundleException("bundle_digest_mismatch");
        }

        using (var config = new Config())
        {
            config.WithFuelConsumption(true);
            config.WithEpochInterruption(true);
            engine = new Engine(config);
        }
        try
        {
            module = Module.FromBytes(engine, "traverse-runtime", bytes);
        }
        catch (WasmtimeException error)
        {
            engine.Dispose();
            throw new TraverseBundleException("runtime/runtime.wasm is not a valid core WebAssembly module", error);
        }

        if (module.Imports.Count != 0)
        {
            module.Dispose();
            engine.Dispose();
            throw new TraverseBundleException("runtime/runtime.wasm requires undeclared ambient imports");
        }

        linker = new Linker(engine);
        store = new Store(engine);
        store.SetLimits(memorySize: maximumMemoryBytes, tableElements: null, instances: 1, tables: 1, memories: 1);
        try
        {
            Instance = linker.Instantiate(store, module);
            ValidateExports(Instance);
        }
        catch (Exception error)
        {
            store.Dispose();
            linker.Dispose();
            module.Dispose();
            engine.Dispose();
            if (error is WasmtimeException)
            {
                throw new TraverseBundleException("bridge_resource_limit", error);
            }
            throw;
        }
    }

    private void ValidateExports(Instance instance)
    {
        if (instance.GetMemory("memory") is null)
        {
            throw new TraverseBundleException("runtime/runtime.wasm is missing required export memory");
        }

        RequireFunction(instance, "traverse_alloc", typeof(int), typeof(int));
        RequireFunction(instance, "traverse_dealloc", null, typeof(int), typeof(int));
        RequireFunction(instance, "traverse_init", typeof(int), typeof(int), typeof(int), typeof(int));
        RequireFunction(instance, "traverse_submit", typeof(int), typeof(int), typeof(int), typeof(int));
        RequireFunction(instance, "traverse_next_event", typeof(int), typeof(int));
        RequireFunction(instance, "traverse_cancel", typeof(int), typeof(int), typeof(int), typeof(int));
        RequireFunction(instance, "traverse_compatible_start", typeof(int), typeof(int), typeof(int), typeof(int));
        RequireFunction(instance, "traverse_compatible_stop", typeof(int), typeof(int), typeof(int), typeof(int));
        RequireFunction(instance, "traverse_compatible_kill", typeof(int), typeof(int), typeof(int), typeof(int));
        RequireFunction(instance, "traverse_shutdown", typeof(int), typeof(int));

        var version = instance.GetFunction<int>("traverse_bridge_abi_version");
        if (version is null || Execute(version) != AbiVersion)
        {
            throw new TraverseBundleException("bridge_version_mismatch");
        }
    }

    internal T Execute<T>(Func<T> action)
    {
        store.Fuel = fuelPerCall;
        store.SetEpochDeadline(1);
        var generation = Interlocked.Increment(ref executionGeneration);
        using var timer = new Timer(
            _ =>
            {
                if (Volatile.Read(ref executionGeneration) == generation)
                {
                    engine.IncrementEpoch();
                }
            },
            null,
            maximumCallDuration,
            Timeout.InfiniteTimeSpan);
        try
        {
            return action();
        }
        finally
        {
            Interlocked.Increment(ref executionGeneration);
        }
    }

    private static void RequireFunction(Instance instance, string name, Type? result, params Type[] parameters)
    {
        if (instance.GetFunction(name, result, parameters) is null)
        {
            throw new TraverseBundleException($"runtime/runtime.wasm is missing or has an invalid signature for {name}");
        }
    }

    private static string NormalizeDigest(string digest)
    {
        var normalized = digest.Trim().ToLowerInvariant();
        return normalized.StartsWith("sha256:", StringComparison.Ordinal) ? normalized : $"sha256:{normalized}";
    }

    public void Dispose()
    {
        store.Dispose();
        linker.Dispose();
        module.Dispose();
        engine.Dispose();
    }
}

public sealed class TraverseBundleException(string message, Exception? innerException = null)
    : ArgumentException(message, innerException);
