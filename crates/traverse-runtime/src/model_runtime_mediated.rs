//! Spec 104 `MediatedConnector` binding for `traverse.model-runtime` (#1719).
//!
//! Wraps [`ExactModelHostConnector`] (`HostConnectorPort`, `&mut self`) behind
//! a `Mutex` so native hosts can activate one shipped binding for nested or
//! Wasmtime `connector_invoke` without hand-writing the bridge.

#[cfg(feature = "wasmtime-executor")]
use crate::exact_model::ExactModelHostConnector;
#[cfg(feature = "wasmtime-executor")]
use crate::executor::{
    ActivatedConnector, ConnectorInvokeRequest, ConnectorInvokeResponse, MediatedConnector,
    SUPPORTED_HOST_ABI_VERSION,
};
#[cfg(feature = "wasmtime-executor")]
use crate::host_connector_dispatch::{
    HostConnectorError, HostConnectorHostRequest, HostConnectorPort, MODEL_EXECUTE_OPERATION,
    MODEL_RUNTIME_CONNECTOR, ModelFailureReason,
};
#[cfg(feature = "wasmtime-executor")]
use serde_json::{Value, json};
#[cfg(feature = "wasmtime-executor")]
use std::sync::{Arc, Mutex};

/// Connector contract version shipped for `traverse.model-runtime`.
#[cfg(feature = "wasmtime-executor")]
pub const MODEL_RUNTIME_CONNECTOR_VERSION: &str = "2.3.0";

/// Spec 104 adapter that forwards `model.execute` to an
/// [`ExactModelHostConnector`].
#[cfg(feature = "wasmtime-executor")]
pub struct ExactModelMediatedConnector {
    host: Mutex<ExactModelHostConnector>,
    binding_id: String,
    target_family: String,
}

#[cfg(feature = "wasmtime-executor")]
impl ExactModelMediatedConnector {
    /// Wrap a configured exact-model host for Spec 104 activation.
    #[must_use]
    pub fn new(host: ExactModelHostConnector) -> Self {
        Self {
            host: Mutex::new(host),
            binding_id: "exact-model-mediated".to_string(),
            target_family: "local".to_string(),
        }
    }

    /// Override the Spec 137-shaped binding id stamped on host requests.
    #[must_use]
    pub fn with_binding_id(mut self, binding_id: impl Into<String>) -> Self {
        self.binding_id = binding_id.into();
        self
    }

    /// Override the target family stamped on host requests.
    #[must_use]
    pub fn with_target_family(mut self, target_family: impl Into<String>) -> Self {
        self.target_family = target_family.into();
        self
    }

    /// Borrow the inner host (for `register_package`, staging I/O, etc.).
    ///
    /// # Errors
    ///
    /// Returns a stable message when the host mutex is unavailable.
    pub fn with_host_mut<R>(
        &self,
        f: impl FnOnce(&mut ExactModelHostConnector) -> R,
    ) -> Result<R, String> {
        let mut guard = self
            .host
            .lock()
            .map_err(|_| "exact-model mediated connector lock unavailable".to_string())?;
        Ok(f(&mut guard))
    }
}

#[cfg(feature = "wasmtime-executor")]
impl MediatedConnector for ExactModelMediatedConnector {
    fn invoke(&self, request: &ConnectorInvokeRequest) -> Result<ConnectorInvokeResponse, String> {
        if request.connector_id != MODEL_RUNTIME_CONNECTOR
            || request.operation != MODEL_EXECUTE_OPERATION
        {
            return Ok(failure_response(
                "incompatible",
                json!({
                    "status": "error",
                    "error": {
                        "code": "incompatible",
                        "message": "ExactModelMediatedConnector only serves traverse.model-runtime model.execute",
                    }
                }),
            ));
        }

        let host_request = HostConnectorHostRequest {
            connector_id: request.connector_id.clone(),
            operation: request.operation.clone(),
            binding_id: self.binding_id.clone(),
            target_family: self.target_family.clone(),
            correlation_id: format!("mediated-{}", request.operation),
            payload: request.payload.clone(),
            cancel_requested: false,
        };

        let mut guard = self
            .host
            .lock()
            .map_err(|_| "exact-model mediated connector lock unavailable".to_string())?;
        match guard.invoke(&host_request) {
            Ok(result) => {
                let output_schema_ref = request
                    .payload
                    .get("model_ref")
                    .and_then(|model_ref| model_ref.get("digest")?.as_str())
                    .and_then(|digest| {
                        guard
                            .packages
                            .resolve_offline(digest)
                            .ok()
                            .map(|package| package.manifest.output_schema_ref.clone())
                    })
                    .unwrap_or_else(|| "schema:model-output".to_string());
                Ok(map_host_success(
                    &request.payload,
                    result,
                    output_schema_ref,
                ))
            }
            Err(error) => Ok(map_host_error(error)),
        }
    }
}

#[cfg(feature = "wasmtime-executor")]
#[allow(clippy::needless_pass_by_value)]
fn map_host_success(
    request_payload: &Value,
    result: crate::host_connector_dispatch::HostConnectorHostResult,
    output_schema_ref: String,
) -> ConnectorInvokeResponse {
    let Some(output_ref) = result.artifact_ref.filter(|value| !value.trim().is_empty()) else {
        return failure_response(
            "unavailable",
            json!({
                "status": "error",
                "error": {
                    "code": "unavailable",
                    "message": "exact-model host returned no output_ref",
                }
            }),
        );
    };
    let placement = result.placement_evidence.as_ref().map_or_else(
        || "wasm-cpu".to_string(),
        |evidence| evidence.placement.clone(),
    );
    let mut payload = json!({
        "status": "ok",
        "output_ref": output_ref,
        "output_schema_ref": output_schema_ref,
        "model_ref": request_payload.get("model_ref").cloned().unwrap_or(Value::Null),
        "placement": placement,
        "result_class": "success",
    });
    if let Some(evidence) = result.model_evidence
        && let Ok(value) = serde_json::to_value(evidence)
    {
        payload["model_evidence"] = value;
    }
    if let Some(evidence) = result.placement_evidence
        && let Ok(value) = serde_json::to_value(evidence)
    {
        payload["placement_evidence"] = value;
    }
    ConnectorInvokeResponse {
        abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
        result_class: "success".to_string(),
        payload,
    }
}

/// Activate `traverse.model-runtime` at the shipped connector version.
#[cfg(feature = "wasmtime-executor")]
#[must_use]
pub fn activate_exact_model_runtime(host: ExactModelHostConnector) -> ActivatedConnector {
    activate_exact_model_runtime_version(host, MODEL_RUNTIME_CONNECTOR_VERSION)
}

/// Activate `traverse.model-runtime` at an explicit semantic version.
#[cfg(feature = "wasmtime-executor")]
#[must_use]
pub fn activate_exact_model_runtime_version(
    host: ExactModelHostConnector,
    version: &str,
) -> ActivatedConnector {
    ActivatedConnector {
        connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
        version: version.to_string(),
        implementation: Arc::new(ExactModelMediatedConnector::new(host)),
    }
}

#[cfg(feature = "wasmtime-executor")]
fn map_host_error(error: HostConnectorError) -> ConnectorInvokeResponse {
    let result_class = error
        .reason
        .map_or_else(|| error.code.as_str(), ModelFailureReason::as_str);
    let mut error_obj = json!({
        "code": error.code.as_str(),
        "message": error.message,
    });
    if let Some(reason) = error.reason {
        error_obj["reason"] = Value::String(reason.as_str().to_string());
    }
    if let Some(detail) = error.detail
        && let Ok(value) = serde_json::to_value(detail)
    {
        error_obj["detail"] = value;
    }
    failure_response(
        result_class,
        json!({
            "status": "error",
            "error": error_obj,
        }),
    )
}

#[cfg(feature = "wasmtime-executor")]
fn failure_response(result_class: &str, payload: Value) -> ConnectorInvokeResponse {
    ConnectorInvokeResponse {
        abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
        result_class: result_class.to_string(),
        payload,
    }
}

#[cfg(all(test, feature = "wasmtime-executor"))]
mod tests {
    use super::{
        ExactModelMediatedConnector, MODEL_RUNTIME_CONNECTOR_VERSION, activate_exact_model_runtime,
        activate_exact_model_runtime_version, map_host_error, map_host_success,
    };
    use crate::exact_model::{
        CommercialUse, ExactModelHostConnector, ExactModelPin, ExecutionPolicy, ModelUsage,
        PLACEMENT_WASM_CPU, PinRights, PlacementEvidence, TrustedModelKeys, encode_guest_frame,
    };
    use crate::executor::{ConnectorInvokeRequest, MediatedConnector, SUPPORTED_HOST_ABI_VERSION};
    use crate::host_connector_dispatch::{
        HostConnectorError, HostConnectorErrorCode, HostConnectorHostResult,
        MODEL_EXECUTE_OPERATION, MODEL_RUNTIME_CONNECTOR, ModelFailureReason,
        ModelRightsDenialDetail,
    };
    use serde_json::{Value, json};
    use std::fs;

    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

    fn pin(digest: &str) -> ExactModelPin {
        ExactModelPin {
            model_id: "fixture.echo".to_string(),
            version: "1.0.0".to_string(),
            digest: digest.to_string(),
            offline_allowed: true,
            target: PLACEMENT_WASM_CPU.to_string(),
            rights: PinRights {
                license_id: "Apache-2.0".to_string(),
                commercial_use: CommercialUse::Allowed,
            },
            key_id: None,
        }
    }

    fn hex(value: &str) -> Vec<u8> {
        (0..value.len())
            .step_by(2)
            .filter_map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
            .collect()
    }

    fn read(relative: &str) -> Result<Vec<u8>, String> {
        fs::read(format!("{ROOT}/{relative}")).map_err(|error| error.to_string())
    }

    #[test]
    fn activate_helper_ships_model_runtime_identity() {
        let host = ExactModelHostConnector::new(vec![pin("deadbeef")], TrustedModelKeys::new());
        let activated = activate_exact_model_runtime(host);
        assert_eq!(activated.connector_id, MODEL_RUNTIME_CONNECTOR);
        assert_eq!(activated.version, MODEL_RUNTIME_CONNECTOR_VERSION);
        let host = ExactModelHostConnector::new(vec![pin("deadbeef")], TrustedModelKeys::new());
        let custom = activate_exact_model_runtime_version(host, "9.9.9");
        assert_eq!(custom.version, "9.9.9");
    }

    #[test]
    fn builders_override_binding_metadata() {
        let host = ExactModelHostConnector::new(vec![pin("deadbeef")], TrustedModelKeys::new());
        let connector = ExactModelMediatedConnector::new(host)
            .with_binding_id("native-binding")
            .with_target_family("macos");
        assert_eq!(connector.binding_id, "native-binding");
        assert_eq!(connector.target_family, "macos");
    }

    #[test]
    fn mediated_connector_rejects_non_model_execute() -> Result<(), String> {
        let host = ExactModelHostConnector::new(vec![pin("deadbeef")], TrustedModelKeys::new());
        let connector = ExactModelMediatedConnector::new(host);
        let response = connector.invoke(&ConnectorInvokeRequest {
            abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
            connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
            operation: "other".to_string(),
            payload: json!({}),
        })?;
        assert_eq!(response.result_class, "incompatible");
        Ok(())
    }

    #[test]
    fn mediated_connector_surfaces_stable_model_unavailable() -> Result<(), String> {
        let mut host = ExactModelHostConnector::new(vec![pin("deadbeef")], TrustedModelKeys::new());
        host.policies.insert(
            "policy-1".to_string(),
            ExecutionPolicy {
                policy_ref: "policy-1".to_string(),
                allowed_classifications: vec!["public".to_string()],
                max_output_bytes: 64,
                ..ExecutionPolicy::default()
            },
        );
        let connector = ExactModelMediatedConnector::new(host);
        let input_ref = connector.with_host_mut(|host| {
            host.io
                .stage_model_input(&encode_guest_frame(2, &[1], &[0]), 64)
                .map_err(|error| error.message)
        })??;
        let response = connector.invoke(&ConnectorInvokeRequest {
            abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
            connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
            operation: MODEL_EXECUTE_OPERATION.to_string(),
            payload: json!({
                "model_ref": {
                    "model_id": "fixture.echo",
                    "version": "1.0.0",
                    "digest": "deadbeef"
                },
                "input_ref": input_ref,
                "policy_ref": "policy-1",
                "data_classification": "public",
                "input_schema_ref": "schema:in",
                "input_schema_version": "1.0.0",
                "max_output_bytes": 64
            }),
        })?;
        assert_ne!(response.result_class, "success");
        assert_eq!(response.payload["status"], "error");
        assert!(response.payload["error"]["code"].as_str().is_some());
        Ok(())
    }

    #[test]
    fn map_host_success_covers_empty_output_and_evidence() {
        let empty = map_host_success(
            &json!({}),
            HostConnectorHostResult {
                artifact_ref: Some("   ".to_string()),
                permission_state: None,
                model_evidence: None,
                placement_evidence: None,
            },
            "schema:out".to_string(),
        );
        assert_eq!(empty.result_class, "unavailable");

        let ok = map_host_success(
            &json!({"model_ref": {"model_id": "m", "version": "1.0.0", "digest": "abc"}}),
            HostConnectorHostResult {
                artifact_ref: Some("out-1".to_string()),
                permission_state: None,
                model_evidence: None,
                placement_evidence: Some(PlacementEvidence::wasm_cpu()),
            },
            "schema:fixture-out".to_string(),
        );
        assert_eq!(ok.result_class, "success");
        assert_eq!(ok.payload["output_ref"], "out-1");
        assert_eq!(ok.payload["placement"], "wasm-cpu");
        assert_eq!(ok.payload["output_schema_ref"], "schema:fixture-out");
        assert!(ok.payload.get("placement_evidence").is_some());

        let default_placement = map_host_success(
            &json!({}),
            HostConnectorHostResult {
                artifact_ref: Some("out-2".to_string()),
                permission_state: None,
                model_evidence: None,
                placement_evidence: None,
            },
            "schema:out".to_string(),
        );
        assert_eq!(default_placement.payload["placement"], "wasm-cpu");
        assert!(
            default_placement
                .payload
                .get("placement_evidence")
                .is_none()
        );
    }

    #[test]
    fn map_host_error_preserves_reason_and_detail() {
        let response = map_host_error(HostConnectorError {
            code: HostConnectorErrorCode::ModelUnavailable,
            reason: Some(ModelFailureReason::PinMismatch),
            detail: Some(Box::new(ModelRightsDenialDetail {
                model_id: Some("fixture.echo".to_string()),
                version: Some("1.0.0".to_string()),
                digest: Some("deadbeef".to_string()),
                field: "rights.commercial_use".to_string(),
                expected: "allowed".to_string(),
                actual: "forbidden".to_string(),
                effective_usage: Some(ModelUsage::Commercial),
            })),
            message: "pin mismatch".to_string(),
        });
        assert_eq!(response.result_class, "pin_mismatch");
        assert_eq!(response.payload["error"]["reason"], "pin_mismatch");
        assert_eq!(
            response.payload["error"]["detail"]["field"],
            "rights.commercial_use"
        );
        let _ = Value::Null;
    }

    #[test]
    fn mediated_connector_runs_signed_digits_package() -> Result<(), String> {
        let vector: Value =
            serde_json::from_slice(&read("fixtures/models/conformance/signed-digits-mlp.json")?)
                .map_err(|error| error.to_string())?;
        let package_pin: ExactModelPin =
            serde_json::from_value(vector["pin"].clone()).map_err(|error| error.to_string())?;
        let public: [u8; 32] = hex(vector["trusted_public_key_hex"]
            .as_str()
            .ok_or_else(|| "missing key".to_string())?)
        .try_into()
        .map_err(|_| "key length".to_string())?;
        let mut keys = TrustedModelKeys::new();
        keys.trust(&public).map_err(|error| error.message)?;
        let mut host = ExactModelHostConnector::new(vec![package_pin.clone()], keys);
        host.model_usage = Some(ModelUsage::Commercial);
        host.register_package(
            &read("fixtures/models/digits-mlp-1.0.0/model.manifest.json")?,
            read("fixtures/models/digits-mlp-1.0.0/model.wasm")?,
            &read("fixtures/models/digits-mlp-1.0.0/model.sig.json")?,
        )
        .map_err(|error| error.message)?;
        host.policies.insert(
            "policy-1".to_string(),
            ExecutionPolicy {
                policy_ref: "policy-1".to_string(),
                allowed_classifications: vec!["sensitive".to_string()],
                max_output_bytes: 64,
                ..ExecutionPolicy::default()
            },
        );
        let connector = ExactModelMediatedConnector::new(host)
            .with_binding_id("digits-binding")
            .with_target_family("local");
        let cases = vector["cases"]
            .as_array()
            .ok_or_else(|| "missing cases".to_string())?;
        let first = cases.first().ok_or_else(|| "empty cases".to_string())?;
        let frame = hex(first["input_frame_hex"]
            .as_str()
            .ok_or_else(|| "missing input".to_string())?);
        let input_ref = connector.with_host_mut(|host| {
            host.io
                .stage_model_input(&frame, 268)
                .map_err(|error| error.message)
        })??;
        let response = connector.invoke(&ConnectorInvokeRequest {
            abi_version: SUPPORTED_HOST_ABI_VERSION.to_string(),
            connector_id: MODEL_RUNTIME_CONNECTOR.to_string(),
            operation: MODEL_EXECUTE_OPERATION.to_string(),
            payload: json!({
                "model_ref": {
                    "model_id": package_pin.model_id,
                    "version": package_pin.version,
                    "digest": package_pin.digest
                },
                "input_ref": input_ref,
                "policy_ref": "policy-1",
                "data_classification": "sensitive",
                "input_schema_ref": "schema:traverse-digits-mlp-in",
                "input_schema_version": "1.0.0",
                "max_output_bytes": 64
            }),
        })?;
        assert_eq!(response.result_class, "success");
        assert_eq!(response.payload["status"], "ok");
        assert!(response.payload["output_ref"].as_str().is_some());
        assert!(response.payload.get("model_evidence").is_some());
        Ok(())
    }
}
