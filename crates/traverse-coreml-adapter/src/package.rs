//! Platform-neutral, safe helpers for the Core ML adapter: unpacking a
//! canonical `mlpackage-zip` (Spec 138 0.13.3), compute-unit names, and
//! moving `f32` tensors in and out of strided Core ML buffers.

use std::fs;
use std::path::{Component, Path};

/// Core ML compute units a host may select (Decision 112 rule 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComputeUnits {
    /// Core ML picks among CPU, GPU and Neural Engine (the default).
    #[default]
    All,
    /// CPU only.
    CpuOnly,
    /// CPU and GPU.
    CpuAndGpu,
    /// CPU and Neural Engine.
    CpuAndNeuralEngine,
}

impl ComputeUnits {
    /// Parse a host configuration value (`all`, `cpu_only`, `cpu_and_gpu`,
    /// `cpu_and_neural_engine`).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "all" => Some(Self::All),
            "cpu_only" => Some(Self::CpuOnly),
            "cpu_and_gpu" => Some(Self::CpuAndGpu),
            "cpu_and_neural_engine" => Some(Self::CpuAndNeuralEngine),
            _ => None,
        }
    }

    /// The name recorded in `placement_evidence.compute_units` and in the
    /// self-check key (FR-061, FR-062).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::CpuOnly => "cpu_only",
            Self::CpuAndGpu => "cpu_and_gpu",
            Self::CpuAndNeuralEngine => "cpu_and_neural_engine",
        }
    }
}

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const LOCAL_HEADER_LEN: usize = 30;
const DATA_DESCRIPTOR_FLAG: u16 = 0x0008;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// A relative entry path with only normal components, so unpacking can
/// never write outside the destination.
fn safe_entry_path(name: &str) -> bool {
    !name.contains(['\\', '\0'])
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(name)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

/// The `(path, contents)` entries of a canonical `mlpackage-zip`: stored
/// (uncompressed) entries with no data descriptors, each a safe relative
/// path, followed by the central directory. Anything else is rejected; the
/// variant digest has already been verified, so this guards the format, not
/// integrity.
///
/// # Errors
///
/// Returns a description of the first malformed or unsupported entry.
pub fn mlpackage_entries(bytes: &[u8]) -> Result<Vec<(&str, &[u8])>, String> {
    let mut entries = Vec::new();
    let mut at = 0;
    loop {
        match u32_at(bytes, at) {
            Some(LOCAL_HEADER) => {}
            Some(CENTRAL_HEADER) if !entries.is_empty() => return Ok(entries),
            _ => return Err(format!("no zip entry header at offset {at}")),
        }
        let field = |offset: usize| u16_at(bytes, at + offset);
        let wide = |offset: usize| u32_at(bytes, at + offset);
        let (Some(flags), Some(method), Some(stored), Some(size), Some(name_len), Some(extra_len)) =
            (field(6), field(8), wide(18), wide(22), field(26), field(28))
        else {
            return Err("truncated zip entry header".to_string());
        };
        if method != 0 || flags & DATA_DESCRIPTOR_FLAG != 0 || stored != size {
            return Err(
                "mlpackage-zip entries must be stored without data descriptors".to_string(),
            );
        }
        let name_start = at + LOCAL_HEADER_LEN;
        let data_start = name_start + usize::from(name_len) + usize::from(extra_len);
        let data_end = data_start
            .checked_add(usize::try_from(size).map_err(|_| "zip entry too large".to_string())?)
            .ok_or_else(|| "zip entry too large".to_string())?;
        let name = bytes
            .get(name_start..name_start + usize::from(name_len))
            .and_then(|name| std::str::from_utf8(name).ok())
            .filter(|name| safe_entry_path(name))
            .ok_or_else(|| format!("unsafe or malformed zip entry name at offset {at}"))?;
        let data = bytes
            .get(data_start..data_end)
            .ok_or_else(|| format!("zip entry {name} is truncated"))?;
        entries.push((name, data));
        at = data_end;
    }
}

/// Check the package's `Manifest.json` the way Core ML's package loader
/// does, before Core ML sees it: Core ML raises a C++ exception (which
/// would abort the host) rather than an error when a required field is
/// missing. The manifest must have `fileFormatVersion` `1.0.0`, an
/// `itemInfoEntries` object whose every entry has string `path`, `name`,
/// `author` and `description` naming a file or directory under `Data/`, and a
/// `rootModelIdentifier` that is one of those entries.
///
/// # Errors
///
/// Returns a description of the first missing or invalid field.
pub fn check_mlpackage_manifest(entries: &[(&str, &[u8])]) -> Result<(), String> {
    let manifest = entries
        .iter()
        .find(|(name, _)| *name == "Manifest.json")
        .and_then(|(_, data)| serde_json::from_slice::<serde_json::Value>(data).ok())
        .ok_or_else(|| "mlpackage has no readable Manifest.json".to_string())?;
    if manifest
        .get("fileFormatVersion")
        .and_then(serde_json::Value::as_str)
        != Some("1.0.0")
    {
        return Err("Manifest.json fileFormatVersion must be 1.0.0".to_string());
    }
    let items = manifest
        .get("itemInfoEntries")
        .and_then(serde_json::Value::as_object)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| "Manifest.json has no itemInfoEntries".to_string())?;
    for (identifier, item) in items {
        for field in ["path", "name", "author", "description"] {
            if item
                .get(field)
                .and_then(serde_json::Value::as_str)
                .is_none()
            {
                return Err(format!("Manifest.json item {identifier} has no {field}"));
            }
        }
        let path = item
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        // An item is a file or a directory (for example the weights).
        let data_path = format!("Data/{path}");
        let directory = format!("{data_path}/");
        if !entries
            .iter()
            .any(|(name, _)| *name == data_path || name.starts_with(&directory))
        {
            return Err(format!(
                "Manifest.json item {identifier} names a missing file"
            ));
        }
    }
    let root = manifest
        .get("rootModelIdentifier")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Manifest.json has no rootModelIdentifier".to_string())?;
    if !items.contains_key(root) {
        return Err("Manifest.json rootModelIdentifier names no item".to_string());
    }
    Ok(())
}

/// Unpack a canonical `mlpackage-zip` into `destination` (a `.mlpackage`
/// directory, created if needed), after checking its manifest
/// ([`check_mlpackage_manifest`]).
///
/// # Errors
///
/// Returns a description when the zip or its manifest is malformed or a
/// file cannot be written.
pub fn unpack_mlpackage_zip(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let entries = mlpackage_entries(bytes)?;
    check_mlpackage_manifest(&entries)?;
    for (name, data) in entries {
        let path = destination.join(name);
        fs::create_dir_all(path.parent().unwrap_or(destination))
            .map_err(|error| format!("{name}: {error}"))?;
        fs::write(&path, data).map_err(|error| format!("{name}: {error}"))?;
    }
    Ok(())
}

/// Element types a Core ML multi-array may hold on output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Element {
    /// IEEE half precision.
    F16,
    /// IEEE single precision.
    F32,
    /// IEEE double precision.
    F64,
}

impl Element {
    fn size(self) -> usize {
        match self {
            Self::F16 => 2,
            Self::F32 => 4,
            Self::F64 => 8,
        }
    }
}

/// Element offsets of every index of `shape`, in row-major order, under
/// `strides` (in elements). `None` on a rank mismatch or overflow.
#[must_use]
pub fn strided_offsets(shape: &[usize], strides: &[usize]) -> Option<Vec<usize>> {
    if shape.len() != strides.len() {
        return None;
    }
    let count = shape
        .iter()
        .try_fold(1_usize, |total, dim| total.checked_mul(*dim))?;
    let mut offsets = Vec::with_capacity(count);
    let mut index = vec![0_usize; shape.len()];
    for _ in 0..count {
        offsets.push(
            index
                .iter()
                .zip(strides)
                .try_fold(0_usize, |total, (at, stride)| {
                    total.checked_add(at.checked_mul(*stride)?)
                })?,
        );
        for axis in (0..shape.len()).rev() {
            index[axis] += 1;
            if index[axis] < shape[axis] {
                break;
            }
            index[axis] = 0;
        }
    }
    Some(offsets)
}

/// Write `values` (row-major) into an `f32` buffer laid out by `strides`.
/// `None` when the counts disagree or an offset falls outside `buffer`.
pub fn write_f32_strided(
    buffer: &mut [u8],
    shape: &[usize],
    strides: &[usize],
    values: &[f32],
) -> Option<()> {
    let offsets = strided_offsets(shape, strides)?;
    if offsets.len() != values.len() {
        return None;
    }
    for (offset, value) in offsets.into_iter().zip(values) {
        let start = offset.checked_mul(4)?;
        buffer
            .get_mut(start..start.checked_add(4)?)?
            .copy_from_slice(&value.to_le_bytes());
    }
    Some(())
}

/// Read a strided buffer of `element` values (row-major) as `f32`. `None`
/// when an offset falls outside `buffer`.
#[must_use]
pub fn read_f32_strided(
    buffer: &[u8],
    element: Element,
    shape: &[usize],
    strides: &[usize],
) -> Option<Vec<f32>> {
    let size = element.size();
    strided_offsets(shape, strides)?
        .into_iter()
        .map(|offset| {
            let start = offset.checked_mul(size)?;
            let bytes = buffer.get(start..start.checked_add(size)?)?;
            Some(match element {
                Element::F16 => f16_to_f32(u16::from_le_bytes(bytes.try_into().ok()?)),
                Element::F32 => f32::from_le_bytes(bytes.try_into().ok()?),
                #[allow(clippy::cast_possible_truncation)] // f64 output narrowed to the f32 frame
                Element::F64 => f64::from_le_bytes(bytes.try_into().ok()?) as f32,
            })
        })
        .collect()
}

/// Convert IEEE half-precision bits to `f32` exactly.
#[must_use]
pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let mantissa = u32::from(bits & 0x3ff);
    match exponent {
        0 => {
            let magnitude = f32::from(bits & 0x3ff) * 2.0_f32.powi(-24);
            if sign == 0 { magnitude } else { -magnitude }
        }
        0x1f => f32::from_bits(sign | 0x7f80_0000 | (mantissa << 13)),
        _ => f32::from_bits(sign | ((exponent + 112) << 23) | (mantissa << 13)),
    }
}

/// Little-endian `f32` values of a frame payload, or `None` when its length
/// is not `count` values.
#[must_use]
pub fn f32_values(payload: &[u8], count: usize) -> Option<Vec<f32>> {
    if payload.len() != count.checked_mul(4)? {
        return None;
    }
    Some(
        payload
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp // conversions here must be exact
)]
mod tests {
    use super::*;

    const FIXTURE_ZIP: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/models/digits-onnx-coreml-1.0.0/variants/coreml/model.mlpackage.zip"
    );

    /// One stored local entry, as the canonical writer emits it.
    fn entry(name: &str, data: &[u8], flags: u16, method: u16) -> Vec<u8> {
        let mut bytes = LOCAL_HEADER.to_le_bytes().to_vec();
        bytes.extend_from_slice(&20_u16.to_le_bytes());
        bytes.extend_from_slice(&flags.to_le_bytes());
        bytes.extend_from_slice(&method.to_le_bytes());
        bytes.extend_from_slice(&[0; 8]); // time, date, crc32
        let size = u32::try_from(data.len()).expect("size").to_le_bytes();
        bytes.extend_from_slice(&size);
        bytes.extend_from_slice(&size);
        bytes.extend_from_slice(&u16::try_from(name.len()).expect("name").to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    fn zip(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes: Vec<u8> = entries.concat();
        bytes.extend_from_slice(&CENTRAL_HEADER.to_le_bytes());
        bytes
    }

    #[test]
    fn compute_units_round_trip_and_default_to_all() {
        for units in [
            ComputeUnits::All,
            ComputeUnits::CpuOnly,
            ComputeUnits::CpuAndGpu,
            ComputeUnits::CpuAndNeuralEngine,
        ] {
            assert_eq!(ComputeUnits::parse(units.as_str()), Some(units));
        }
        assert_eq!(ComputeUnits::default(), ComputeUnits::All);
        assert_eq!(ComputeUnits::parse("gpu"), None);
    }

    #[test]
    fn committed_fixture_variant_unpacks_to_an_mlpackage() {
        let bytes = fs::read(FIXTURE_ZIP).expect("fixture zip");
        let names: Vec<&str> = mlpackage_entries(&bytes)
            .expect("entries")
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert!(names.contains(&"Manifest.json"));
        assert!(names.contains(&"Data/com.apple.CoreML/model.mlmodel"));
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "canonical zips are sorted");

        let dir =
            std::env::temp_dir().join(format!("traverse-coreml-unpack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        unpack_mlpackage_zip(&bytes, &dir).expect("unpack");
        assert!(dir.join("Manifest.json").is_file());
        assert!(dir.join("Data/com.apple.CoreML/model.mlmodel").is_file());
        // A file where a directory must go cannot be written.
        let blocked = dir.join("Manifest.json").join("nested");
        assert!(unpack_mlpackage_zip(&bytes, &blocked).is_err());
        // An existing directory where a file must go cannot be overwritten.
        fs::remove_file(dir.join("Manifest.json")).expect("remove");
        fs::create_dir(dir.join("Manifest.json")).expect("occupy");
        assert!(unpack_mlpackage_zip(&bytes, &dir).is_err());
        // A zip whose manifest is invalid is never unpacked.
        let empty = dir.join("never");
        assert!(unpack_mlpackage_zip(&zip(&[entry("a", b"x", 0, 0)]), &empty).is_err());
        assert!(!empty.exists());
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn unsupported_or_unsafe_zips_are_rejected() {
        let good = entry("Manifest.json", b"{}", 0, 0);
        assert_eq!(
            mlpackage_entries(&zip(std::slice::from_ref(&good))).expect("good"),
            vec![("Manifest.json", b"{}".as_slice())]
        );
        for bad in [
            zip(&[entry("a", b"x", 0, 8)]), // deflated
            zip(&[entry("a", b"x", DATA_DESCRIPTOR_FLAG, 0)]),
            zip(&[entry("../escape", b"x", 0, 0)]),
            zip(&[entry("/absolute", b"x", 0, 0)]),
            zip(&[entry("dir/", b"", 0, 0)]),
            zip(&[entry("back\\slash", b"x", 0, 0)]),
            zip(&[entry("a/./b", b"x", 0, 0)]),
            zip(&[entry("a//b", b"x", 0, 0)]),
            zip(&[entry("", b"x", 0, 0)]),
            zip(&[]), // no entries
            b"not a zip".to_vec(),
            good[..20].to_vec(),             // truncated header
            good[..good.len() - 1].to_vec(), // truncated data
        ] {
            assert!(mlpackage_entries(&bad).is_err(), "{bad:?}");
        }
        let mut short_name = good.clone();
        short_name.truncate(LOCAL_HEADER_LEN + 3);
        assert!(mlpackage_entries(&short_name).is_err());
        let mut invalid_utf8 = entry("ab", b"x", 0, 0);
        invalid_utf8[LOCAL_HEADER_LEN] = 0xff;
        assert!(mlpackage_entries(&zip(&[invalid_utf8])).is_err());
        let mut mismatched = entry("a", b"x", 0, 0);
        mismatched[18] = 2; // stored size != uncompressed size
        assert!(mlpackage_entries(&zip(&[mismatched])).is_err());
        let mut huge = entry("a", b"x", 0, 0);
        huge[18..26].copy_from_slice(&[0xff; 8]);
        assert!(mlpackage_entries(&zip(&[huge])).is_err());
    }

    #[test]
    fn manifests_core_ml_would_throw_on_are_rejected() {
        let model = ("Data/com.apple.CoreML/model.mlmodel", b"m".as_slice());
        let item = serde_json::json!({
            "path": "com.apple.CoreML/model.mlmodel",
            "name": "model.mlmodel",
            "author": "com.apple.CoreML",
            "description": "CoreML Model Specification",
        });
        let good = serde_json::json!({
            "fileFormatVersion": "1.0.0",
            "itemInfoEntries": { "A": item },
            "rootModelIdentifier": "A",
        });
        let check = |manifest: &serde_json::Value| {
            let bytes = serde_json::to_vec(manifest).expect("json");
            check_mlpackage_manifest(&[model, ("Manifest.json", bytes.as_slice())])
        };
        assert_eq!(check(&good), Ok(()));
        let mut weights = good.clone();
        weights["itemInfoEntries"]["W"] = serde_json::json!({
            "path": "com.apple.CoreML", "name": "weights", "author": "a", "description": "d",
        });
        assert_eq!(check(&weights), Ok(()));
        let mut cases = Vec::new();
        for (pointer, value) in [
            ("/fileFormatVersion", serde_json::json!("2.0.0")),
            ("/itemInfoEntries", serde_json::json!({})),
            ("/itemInfoEntries/A/author", serde_json::json!(7)),
            (
                "/itemInfoEntries/A/path",
                serde_json::json!("com.apple.CoreML/other"),
            ),
            ("/rootModelIdentifier", serde_json::json!("B")),
            ("/rootModelIdentifier", serde_json::json!(null)),
        ] {
            let mut manifest = good.clone();
            *manifest.pointer_mut(pointer).expect("pointer") = value;
            cases.push(manifest);
        }
        for manifest in &cases {
            assert!(check(manifest).is_err(), "{manifest}");
        }
        assert!(check_mlpackage_manifest(&[model]).is_err());
        assert!(check_mlpackage_manifest(&[("Manifest.json", b"{".as_slice())]).is_err());
    }

    #[test]
    fn strided_buffers_round_trip_with_padding() {
        // A [2, 3] array whose rows are padded to 4 elements.
        let (shape, strides) = ([2_usize, 3], [4_usize, 1]);
        assert_eq!(
            strided_offsets(&shape, &strides),
            Some(vec![0, 1, 2, 4, 5, 6])
        );
        let values = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let mut buffer = vec![0_u8; 8 * 4];
        write_f32_strided(&mut buffer, &shape, &strides, &values).expect("write");
        assert_eq!(
            read_f32_strided(&buffer, Element::F32, &shape, &strides),
            Some(values.to_vec())
        );
        assert!(write_f32_strided(&mut buffer, &shape, &strides, &values[..5]).is_none());
        assert!(write_f32_strided(&mut buffer[..20], &shape, &strides, &values).is_none());
        assert!(read_f32_strided(&buffer[..20], Element::F32, &shape, &strides).is_none());
        assert_eq!(strided_offsets(&shape, &[1]), None);
        assert_eq!(strided_offsets(&[usize::MAX, 2], &[1, 1]), None);
        assert_eq!(strided_offsets(&[3], &[usize::MAX]), None);
        assert_eq!(strided_offsets(&[], &[]), Some(vec![0]));

        let doubles: Vec<u8> = [0.5_f64, -2.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(
            read_f32_strided(&doubles, Element::F64, &[2], &[1]),
            Some(vec![0.5, -2.0])
        );
        let halves: Vec<u8> = [0x3c00_u16, 0xc000]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(
            read_f32_strided(&halves, Element::F16, &[2], &[1]),
            Some(vec![1.0, -2.0])
        );
    }

    #[test]
    fn half_precision_converts_exactly() {
        assert_eq!(f16_to_f32(0x0000).to_bits(), 0.0_f32.to_bits());
        assert_eq!(f16_to_f32(0x8000).to_bits(), (-0.0_f32).to_bits());
        assert_eq!(f16_to_f32(0x0001), 2.0_f32.powi(-24));
        assert_eq!(f16_to_f32(0x8001), -(2.0_f32.powi(-24)));
        assert_eq!(f16_to_f32(0x3555), 0.333_251_95);
        assert_eq!(f16_to_f32(0x7bff), 65504.0);
        assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
        assert_eq!(f16_to_f32(0xfc00), f32::NEG_INFINITY);
        assert!(f16_to_f32(0x7e00).is_nan());
    }

    #[test]
    fn f32_payloads_must_hold_exactly_count_values() {
        let bytes: Vec<u8> = [1.5_f32, -1.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(f32_values(&bytes, 2), Some(vec![1.5, -1.0]));
        assert_eq!(f32_values(&bytes, 3), None);
        assert_eq!(f32_values(&bytes, usize::MAX), None);
    }
}
