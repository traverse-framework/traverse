//! Native accelerator adapters (Spec 138 0.12, FR-060 to FR-066; Decisions
//! 110 and 112).
//!
//! An adapter runs a package's signed variant on a native accelerator
//! instead of the `wasm-cpu` guest. The connector owns every rule around it:
//! the policy's placement order, fallback, the device self-check, ceilings,
//! cancellation, and evidence. An adapter only answers four questions (is it
//! available, which compute units and platform version key its self-check,
//! does it meter fuel) and runs one prediction on the guest frame format.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    ADAPTER_COREML, AcceleratorVariant, CachedAcceleratorVariant, PLACEMENT_WASM_CPU,
    VariantFrameContract, VariantTolerance, decode_guest_frame, hex_decode,
};
use crate::host_connector_dispatch::HostConnectorError;

/// Whether a placement meters fuel (FR-064).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FuelMetering {
    /// Fuel is metered (`wasm-cpu`).
    Metered,
    /// The adapter cannot meter fuel; `fuel_consumed` is omitted.
    NotApplicable,
}

/// The placement that ran a `model.execute` (FR-061).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementEvidence {
    /// `wasm-cpu` or an adapter id.
    pub placement: String,
    /// Compute units the placement used (`cpu` for `wasm-cpu`).
    pub compute_units: String,
    /// Whether fuel was metered.
    pub fuel_metering: FuelMetering,
}

impl PlacementEvidence {
    /// Evidence for the `wasm-cpu` guest.
    #[must_use]
    pub fn wasm_cpu() -> Self {
        Self {
            placement: PLACEMENT_WASM_CPU.to_string(),
            compute_units: "cpu".to_string(),
            fuel_metering: FuelMetering::Metered,
        }
    }
}

/// A native accelerator adapter (FR-063 to FR-065). Implementations only
/// run predictions; the connector enforces every Spec 138 rule around them.
pub trait AcceleratorAdapter: Send {
    /// Adapter id from the closed list (FR-066), for example `coreml`.
    fn id(&self) -> &str;

    /// Whether this device can run the adapter at all.
    fn available(&self) -> bool;

    /// Compute units the adapter is configured to use. Recorded in
    /// evidence and part of the self-check key.
    fn compute_units(&self) -> String;

    /// Platform version (for example the OS version). Part of the
    /// self-check key, so an OS update re-runs the check.
    fn platform_version(&self) -> String;

    /// Whether the adapter meters fuel. Native accelerators do not.
    fn fuel_metering(&self) -> FuelMetering {
        FuelMetering::NotApplicable
    }

    /// Run one prediction: consume a guest input frame and return a guest
    /// output frame (FR-065). The connector checks cancellation and the
    /// deadline before and after, so an adapter that cannot interrupt a
    /// prediction need not.
    ///
    /// # Errors
    ///
    /// Returns a Spec 138 error when the prediction fails.
    fn predict(
        &mut self,
        variant: &CachedAcceleratorVariant,
        input: &[u8],
    ) -> Result<Vec<u8>, HostConnectorError>;
}

/// Placement ids a policy may name (FR-060, FR-066).
pub(super) const KNOWN_PLACEMENTS: [&str; 2] = [PLACEMENT_WASM_CPU, ADAPTER_COREML];

/// A policy's effective placement order: the default is `["wasm-cpu"]`.
/// Returns `None` when the list names an unknown id or repeats one.
pub(super) fn effective_placements(placements: &[String]) -> Option<Vec<&str>> {
    if placements.is_empty() {
        return Some(vec![PLACEMENT_WASM_CPU]);
    }
    let mut seen = Vec::new();
    for id in placements {
        if !KNOWN_PLACEMENTS.contains(&id.as_str()) || seen.contains(&id.as_str()) {
            return None;
        }
        seen.push(id.as_str());
    }
    Some(seen)
}

/// Self-check results keyed by variant digest, adapter, platform version,
/// and compute units (FR-062).
pub(super) type SelfCheckKey = (String, String, String, String);

/// What a device self-check found (FR-062): whether the variant passed and,
/// for a failure, the worst case, so the host trace shows why a variant was
/// skipped on this device (Spec 138 "Device self-check"; `#1708`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelfCheckEvidence {
    /// Every case was within the variant's tolerance.
    pub passed: bool,
    /// Conformance-vector cases run.
    pub cases: u32,
    /// Cases outside the tolerance, or whose prediction failed.
    pub failed_cases: u32,
    /// Largest `|Δ|` over the declared float outputs of every case, after
    /// `score_transform`; `None` when no declared float output was compared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worst_abs_diff: Option<f64>,
    /// Zero-based index of the case with `worst_abs_diff`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worst_case: Option<u32>,
    /// Cases whose `top_k` indices differed from the reference.
    pub top_k_mismatches: u32,
    /// Cases whose prediction failed or returned a frame with another
    /// dtype, shape, or undeclared-dtype bytes.
    pub other_failures: u32,
}

impl SelfCheckEvidence {
    /// A vector that could not be read, or had no cases, never passes.
    fn unreadable() -> Self {
        Self {
            passed: false,
            cases: 0,
            failed_cases: 0,
            worst_abs_diff: None,
            worst_case: None,
            top_k_mismatches: 0,
            other_failures: 0,
        }
    }
}

/// Run (or reuse) the device self-check for one variant. Every case runs,
/// so a failure records its worst case. A failure, an unreadable vector, or
/// any prediction error makes the variant unavailable on this device.
pub(super) fn self_check(
    cache: &mut HashMap<SelfCheckKey, SelfCheckEvidence>,
    adapter: &mut dyn AcceleratorAdapter,
    variant: &AcceleratorVariant,
    cached: &CachedAcceleratorVariant,
) -> SelfCheckEvidence {
    let key = (
        variant.digest.clone(),
        adapter.id().to_string(),
        adapter.platform_version(),
        adapter.compute_units(),
    );
    if let Some(evidence) = cache.get(&key) {
        return evidence.clone();
    }
    let cases = frame_contract(&cached.conformance_vector)
        .and_then(|_| vector_cases(&cached.conformance_vector))
        .filter(|cases| !cases.is_empty());
    let evidence = cases.map_or_else(SelfCheckEvidence::unreadable, |cases| {
        let mut evidence = SelfCheckEvidence {
            passed: true,
            cases: u32::try_from(cases.len()).unwrap_or(u32::MAX),
            ..SelfCheckEvidence::unreadable()
        };
        for (index, (input, expected)) in cases.iter().enumerate() {
            let comparison = adapter
                .predict(cached, input)
                .map_or(CaseComparison::FAILED, |actual| {
                    compare_case(expected, &actual, &variant.tolerance)
                });
            if let Some(diff) = comparison.worst_abs_diff
                && evidence
                    .worst_abs_diff
                    .is_none_or(|worst| diff > worst || diff.is_nan())
            {
                evidence.worst_abs_diff = Some(diff);
                evidence.worst_case = Some(u32::try_from(index).unwrap_or(u32::MAX));
            }
            if !comparison.passed {
                evidence.passed = false;
                evidence.failed_cases += 1;
                if comparison.top_k_mismatch {
                    evidence.top_k_mismatches += 1;
                }
                if comparison.worst_abs_diff.is_none() {
                    evidence.other_failures += 1;
                }
            }
        }
        evidence
    });
    cache.insert(key, evidence.clone());
    evidence
}

/// The `{input_frame_hex, output_frame_hex}` cases of a conformance vector.
fn vector_cases(vector: &[u8]) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
    let value: Value = serde_json::from_slice(vector).ok()?;
    value
        .get("cases")?
        .as_array()?
        .iter()
        .map(|case| {
            let input = hex_decode(case.get("input_frame_hex")?.as_str()?)?;
            let output = hex_decode(case.get("output_frame_hex")?.as_str()?)?;
            Some((input, output))
        })
        .collect()
}

/// The dtype codes shared by every case of a conformance vector (FR-065).
pub(super) fn frame_contract(vector: &[u8]) -> Option<VariantFrameContract> {
    let mut contract = None;
    for (input, expected) in vector_cases(vector)? {
        let case = VariantFrameContract {
            input_dtype: decode_guest_frame(&input).ok()?.0,
            output_dtype: decode_guest_frame(&expected).ok()?.0,
        };
        if contract.is_some_and(|seen| seen != case) {
            return None;
        }
        contract = Some(case);
    }
    contract
}

/// One self-check case compared with its reference (FR-062).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CaseComparison {
    /// Within the variant's tolerance.
    pub(super) passed: bool,
    /// Largest `|Δ|` over a declared float output, after `score_transform`.
    pub(super) worst_abs_diff: Option<f64>,
    /// The `top_k` indices differed.
    pub(super) top_k_mismatch: bool,
}

impl CaseComparison {
    const FAILED: Self = Self {
        passed: false,
        worst_abs_diff: None,
        top_k_mismatch: false,
    };
}

/// Compare an accelerated output frame with the `wasm-cpu` reference
/// (FR-062, Decision 106). Frames must agree on dtype and dims. For a dtype
/// the variant declares in `tolerance.f32_dtypes`, every element must be
/// within `max_abs_diff`, and the indices of the `top_k` largest values must
/// match in order. Declared outputs are first mapped by
/// `tolerance.score_transform` (Decision 116). Any other dtype must match
/// byte-for-byte. A NaN difference is reported as the worst and never
/// passes.
pub(super) fn compare_case(
    expected: &[u8],
    actual: &[u8],
    tolerance: &VariantTolerance,
) -> CaseComparison {
    let (Ok((dtype, dims, reference)), Ok((actual_dtype, actual_dims, values))) =
        (decode_guest_frame(expected), decode_guest_frame(actual))
    else {
        return CaseComparison::FAILED;
    };
    if dtype != actual_dtype || dims != actual_dims {
        return CaseComparison::FAILED;
    }
    if !tolerance.f32_dtypes.contains(&dtype) {
        return CaseComparison {
            passed: reference == values,
            ..CaseComparison::FAILED
        };
    }
    let (Some(reference), Some(values)) = (f32s(&reference), f32s(&values)) else {
        return CaseComparison::FAILED;
    };
    if reference.len() != values.len() {
        return CaseComparison::FAILED;
    }
    let transform = tolerance.score_transform;
    let worst = reference
        .iter()
        .zip(&values)
        .map(|(left, right)| (transform.apply(*left) - transform.apply(*right)).abs())
        .fold(0.0_f64, |worst, diff| {
            if diff.is_nan() || worst.is_nan() {
                f64::NAN
            } else {
                worst.max(diff)
            }
        });
    let k = usize::try_from(tolerance.top_k).unwrap_or(usize::MAX);
    let top_k_mismatch = top_indices(&reference, k) != top_indices(&values, k);
    CaseComparison {
        passed: worst.le(&tolerance.max_abs_diff) && !top_k_mismatch,
        worst_abs_diff: Some(worst),
        top_k_mismatch,
    }
}

/// Whether an accelerated output is within the variant's tolerance.
#[cfg(test)]
fn within_tolerance(expected: &[u8], actual: &[u8], tolerance: &VariantTolerance) -> bool {
    compare_case(expected, actual, tolerance).passed
}

fn f32s(payload: &[u8]) -> Option<Vec<f32>> {
    payload.len().is_multiple_of(4).then(|| {
        payload
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect()
    })
}

/// Indices of the `k` largest values, largest first; ties keep index order.
fn top_indices(values: &[f32], k: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|left, right| values[*right].total_cmp(&values[*left]));
    order.truncate(k);
    order
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::exact_model::encode_guest_frame;

    fn frame(dtype: u8, values: &[f32]) -> Vec<u8> {
        let payload: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        encode_guest_frame(
            dtype,
            &[u32::try_from(values.len()).expect("len")],
            &payload,
        )
    }

    fn tolerance(top_k: u32) -> VariantTolerance {
        VariantTolerance {
            max_abs_diff: 5e-3,
            top_k,
            f32_dtypes: vec![3],
            score_transform: crate::exact_model::ScoreTransform::Identity,
        }
    }

    #[test]
    fn tolerance_compares_declared_float_dtypes_elementwise_and_by_rank() {
        let reference = frame(3, &[0.1, 0.9, 0.5]);
        assert!(within_tolerance(
            &reference,
            &frame(3, &[0.102, 0.899, 0.501]),
            &tolerance(2)
        ));
        // Outside max_abs_diff.
        assert!(!within_tolerance(
            &reference,
            &frame(3, &[0.11, 0.9, 0.5]),
            &tolerance(0)
        ));
        // Within max_abs_diff but the top-2 order flips.
        let close = frame(3, &[0.1, 0.501, 0.503]);
        let base = frame(3, &[0.1, 0.503, 0.501]);
        assert!(!within_tolerance(&base, &close, &tolerance(2)));
        assert!(within_tolerance(&base, &close, &tolerance(0)));
        // NaN never passes.
        assert!(!within_tolerance(
            &reference,
            &frame(3, &[f32::NAN, 0.9, 0.5]),
            &tolerance(0)
        ));
        // Dtype or dims disagreement, and malformed frames.
        assert!(!within_tolerance(
            &reference,
            &frame(2, &[0.1, 0.9, 0.5]),
            &tolerance(0)
        ));
        assert!(!within_tolerance(
            &reference,
            &frame(3, &[0.1, 0.9]),
            &tolerance(0)
        ));
        assert!(!within_tolerance(&reference, b"bad", &tolerance(0)));
        // An undeclared dtype (here: integer labels as 4-byte values that
        // would pass as denormal floats) must match byte-for-byte.
        let labels = encode_guest_frame(7, &[1], &3_u32.to_le_bytes());
        let other = encode_guest_frame(7, &[1], &4_u32.to_le_bytes());
        assert!(!within_tolerance(&labels, &other, &tolerance(0)));
        assert!(within_tolerance(&labels, &labels, &tolerance(0)));
        // A declared float dtype whose payload is not a whole number of f32s.
        let ragged = encode_guest_frame(3, &[1], &[1, 2, 3]);
        assert!(!within_tolerance(&ragged, &ragged, &tolerance(0)));
    }

    #[test]
    fn case_comparison_reports_the_worst_difference_and_top_k_mismatch() {
        let reference = frame(3, &[0.1, 0.9, 0.5]);
        let shifted = compare_case(&reference, &frame(3, &[0.1, 0.89, 0.5]), &tolerance(1));
        assert!(!shifted.passed && !shifted.top_k_mismatch);
        assert!(
            shifted
                .worst_abs_diff
                .is_some_and(|diff| (diff - 0.01).abs() < 1e-6)
        );
        let flipped = compare_case(&reference, &frame(3, &[0.1, 0.5, 0.9]), &tolerance(1));
        assert!(flipped.top_k_mismatch && !flipped.passed);
        for nan in [[f32::NAN, 0.9, 0.5], [0.1, f32::NAN, 0.5]] {
            let compared = compare_case(&reference, &frame(3, &nan), &tolerance(0));
            assert!(!compared.passed && compared.worst_abs_diff.is_some_and(f64::is_nan));
        }
        let labels = frame(5, &[1.0, 2.0]);
        assert_eq!(
            compare_case(&labels, &labels, &tolerance(0)),
            CaseComparison {
                passed: true,
                worst_abs_diff: None,
                top_k_mismatch: false,
            }
        );
    }

    /// Answers each prediction with its input shifted by the next delta.
    struct Shift(Vec<f32>);

    impl AcceleratorAdapter for Shift {
        fn id(&self) -> &'static str {
            ADAPTER_COREML
        }

        fn available(&self) -> bool {
            true
        }

        fn compute_units(&self) -> String {
            "all".to_string()
        }

        fn platform_version(&self) -> String {
            "os".to_string()
        }

        fn predict(
            &mut self,
            _variant: &CachedAcceleratorVariant,
            input: &[u8],
        ) -> Result<Vec<u8>, HostConnectorError> {
            let delta = self.0.remove(0);
            let (_, _, payload) = decode_guest_frame(input)?;
            let values: Vec<f32> = f32s(&payload)
                .unwrap_or_default()
                .iter()
                .map(|value| value + delta)
                .collect();
            Ok(frame(3, &values))
        }
    }

    #[test]
    fn self_check_runs_every_case_and_keeps_the_worst() {
        let case = frame(3, &[0.1, 0.9]);
        let cached = CachedAcceleratorVariant {
            adapter: ADAPTER_COREML.to_string(),
            path: "variants/coreml/model.mlpackage.zip".to_string(),
            bytes: Vec::new(),
            conformance_vector_path: "variants/coreml/conformance.json".to_string(),
            conformance_vector: vector(&[(&case, &case), (&case, &case), (&case, &case)]),
        };
        let variant: AcceleratorVariant = serde_json::from_value(serde_json::json!({
            "adapter": "coreml",
            "format": "mlpackage-zip",
            "path": "variants/coreml/model.mlpackage.zip",
            "digest": "ab".repeat(32),
            "conversion": { "tool": "t", "tool_version": "1", "source_digest": "cd".repeat(32) },
            "conformance_vector_path": "variants/coreml/conformance.json",
            "conformance_vector_digest": "ef".repeat(32),
            "tolerance": { "max_abs_diff": 5e-3, "top_k": 0, "f32_dtypes": [3] },
            "peak_memory_bytes": 1024,
        }))
        .expect("variant");
        let mut cache = HashMap::new();
        let evidence = self_check(
            &mut cache,
            &mut Shift(vec![0.001, 0.02, 0.01]),
            &variant,
            &cached,
        );
        assert!(!evidence.passed);
        assert_eq!(
            (evidence.cases, evidence.failed_cases, evidence.worst_case),
            (3, 2, Some(1))
        );
        assert!(
            evidence
                .worst_abs_diff
                .is_some_and(|diff| (diff - 0.02).abs() < 1e-6)
        );
        // A stored result is reused without predicting again.
        assert_eq!(
            self_check(&mut cache, &mut Shift(Vec::new()), &variant, &cached),
            evidence
        );
    }

    #[test]
    fn score_transform_compares_sigmoid_scores_instead_of_raw_logits() {
        use crate::exact_model::ScoreTransform;
        assert!((ScoreTransform::Sigmoid.apply(0.0) - 0.5).abs() < 1e-12);
        assert!((ScoreTransform::Identity.apply(1.5) - 1.5).abs() < 1e-12);
        // Logits far from 0 differ by 3e-2 raw, but their sigmoid scores
        // differ by under 5e-3 (Decision 106's bar for BirdNET).
        let reference = frame(3, &[6.0, -6.0, 2.0]);
        let accelerated = frame(3, &[6.03, -6.03, 2.001]);
        let mut raw = tolerance(3);
        assert!(!within_tolerance(&reference, &accelerated, &raw));
        raw.score_transform = ScoreTransform::Sigmoid;
        assert!(within_tolerance(&reference, &accelerated, &raw));
        // Near 0 the sigmoid is steep enough that a 3e-2 logit change fails.
        let near_zero = frame(3, &[0.0, -6.0, 2.0]);
        let shifted = frame(3, &[0.03, -6.0, 2.0]);
        assert!(!within_tolerance(&near_zero, &shifted, &raw));
    }

    #[test]
    fn placements_default_to_wasm_cpu_and_reject_unknown_or_repeated_ids() {
        assert_eq!(effective_placements(&[]), Some(vec![PLACEMENT_WASM_CPU]));
        let both = ["coreml".to_string(), "wasm-cpu".to_string()];
        assert_eq!(
            effective_placements(&both),
            Some(vec!["coreml", "wasm-cpu"])
        );
        assert_eq!(effective_placements(&["metal".to_string()]), None);
        assert_eq!(
            effective_placements(&["wasm-cpu".to_string(), "wasm-cpu".to_string()]),
            None
        );
        assert_eq!(
            PlacementEvidence::wasm_cpu().fuel_metering,
            FuelMetering::Metered
        );
    }

    /// An adapter that relies on every default.
    struct Minimal;

    impl AcceleratorAdapter for Minimal {
        fn id(&self) -> &'static str {
            "coreml"
        }
        fn available(&self) -> bool {
            true
        }
        fn compute_units(&self) -> String {
            "all".to_string()
        }
        fn platform_version(&self) -> String {
            "1".to_string()
        }
        fn predict(
            &mut self,
            _variant: &CachedAcceleratorVariant,
            input: &[u8],
        ) -> Result<Vec<u8>, HostConnectorError> {
            Ok(input.to_vec())
        }
    }

    #[test]
    fn native_adapters_do_not_meter_fuel_by_default() {
        assert_eq!(Minimal.fuel_metering(), FuelMetering::NotApplicable);
        assert_eq!((Minimal.id(), Minimal.available()), ("coreml", true));
        assert_eq!(
            (Minimal.compute_units(), Minimal.platform_version()),
            ("all".to_string(), "1".to_string())
        );
        let variant = CachedAcceleratorVariant {
            adapter: "coreml".to_string(),
            path: "p".to_string(),
            bytes: Vec::new(),
            conformance_vector_path: "v".to_string(),
            conformance_vector: Vec::new(),
        };
        assert_eq!(Minimal.predict(&variant, b"x").expect("echo"), b"x");
    }

    fn vector(cases: &[(&[u8], &[u8])]) -> Vec<u8> {
        let cases: Vec<Value> = cases
            .iter()
            .map(|(input, output)| {
                serde_json::json!({
                    "input_frame_hex": crate::exact_model::hex_encode(input),
                    "output_frame_hex": crate::exact_model::hex_encode(output),
                })
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({ "cases": cases })).expect("vector")
    }

    #[test]
    fn frame_contract_comes_from_vector_cases_that_agree() {
        let input = encode_guest_frame(2, &[1], &1.0_f32.to_le_bytes());
        let output = encode_guest_frame(3, &[1], &2.0_f32.to_le_bytes());
        let other_output = encode_guest_frame(4, &[1], &2.0_f32.to_le_bytes());
        assert_eq!(
            frame_contract(&vector(&[(&input, &output), (&input, &output)])),
            Some(VariantFrameContract {
                input_dtype: 2,
                output_dtype: 3,
            })
        );
        assert_eq!(
            frame_contract(&vector(&[(&input, &output), (&input, &other_output)])),
            None
        );
        assert_eq!(frame_contract(&vector(&[])), None);
        assert_eq!(frame_contract(&vector(&[(b"x", &output)])), None);
        assert_eq!(frame_contract(&vector(&[(&input, b"x")])), None);
        assert_eq!(frame_contract(b"not json"), None);
        let cached = CachedAcceleratorVariant {
            adapter: ADAPTER_COREML.to_string(),
            path: "variants/coreml/model.mlpackage.zip".to_string(),
            bytes: Vec::new(),
            conformance_vector_path: "variants/coreml/conformance.json".to_string(),
            conformance_vector: vector(&[(&input, &output)]),
        };
        assert_eq!(
            cached
                .frame_contract()
                .map(|contract| contract.output_dtype),
            Some(3)
        );
    }

    #[test]
    fn vector_cases_reject_malformed_vectors() {
        assert!(vector_cases(b"not json").is_none());
        assert!(
            vector_cases(br#"{"cases":[{"input_frame_hex":"zz","output_frame_hex":"00"}]}"#)
                .is_none()
        );
        assert_eq!(
            vector_cases(br#"{"cases":[{"input_frame_hex":"01","output_frame_hex":"02"}]}"#),
            Some(vec![(vec![1], vec![2])])
        );
    }
}
