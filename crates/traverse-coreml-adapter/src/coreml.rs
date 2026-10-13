//! The audited Core ML boundary (ADR-0082; Decisions 112 and 113).
//!
//! This is the only module in the crate that may use `unsafe`. Every unsafe
//! operation is one of:
//! - a call to an `objc2-core-ml` / `objc2-foundation` binding that is
//!   `unsafe` only because the Objective-C method is unverified, called with
//!   arguments of the declared types;
//! - one of the two raw buffer views Core ML hands to a block
//!   (`getMutableBytesWithHandler`, `getBytesWithHandler`), and the
//!   `strides` array that comes with the first, each valid only inside the
//!   block and never escaping it;
//! - `unsafe impl Send for CoreMlAdapter`, justified on the impl.
//!
//! Core ML reports some failures (for example a malformed package) by raising
//! an Objective-C exception, which Rust cannot unwind through. Every
//! prediction therefore runs inside `objc2::exception::catch`, so an
//! exception becomes an `execution_failed` error instead of aborting the
//! host process. Error messages carry the `NSError` domain and code only,
//! never Core ML's localized text, which can include host file paths.
//!
//! Policy, tolerance, cancellation and evidence stay in `traverse-runtime`;
//! this module only turns one guest frame into one Core ML prediction.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::fs;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use block2::StackBlock;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_core_ml::{
    MLComputeUnits, MLDictionaryFeatureProvider, MLFeatureProvider, MLFeatureValue, MLModel,
    MLModelConfiguration, MLMultiArray, MLMultiArrayDataType,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSInteger, NSNumber, NSProcessInfo, NSString, NSURL,
};
use traverse_runtime::exact_model::{
    ADAPTER_COREML, AcceleratorAdapter, CachedAcceleratorVariant, decode_guest_frame, digest_hex,
    encode_guest_frame,
};
use traverse_runtime::host_connector_dispatch::{HostConnectorError, HostConnectorErrorCode};

use crate::package::{
    ComputeUnits, Element, f32_values, read_f32_strided, unpack_mlpackage_zip, write_f32_strided,
};

/// Minimum OS for an fp32 ML Program built for iOS 17 (Spec 138 0.13.3).
#[cfg(target_os = "macos")]
const MINIMUM_OS_MAJOR: NSInteger = 14;
#[cfg(not(target_os = "macos"))]
const MINIMUM_OS_MAJOR: NSInteger = 17;

fn failed(message: &str) -> HostConnectorError {
    HostConnectorError {
        code: HostConnectorErrorCode::ExecutionFailed,
        reason: None,
        detail: None,
        message: message.to_string(),
    }
}

fn core_ml_failed(step: &str, error: &NSError) -> HostConnectorError {
    failed(&format!(
        "Core ML {step} failed ({} {})",
        error.domain(),
        error.code()
    ))
}

fn ml_compute_units(units: ComputeUnits) -> MLComputeUnits {
    match units {
        ComputeUnits::All => MLComputeUnits::All,
        ComputeUnits::CpuOnly => MLComputeUnits::CPUOnly,
        ComputeUnits::CpuAndGpu => MLComputeUnits::CPUAndGPU,
        ComputeUnits::CpuAndNeuralEngine => MLComputeUnits::CPUAndNeuralEngine,
    }
}

fn usizes(numbers: &NSArray<NSNumber>) -> Option<Vec<usize>> {
    numbers
        .iter()
        .map(|number| usize::try_from(number.integerValue()).ok())
        .collect()
}

fn ns_shape(shape: &[usize]) -> Option<Retained<NSArray<NSNumber>>> {
    let numbers = shape
        .iter()
        .map(|dim| {
            NSInteger::try_from(*dim)
                .ok()
                .map(NSNumber::numberWithInteger)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(NSArray::from_retained_slice(&numbers))
}

/// The single key of a feature-description dictionary.
fn sole_name<V: objc2::Message>(
    descriptions: &NSDictionary<NSString, V>,
) -> Option<Retained<NSString>> {
    let keys = descriptions.allKeys();
    (keys.count() == 1).then(|| keys.firstObject()).flatten()
}

/// A compiled, loaded variant and its single input and output names.
struct LoadedModel {
    model: Retained<MLModel>,
    input: Retained<NSString>,
    output: Retained<NSString>,
}

/// The Core ML adapter (`coreml`, Spec 138 FR-063 to FR-066).
///
/// It unpacks a variant's `mlpackage-zip` into a private cache directory,
/// compiles it on the device with `MLModel.compileModel`, and keeps the
/// loaded model for the adapter's lifetime, keyed by variant digest.
pub struct CoreMlAdapter {
    compute_units: ComputeUnits,
    cache_dir: PathBuf,
    models: HashMap<String, LoadedModel>,
}

// SAFETY: the adapter is only ever used through `&mut self` (the connector
// owns it behind its lock), so its Core ML objects are never touched from two
// threads at once. `MLModel`, `MLModelConfiguration` and `NSString` are not
// bound to the thread that created them (they are not main-thread-only
// UIKit/AppKit objects), so moving exclusive ownership between threads is
// sound.
unsafe impl Send for CoreMlAdapter {}

impl CoreMlAdapter {
    /// An adapter that runs predictions on `compute_units` and keeps its
    /// unpacked and compiled variants under `cache_dir`.
    #[must_use]
    pub fn new(compute_units: ComputeUnits, cache_dir: PathBuf) -> Self {
        Self {
            compute_units,
            cache_dir,
            models: HashMap::new(),
        }
    }

    fn load(
        &mut self,
        variant: &CachedAcceleratorVariant,
    ) -> Result<&LoadedModel, HostConnectorError> {
        let digest = digest_hex(&variant.bytes);
        if !self.models.contains_key(&digest) {
            let loaded = self.compile(&variant.bytes, &self.cache_dir.join(&digest))?;
            self.models.insert(digest.clone(), loaded);
        }
        self.models
            .get(&digest)
            .ok_or_else(|| failed("Core ML model cache lost an entry"))
    }

    fn compile(&self, zip: &[u8], directory: &Path) -> Result<LoadedModel, HostConnectorError> {
        // A previous process may have left a partial directory; start clean.
        let _ = fs::remove_dir_all(directory);
        let package = directory.join("model.mlpackage");
        unpack_mlpackage_zip(zip, &package)
            .map_err(|error| failed(&format!("mlpackage-zip variant: {error}")))?;
        let package_path = package
            .to_str()
            .ok_or_else(|| failed("Core ML cache path is not UTF-8"))?;
        let package_url = NSURL::fileURLWithPath(&NSString::from_str(package_path));
        // SAFETY: a file URL to the unpacked `.mlpackage`. The synchronous
        // compile is deprecated in favour of a completion handler, but the
        // connector already calls `predict` synchronously.
        #[allow(deprecated)]
        let compiled = unsafe { MLModel::compileModelAtURL_error(&package_url) }
            .map_err(|error| core_ml_failed("compile", &error))?;
        // Keep the compiled model beside the package: Core ML compiles into
        // a temporary directory the system may clean up. When the move is
        // impossible (another volume), load it where it is.
        let target = directory.join("model.mlmodelc");
        let model_url = match compiled.path() {
            Some(path) if fs::rename(path.to_string(), &target).is_ok() => target
                .to_str()
                .map(|path| NSURL::fileURLWithPath(&NSString::from_str(path)))
                .unwrap_or(compiled),
            _ => compiled,
        };
        // SAFETY: plain configuration object creation and a setter with a
        // valid `MLComputeUnits` constant.
        let configuration = unsafe { MLModelConfiguration::new() };
        unsafe { configuration.setComputeUnits(ml_compute_units(self.compute_units)) };
        // SAFETY: a file URL to a compiled `.mlmodelc` and a configuration.
        let model = unsafe {
            MLModel::modelWithContentsOfURL_configuration_error(&model_url, &configuration)
        }
        .map_err(|error| core_ml_failed("load", &error))?;
        // SAFETY: description getters on a loaded model.
        let description = unsafe { model.modelDescription() };
        let (inputs, outputs) = unsafe {
            (
                description.inputDescriptionsByName(),
                description.outputDescriptionsByName(),
            )
        };
        let input = sole_name(&inputs)
            .ok_or_else(|| failed("Core ML variant must have exactly one input"))?;
        let output = sole_name(&outputs)
            .ok_or_else(|| failed("Core ML variant must have exactly one output"))?;
        Ok(LoadedModel {
            model,
            input,
            output,
        })
    }
}

/// Run one prediction on `values` (row-major, `shape`) and return the
/// output shape and values.
fn run(
    loaded: &LoadedModel,
    shape: &[usize],
    values: &[f32],
) -> Result<(Vec<usize>, Vec<f32>), HostConnectorError> {
    let ns_shape = ns_shape(shape).ok_or_else(|| failed("input shape is too large"))?;
    // SAFETY: allocation and init of a Core ML-owned float32 array.
    let array = unsafe {
        MLMultiArray::initWithShape_dataType_error(
            MLMultiArray::alloc(),
            &ns_shape,
            MLMultiArrayDataType::Float32,
        )
    }
    .map_err(|error| core_ml_failed("input allocation", &error))?;
    let written = RefCell::new(false);
    let fill = StackBlock::new(
        |bytes: NonNull<c_void>, size: NSInteger, strides: NonNull<NSArray<NSNumber>>| {
            let Ok(size) = usize::try_from(size) else {
                return;
            };
            // SAFETY: Core ML passes the array's backing buffer, `size` bytes
            // long and exclusively ours for the duration of this block, and
            // its strides array, valid for the same duration. Neither
            // reference escapes the block.
            let (buffer, strides) = unsafe {
                (
                    std::slice::from_raw_parts_mut(bytes.as_ptr().cast::<u8>(), size),
                    strides.as_ref(),
                )
            };
            *written.borrow_mut() = usizes(strides)
                .and_then(|strides| write_f32_strided(buffer, shape, &strides, values))
                .is_some();
        },
    );
    // SAFETY: the block only writes inside the buffer it is given.
    unsafe { array.getMutableBytesWithHandler(&fill) };
    if !*written.borrow() {
        return Err(failed("input does not fit the Core ML input array"));
    }

    // SAFETY: wrapping a multi-array in a feature value.
    let value = unsafe { MLFeatureValue::featureValueWithMultiArray(&array) };
    let features: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_retained_objects(
        &[&*loaded.input],
        &[Retained::into_super(Retained::into_super(value))],
    );
    // SAFETY: a dictionary of feature name to `MLFeatureValue`, as required.
    let provider = unsafe {
        MLDictionaryFeatureProvider::initWithDictionary_error(
            MLDictionaryFeatureProvider::alloc(),
            &features,
        )
    }
    .map_err(|error| core_ml_failed("input features", &error))?;
    // SAFETY: a prediction on a loaded model with a valid feature provider.
    let outputs = unsafe {
        loaded
            .model
            .predictionFromFeatures_error(ProtocolObject::from_ref(&*provider))
    }
    .map_err(|error| core_ml_failed("prediction", &error))?;
    // SAFETY: reading the named output feature and its multi-array.
    let output = unsafe { outputs.featureValueForName(&loaded.output) }
        .and_then(|feature| unsafe { feature.multiArrayValue() })
        .ok_or_else(|| failed("Core ML output is not a multi-array"))?;
    // SAFETY: shape, strides and data type getters on the output array.
    let (out_shape, out_strides, data_type) =
        unsafe { (output.shape(), output.strides(), output.dataType()) };
    let element = match data_type {
        MLMultiArrayDataType::Float32 => Element::F32,
        MLMultiArrayDataType::Float16 => Element::F16,
        MLMultiArrayDataType::Float64 => Element::F64,
        _ => return Err(failed("Core ML output is not a floating-point array")),
    };
    let (Some(out_shape), Some(out_strides)) = (usizes(&out_shape), usizes(&out_strides)) else {
        return Err(failed("Core ML output shape is invalid"));
    };
    let copied = RefCell::new(Vec::new());
    let read = StackBlock::new(|bytes: NonNull<c_void>, size: NSInteger| {
        let Ok(size) = usize::try_from(size) else {
            return;
        };
        // SAFETY: Core ML passes the output's backing buffer, `size` bytes
        // long and valid for the duration of this block; it is copied out.
        let buffer = unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<u8>(), size) };
        *copied.borrow_mut() = buffer.to_vec();
    });
    // SAFETY: the block only reads inside the buffer it is given.
    unsafe { output.getBytesWithHandler(&read) };
    let values = read_f32_strided(&copied.borrow(), element, &out_shape, &out_strides)
        .ok_or_else(|| failed("Core ML output buffer is smaller than its shape"))?;
    Ok((out_shape, values))
}

impl AcceleratorAdapter for CoreMlAdapter {
    fn id(&self) -> &str {
        ADAPTER_COREML
    }

    fn available(&self) -> bool {
        NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            >= MINIMUM_OS_MAJOR
    }

    fn compute_units(&self) -> String {
        self.compute_units.as_str().to_string()
    }

    fn platform_version(&self) -> String {
        NSProcessInfo::processInfo()
            .operatingSystemVersionString()
            .to_string()
    }

    fn predict(
        &mut self,
        variant: &CachedAcceleratorVariant,
        input: &[u8],
    ) -> Result<Vec<u8>, HostConnectorError> {
        objc2::exception::catch(AssertUnwindSafe(|| self.predict_frame(variant, input)))
            .unwrap_or_else(|_| Err(failed("Core ML raised an exception")))
    }
}

impl CoreMlAdapter {
    fn predict_frame(
        &mut self,
        variant: &CachedAcceleratorVariant,
        input: &[u8],
    ) -> Result<Vec<u8>, HostConnectorError> {
        let contract = variant
            .frame_contract()
            .ok_or_else(|| failed("variant conformance vector has no frame contract"))?;
        let (dtype, dims, payload) = decode_guest_frame(input)?;
        if dtype != contract.input_dtype {
            return Err(failed("input frame dtype does not match the variant"));
        }
        let shape: Vec<usize> = dims
            .iter()
            .map(|dim| usize::try_from(*dim).ok())
            .collect::<Option<_>>()
            .ok_or_else(|| failed("input dims are too large"))?;
        let count = shape
            .iter()
            .try_fold(1_usize, |total, dim| total.checked_mul(*dim))
            .ok_or_else(|| failed("input dims are too large"))?;
        let values =
            f32_values(&payload, count).ok_or_else(|| failed("input payload is not f32 values"))?;
        let loaded = self.load(variant)?;
        let (out_shape, out_values) = run(loaded, &shape, &values)?;
        let out_dims = out_shape
            .iter()
            .map(|dim| u32::try_from(*dim).ok())
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| failed("Core ML output dims are too large"))?;
        let bytes: Vec<u8> = out_values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        Ok(encode_guest_frame(contract.output_dtype, &out_dims, &bytes))
    }
}
