# macOS audio-analysis example

`TraverseAudioAnalysisExample` is the macOS counterpart to the browser
audio-analysis example. It loads the checked-in `audio-analysis` bundle,
verifies its `runtime.wasm` digest, registers `AppleAudioInputAdapters`, sends
an app command, and prints only runtime events. It does not implement a state
machine, own a sidecar, or expose audio paths or raw bytes.

Run from this package on macOS:

```bash
swift run TraverseAudioAnalysisExample
```

The host app needs `NSMicrophoneUsageDescription` before a production UI can
request microphone permission. The example intentionally uses the same
bounded, opaque-artifact adapter boundary as the public SDK.
