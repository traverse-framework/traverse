using System.Buffers.Binary;
using Wasmtime;

namespace Traverse.Embedder;

/// <summary>Serialized UTF-8 JSON client for the governed runtime-WASM bridge.</summary>
public sealed class WasmtimeBridgeClient
{
    public const int DefaultMaximumOutputBytes = 1024 * 1024;
    private const int DescriptorBytes = 8;

    private readonly object synchronization = new();
    private readonly Instance instance;
    private readonly Memory memory;
    private readonly Func<int, int> allocate;
    private readonly Action<int, int> deallocate;
    private readonly int maximumOutputBytes;

    public WasmtimeBridgeClient(
        WasmtimeRuntimeBridge bridge,
        int maximumOutputBytes = DefaultMaximumOutputBytes)
    {
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(maximumOutputBytes);
        instance = bridge.Instance;
        memory = instance.GetMemory("memory")
            ?? throw new TraverseBridgeException(-3, "bridge_invalid_descriptor");
        allocate = instance.GetFunction<int, int>("traverse_alloc")
            ?? throw new TraverseBridgeException(-5, "bridge allocation export is unavailable");
        deallocate = instance.GetAction<int, int>("traverse_dealloc")
            ?? throw new TraverseBridgeException(-5, "bridge deallocation export is unavailable");
        this.maximumOutputBytes = maximumOutputBytes;
    }

    public byte[] Initialize(ReadOnlySpan<byte> configJson)
    {
        var input = configJson.ToArray();
        return Serialized(() => InvokeWithInput("traverse_init", input));
    }

    public byte[] Submit(ReadOnlySpan<byte> requestJson)
    {
        var input = requestJson.ToArray();
        return Serialized(() => InvokeWithInput("traverse_submit", input));
    }

    public byte[] Cancel(ReadOnlySpan<byte> requestJson)
    {
        var input = requestJson.ToArray();
        return Serialized(() => InvokeWithInput("traverse_cancel", input));
    }

    public byte[] CompatibleStart(ReadOnlySpan<byte> requestJson)
    {
        var input = requestJson.ToArray();
        return Serialized(() => InvokeWithInput("traverse_compatible_start", input));
    }

    public byte[] CompatibleStop(ReadOnlySpan<byte> requestJson)
    {
        var input = requestJson.ToArray();
        return Serialized(() => InvokeWithInput("traverse_compatible_stop", input));
    }

    public byte[] CompatibleKill(ReadOnlySpan<byte> requestJson)
    {
        var input = requestJson.ToArray();
        return Serialized(() => InvokeWithInput("traverse_compatible_kill", input));
    }

    public byte[]? NextEvent() => Serialized(() =>
    {
        var descriptor = Allocate(DescriptorBytes);
        try
        {
            var function = instance.GetFunction<int, int>("traverse_next_event")
                ?? throw new TraverseBridgeException(-5, "bridge event export is unavailable");
            var status = function(descriptor);
            return status == 0 ? null : ReadResult(status, descriptor);
        }
        finally
        {
            deallocate(descriptor, DescriptorBytes);
        }
    });

    public byte[] Shutdown() => Serialized(() =>
    {
        var descriptor = Allocate(DescriptorBytes);
        try
        {
            var function = instance.GetFunction<int, int>("traverse_shutdown")
                ?? throw new TraverseBridgeException(-5, "bridge shutdown export is unavailable");
            return ReadResult(function(descriptor), descriptor);
        }
        finally
        {
            deallocate(descriptor, DescriptorBytes);
        }
    });

    private byte[] InvokeWithInput(string export, ReadOnlySpan<byte> input)
    {
        var inputPointer = Allocate(input.Length);
        var descriptor = Allocate(DescriptorBytes);
        try
        {
            input.CopyTo(memory.GetSpan(inputPointer, input.Length));
            var function = instance.GetFunction<int, int, int, int>(export)
                ?? throw new TraverseBridgeException(-5, $"bridge export {export} is unavailable");
            return ReadResult(function(inputPointer, input.Length, descriptor), descriptor);
        }
        finally
        {
            deallocate(descriptor, DescriptorBytes);
            deallocate(inputPointer, input.Length);
        }
    }

    private int Allocate(int length)
    {
        var pointer = allocate(length);
        if (pointer < 0) throw new TraverseBridgeException(-4, "bridge allocation failed");
        return pointer;
    }

    private byte[] ReadResult(int status, int descriptor)
    {
        var rawDescriptor = memory.GetSpan(descriptor, DescriptorBytes);
        var pointer = BinaryPrimitives.ReadInt32LittleEndian(rawDescriptor[..4]);
        var length = BinaryPrimitives.ReadInt32LittleEndian(rawDescriptor[4..]);
        if (pointer < 0 || length < 0 || length > maximumOutputBytes)
        {
            throw new TraverseBridgeException(-3, "bridge_invalid_descriptor");
        }

        var output = memory.GetSpan(pointer, length).ToArray();
        if (status < 0)
        {
            throw new TraverseBridgeException(status, System.Text.Encoding.UTF8.GetString(output));
        }
        return output;
    }

    private T Serialized<T>(Func<T> operation)
    {
        lock (synchronization) return operation();
    }
}

public sealed class TraverseBridgeException(int status, string message) : InvalidOperationException(message)
{
    public int Status { get; } = status;
}
