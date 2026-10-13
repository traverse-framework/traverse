"""Decision 112 spike: Core ML (Mac) vs the tract int8 reference, Decision 106 bar."""
import os, sys, time
import numpy as np
import coremltools as ct

clips = sorted(f for f in os.listdir("clips") if f.endswith(".f32le"))
def sig(v): return 1 / (1 + np.exp(-v.astype(np.float64)))
def load(path, shape=None):
    a = np.array(np.frombuffer(open(path, "rb").read(), np.float32))
    return a.reshape(shape) if shape else a
refs = {c: load(f"ref/{c[:-6]}.tract.f32le") for c in clips}
for prec in ("fp16", "fp32"):
    for cu_name, cu in (("ALL", ct.ComputeUnit.ALL), ("CPU_ONLY", ct.ComputeUnit.CPU_ONLY)):
        m = ct.models.MLModel(f"birdnet-{prec}.mlpackage", compute_units=cu)
        m.predict({"input": np.zeros((1, 144000), np.float32)})
        worst, top5_ok, times = 0.0, 0, []
        for c in clips:
            x = load(f"clips/{c}", (1, 144000))
            t = time.perf_counter()
            y = np.array(m.predict({"input": x})["output"]).reshape(-1)
            times.append(time.perf_counter() - t)
            r = refs[c]
            worst = max(worst, float(np.max(np.abs(sig(y) - sig(r)))))
            top5_ok += int(list(np.argsort(-r)[:5]) == list(np.argsort(-y)[:5]))
        print(f"{prec} {cu_name:8s} worst|dsig|={worst:.2e} top5={top5_ok}/{len(clips)} "
              f"mean={1000*np.mean(times):.1f}ms p95={1000*np.percentile(times, 95):.1f}ms")
