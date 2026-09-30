//! #1590 harness: runs the spike guest on wasmi (fuel metered, as the Swift
//! host does) or wasmtime, over a directory of 144_000-sample f32le clips.
//! Usage: harness <wasmi|wasmtime> <guest.wasm> <clips_dir> <out_dir>
use std::time::{Duration, Instant};

const OUT_BYTES: i32 = 6522 * 4;

fn clips(dir: &str) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "f32le")).collect();
    v.sort();
    v
}

fn report(engine: &str, prepare: Duration, runs: &[Duration], peak_bytes: usize, fuel: Option<(u64, u64)>) {
    let mean = runs.iter().sum::<Duration>() / runs.len() as u32;
    let max = runs.iter().max().copied().unwrap_or_default();
    println!("engine={engine} clips={} prepare={prepare:?} mean_run={mean:?} max_run={max:?} peak_linear_memory_mib={:.1}{}",
        runs.len(), peak_bytes as f64 / 1_048_576.0,
        fuel.map(|(p, r)| format!(" prepare_fuel={p} mean_run_fuel={r}")).unwrap_or_default());
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (engine, wasm_path, clip_dir, out_dir) = (&a[1], &a[2], &a[3], &a[4]);
    let wasm = std::fs::read(wasm_path).unwrap();
    let paths = clips(clip_dir);
    match engine.as_str() {
        "wasmi" => {
            use wasmi::*;
            let mut config = Config::default();
            config.consume_fuel(true);
            config.wasm_simd(true);
            let engine = Engine::new(&config);
            let module = Module::new(&engine, &wasm[..]).unwrap();
            let mut store = Store::new(&engine, ());
            store.set_fuel(u64::MAX / 2).unwrap();
            let instance = Linker::new(&engine).instantiate_and_start(&mut store, &module).unwrap();
            let memory = instance.get_memory(&store, "memory").unwrap();
            let alloc = instance.get_typed_func::<i32, i32>(&store, "model_alloc").unwrap();
            let prepare = instance.get_typed_func::<(), i32>(&store, "model_prepare").unwrap();
            let execute = instance.get_typed_func::<(i32, i32, i32, i32), i32>(&store, "model_execute").unwrap();
            let fuel0 = store.get_fuel().unwrap();
            let t = Instant::now();
            assert_eq!(prepare.call(&mut store, ()).unwrap(), 0, "prepare failed");
            let prepare_time = t.elapsed();
            let prepare_fuel = fuel0 - store.get_fuel().unwrap();
            let mut runs = Vec::new();
            let mut fuel_total = 0;
            for p in &paths {
                let input = std::fs::read(p).unwrap();
                let in_ptr = alloc.call(&mut store, input.len() as i32).unwrap();
                let out_ptr = alloc.call(&mut store, OUT_BYTES).unwrap();
                memory.write(&mut store, in_ptr as usize, &input).unwrap();
                let before = store.get_fuel().unwrap();
                let t = Instant::now();
                let n = execute.call(&mut store, (in_ptr, input.len() as i32, out_ptr, OUT_BYTES)).unwrap();
                runs.push(t.elapsed());
                fuel_total += before - store.get_fuel().unwrap();
                assert_eq!(n, OUT_BYTES, "execute returned {n}");
                let mut out = vec![0_u8; n as usize];
                memory.read(&store, out_ptr as usize, &mut out).unwrap();
                let stem = p.file_stem().unwrap().to_str().unwrap();
                std::fs::write(format!("{out_dir}/{stem}.wasmi.f32le"), out).unwrap();
            }
            report("wasmi", prepare_time, &runs, memory.data_size(&store), Some((prepare_fuel, fuel_total / paths.len() as u64)));
        }
        _ => {
            use wasmtime::*;
            let engine = Engine::default();
            let module = Module::new(&engine, &wasm).unwrap();
            let mut store = Store::new(&engine, ());
            let instance = Linker::new(&engine).instantiate(&mut store, &module).unwrap();
            let memory = instance.get_memory(&mut store, "memory").unwrap();
            let alloc = instance.get_typed_func::<i32, i32>(&mut store, "model_alloc").unwrap();
            let prepare = instance.get_typed_func::<(), i32>(&mut store, "model_prepare").unwrap();
            let execute = instance.get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, "model_execute").unwrap();
            let t = Instant::now();
            assert_eq!(prepare.call(&mut store, ()).unwrap(), 0);
            let prepare_time = t.elapsed();
            let mut runs = Vec::new();
            for p in &paths {
                let input = std::fs::read(p).unwrap();
                let in_ptr = alloc.call(&mut store, input.len() as i32).unwrap();
                let out_ptr = alloc.call(&mut store, OUT_BYTES).unwrap();
                memory.write(&mut store, in_ptr as usize, &input).unwrap();
                let t = Instant::now();
                let n = execute.call(&mut store, (in_ptr, input.len() as i32, out_ptr, OUT_BYTES)).unwrap();
                runs.push(t.elapsed());
                assert_eq!(n, OUT_BYTES);
                let mut out = vec![0_u8; n as usize];
                memory.read(&store, out_ptr as usize, &mut out).unwrap();
                let stem = p.file_stem().unwrap().to_str().unwrap();
                std::fs::write(format!("{out_dir}/{stem}.wasmtime.f32le"), out).unwrap();
            }
            report("wasmtime", prepare_time, &runs, memory.data_size(&store), None);
        }
    }
}
