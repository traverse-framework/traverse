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
    VariantTolerance, decode_guest_frame, hex_decode,
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

/// Run (or reuse) the device self-check for one variant. A failure, an
/// unreadable vector, or any prediction error makes the variant unavailable
/// on this device.
pub(super) fn self_check(
    cache: &mut HashMap<SelfCheckKey, bool>,
    adapter: &mut dyn AcceleratorAdapter,
    variant: &AcceleratorVariant,
    cached: &CachedAcceleratorVariant,
) -> bool {
    let key = (
        variant.digest.clone(),
        adapter.id().to_string(),
        adapter.platform_version(),
        adapter.compute_units(),
    );
    if let Some(passed) = cache.get(&key) {
        return *passed;
    }
    let passed = vector_cases(&cached.conformance_vector).is_some_and(|cases| {
        !cases.is_empty()
            && cases.iter().all(|(input, expected)| {
                adapter
                    .predict(cached, input)
                    .is_ok_and(|actual| within_tolerance(expected, &actual, &variant.tolerance))
            })
    });
    cache.insert(key, passed);
    passed
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

/// Compare an accelerated output frame with the `wasm-cpu` reference
/// (FR-062, Decision 106). Frames must agree on dtype and dims. For a dtype
/// the variant declares in `tolerance.f32_dtypes`, every element must be
/// within `max_abs_diff`, and the indices of the `top_k` largest values must
/// match in order. Any other dtype must match byte-for-byte.
pub(super) fn within_tolerance(
    expected: &[u8],
    actual: &[u8],
    tolerance: &VariantTolerance,
) -> bool {
    let (Ok((dtype, dims, reference)), Ok((actual_dtype, actual_dims, values))) =
        (decode_guest_frame(expected), decode_guest_frame(actual))
    else {
        return false;
    };
    if dtype != actual_dtype || dims != actual_dims {
        return false;
    }
    if !tolerance.f32_dtypes.contains(&dtype) {
        return reference == values;
    }
    let (Some(reference), Some(values)) = (f32s(&reference), f32s(&values)) else {
        return false;
    };
    if reference.len() != values.len()
        || reference.iter().zip(&values).any(|(left, right)| {
            !(f64::from(*left) - f64::from(*right))
                .abs()
                .le(&tolerance.max_abs_diff)
        })
    {
        return false;
    }
    let k = usize::try_from(tolerance.top_k).unwrap_or(usize::MAX);
    top_indices(&reference, k) == top_indices(&values, k)
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
