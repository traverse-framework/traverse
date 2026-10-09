# Android emulator tests for Kotlin exact-ref model execution (#1611)

The Kotlin unit tests (`testDebugUnitTest`) prove parity with a **host-JVM**
build of `traverse-android-host`. The `android-emulator-tests` CI job proves
the Android-specific path that the unit tests cannot reach:

- `System.loadLibrary("traverse_android_host")` from `jniLibs`;
- the cargo-ndk `x86_64` library the AAR ships, built with 16 KiB pages and a
  single exported JNI method, both checked by
  `scripts/build_android_host_ndk.sh`;
- a real Android runtime (API 34, `google_apis`, `x86_64`).

## What runs

`src/androidTest/.../ExactModelHostInstrumentedTest.kt`, through the Android
Test Orchestrator, one process per test:

| Test | Proves |
| --- | --- |
| `signedClassifierVectorIsByteIdentical`, `signedDigitsMlpVectorIsByteIdentical`, `onnxRunnerVectorIsByteIdenticalWithSimd` | The signed vectors byte-for-byte, including the `+simd128` ONNX runner on `wasmi` |
| `preparedV3VectorIsByteIdenticalFreshAndWarm` | Guest ABI v3 snapshot reuse (Decision 110), fresh and warm |
| `rightsConformanceSuiteMatchesEveryOtherEmbedder` | The shared 21-case Spec 138 rights suite (FR-041), with codes, reasons, details and evidence identical to every other embedder |
| `missingNativeLibraryFailsClosed` | A library that fails to load gives `model_unavailable` / `engine_unavailable` on every model call, and the rest of the embedder keeps working |

The fixtures are the repository's `fixtures/models`, bundled as test-APK
assets. The missing-library test points the loader at a path that does not
exist before anything touches `ExactModelNative`. That fails the load exactly
as a stripped `.so` would, and the orchestrator keeps that failure out of the
other tests' processes.

## Runtime and flakiness mitigations

- **Runtime:** about 10–15 minutes on a cache hit, most of it the cargo-ndk
  build and the emulator boot. A cold cache adds a few minutes to create the
  AVD snapshot. The job's timeout is 45 minutes.
- **Hardware acceleration:** a udev rule enables KVM on the `ubuntu-latest`
  runner. Without KVM the emulator is too slow to be reliable.
- **Snapshot cache:** `actions/cache` keeps the AVD and a boot snapshot,
  keyed by API level, ABI and target. The test step boots from it with
  `-no-snapshot-save`, so every run starts from the same clean state.
- **Quiet emulator:** headless, no audio, no camera, no boot animation,
  `swiftshader_indirect` GPU, and animations disabled for the test run.
- **Isolation:** the Android Test Orchestrator with `clearPackageData` runs
  each test in a fresh process, so no test depends on another's native state.
- **Diagnosis:** on failure the job uploads
  `traverse-embedder/build/reports/androidTests/`.

## Running it locally

You need a JDK 17+, the Android SDK with an `x86_64` system image, the NDK
(`ANDROID_NDK_HOME`), Rust 1.94.0 with the `aarch64-linux-android` and
`x86_64-linux-android` targets, and `cargo-ndk` 4.1.2.

```bash
bash scripts/build_android_host_ndk.sh packages/kotlin/TraverseEmbedder/traverse-embedder/build/jniLibs
```

Then start an `x86_64` emulator (API 28 or later) or connect a device, and
run:

```bash
cd packages/kotlin/TraverseEmbedder && gradle --no-daemon :traverse-embedder:connectedDebugAndroidTest
```

On an Apple-silicon Mac, use an `arm64-v8a` emulator image instead. The same
build step produces `arm64-v8a` too.
