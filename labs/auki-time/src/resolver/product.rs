//! Validate retained SDK Product evidence before admitting it to the local index.

use auki_components::{
    ClockReference, OutputManifest, PayloadContract, ProductForm, RetainedProduct,
    StructuredPayloadContract, clock, manifest_hash,
};
use auki_registry::{ClockBody, ClockRegistryEntry};

use super::{
    OFFSET_SCHEMA, OffsetContract, OffsetSample, TemporalError, TemporalEvidence, TemporalResolver,
    TimeInterval,
};
use crate::components::{
    SessionClockError, SessionUtcContract, SessionUtcSample, read_session_sample,
};

const MAX_ID_BYTES: usize = 1024;

fn invalid(message: impl Into<String>) -> TemporalError {
    TemporalError::Invalid(message.into())
}

fn bounded_id(value: &str) -> Result<(), TemporalError> {
    if value.trim().is_empty() || value.len() > MAX_ID_BYTES {
        return Err(invalid("identity requires a bounded, nonempty string"));
    }
    Ok(())
}

fn validate_reference(reference: &ClockReference) -> Result<(), TemporalError> {
    bounded_id(&reference.peer_id)?;
    bounded_id(&reference.id)?;
    clock::validate_clock(reference).map_err(TemporalError::Invalid)
}

/// Obtain the exact existing registry reference, including the definition hash.
/// Registration validates the definition; this function performs no lookup.
pub fn clock_reference(definition: &ClockRegistryEntry) -> ClockReference {
    ClockReference {
        peer_id: definition.peer_id.clone(),
        id: definition.clock_id.clone(),
        hash: definition.hash(),
    }
}

impl OffsetContract {
    /// Bind the immutable relationship declaration into the Output's identity.
    pub fn output_id(&self) -> String {
        format!("clock-offset-v1-{}", manifest_hash(self))
    }

    /// The offset profile is separate from the point-only session UTC profile.
    pub fn payload_contract(&self) -> PayloadContract {
        PayloadContract::Structured(StructuredPayloadContract {
            modality: "time_transform".into(),
            datatype: OFFSET_SCHEMA.into(),
            schema: OFFSET_SCHEMA.into(),
            observes: "clock_offset".into(),
            unit: Some("ns".into()),
        })
    }

    pub fn validate(&self) -> Result<(), TemporalError> {
        validate_reference(&self.from_clock)?;
        validate_reference(&self.to_clock)?;
        if self.from_clock == self.to_clock {
            return Err(invalid(
                "offset relationship requires distinct exact clocks",
            ));
        }
        Ok(())
    }

    /// The reporter need not own either endpoint. Application admission is
    /// separate from the validity of its exact Product/Output provenance.
    pub fn validate_output(&self, output: &OutputManifest) -> Result<(), TemporalError> {
        self.validate()?;
        output.validate().map_err(TemporalError::Invalid)?;
        validate_output_identity(output)?;
        if output.clock != self.from_clock
            || output.output_id != self.output_id()
            || output.payload != self.payload_contract()
            || output.spatial_frame.is_some()
            || output.spatial_frame_id.is_some()
        {
            return Err(invalid("offset Output does not match its exact contract"));
        }
        Ok(())
    }
}

fn validate_output_identity(output: &OutputManifest) -> Result<(), TemporalError> {
    for value in [
        &output.peer_id,
        &output.component_id,
        &output.slot,
        &output.output_id,
    ] {
        bounded_id(value)?;
    }
    if output.component_manifest_hash.len() != 32
        || !output
            .component_manifest_hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("Output requires an exact Component manifest hash"));
    }
    Ok(())
}

fn validate_product<T>(product: &RetainedProduct<T>) -> Result<(), TemporalError> {
    bounded_id(&product.manifest.peer_id)?;
    bounded_id(&product.manifest.product_id)?;
    validate_output_identity(&product.producer)?;
    if product.manifest.producer != product.producer.reference()
        || product.manifest.peer_id != product.producer.peer_id
        || product.manifest.hash() != product.manifest_hash
        || product.manifest.form != ProductForm::Buffer
        || product.manifest.schema != "auki.product-manifest/v1"
    {
        return Err(invalid("offset Product provenance mismatch"));
    }
    Ok(())
}

fn validate_sample(sample: &OffsetSample, measurement_ns: u64) -> Result<(), TemporalError> {
    let validity = match sample {
        OffsetSample::Available { validity, .. } | OffsetSample::Unavailable { validity } => {
            validity
        }
    };
    if validity.start_ns > validity.end_ns
        || measurement_ns < validity.start_ns
        || measurement_ns > validity.end_ns
    {
        return Err(invalid(
            "offset applicability must be ordered and contain its measurement time",
        ));
    }
    Ok(())
}

impl TemporalResolver {
    /// Register a verified definition locally. Names, equal epochs and scope
    /// never create a relationship to any other registered clock.
    pub fn register_clock(
        &mut self,
        reference: ClockReference,
        definition: ClockRegistryEntry,
    ) -> Result<(), TemporalError> {
        validate_reference(&reference)?;
        if definition.session_id.trim().is_empty() || definition.session_id.len() > 256 {
            return Err(invalid("clock requires a bounded, nonempty session ID"));
        }
        if reference != clock_reference(&definition) {
            return Err(invalid("clock reference does not match its definition"));
        }
        let supported = match &definition.body {
            ClockBody::MonotonicClock(meta) => {
                meta.monotonic
                    && meta.epoch.is_none()
                    && matches!(meta.unit.as_str(), "ns" | "nanoseconds")
            }
            ClockBody::UtcClock(meta) => {
                !meta.monotonic
                    && meta.epoch.as_deref() == Some("1970-01-01T00:00:00Z")
                    && matches!(meta.unit.as_str(), "ns" | "nanoseconds")
            }
        };
        if !supported {
            return Err(invalid(
                "unsupported clock: requires nanosecond monotonic or Unix UTC semantics",
            ));
        }
        if let Some((_, existing)) = self.clocks.iter().find(|(key, _)| *key == reference) {
            if existing != &definition {
                return Err(invalid("exact clock definition changed"));
            }
            return Ok(());
        }
        if self.clocks.len() >= self.limits.max_clocks {
            return Err(TemporalError::LimitExceeded { resource: "clocks" });
        }
        self.clocks.push((reference, definition));
        Ok(())
    }

    pub(super) fn require_clock(&self, reference: &ClockReference) -> Result<(), TemporalError> {
        validate_reference(reference)?;
        if self.clocks.iter().any(|(key, _)| key == reference) {
            Ok(())
        } else {
            Err(TemporalError::MissingClock(Box::new(reference.clone())))
        }
    }

    /// Admit one exact retained observation. This neither fetches a remote
    /// Product nor treats Catalog discovery or transport identity as authority.
    /// The application's admission predicate runs even for duplicate evidence.
    /// Copies of the same Output/sequence retained by other Products are one
    /// measurement: the first admitted Product is its canonical provenance.
    /// A differing copy or profile reinterpretation is rejected.
    pub fn insert_offset_sample(
        &mut self,
        product: &RetainedProduct<OffsetSample>,
        contract: &OffsetContract,
        sequence: u64,
        authorize: impl FnOnce(&TemporalEvidence) -> bool,
    ) -> Result<(), TemporalError> {
        contract.validate_output(&product.producer)?;
        validate_product(product)?;
        self.require_clock(&contract.from_clock)?;
        self.require_clock(&contract.to_clock)?;
        let snapshot = product.buffer().snapshot(sequence, sequence);
        let envelope = snapshot
            .first()
            .ok_or(TemporalError::MissingSample { sequence })?;
        let observation = &envelope.payload;
        if snapshot.len() != 1
            || envelope.sequence != sequence
            || observation.sequence != sequence
            || envelope.timestamp_ns != observation.timestamp_ns
            || observation.output != product.producer.reference()
        {
            return Err(invalid("offset observation provenance mismatch"));
        }
        validate_sample(&observation.payload, observation.timestamp_ns)?;
        let evidence = TemporalEvidence {
            product: product.reference(),
            output: observation.output.clone(),
            sequence,
            measurement_ns: observation.timestamp_ns,
            contract: contract.clone(),
            sample: (*observation.payload).clone(),
            source_contract_hash: manifest_hash(contract),
            source_payload_hash: manifest_hash(&*observation.payload),
        };
        self.admit(evidence, authorize)
    }

    /// Read the existing session UTC history profile without inventing a held
    /// offset. Every such sample is applicable at its measurement instant only.
    /// Absolute ideal-UTC accuracy is not a local session-to-local-UTC error.
    pub fn insert_session_sample(
        &mut self,
        product: &RetainedProduct<SessionUtcSample>,
        contract: &SessionUtcContract,
        sequence: u64,
        authorize: impl FnOnce(&TemporalEvidence) -> bool,
    ) -> Result<(), TemporalError> {
        validate_product(product)?;
        let observation =
            read_session_sample(product, contract, sequence).map_err(|error| match error {
                SessionClockError::MissingSample { sequence } => {
                    TemporalError::MissingSample { sequence }
                }
                other => invalid(other.to_string()),
            })?;
        let offset_contract = OffsetContract {
            from_clock: contract.from_reference(),
            to_clock: contract.to_reference(),
            allow_inverse: true,
        };
        offset_contract.validate()?;
        self.require_clock(&offset_contract.from_clock)?;
        self.require_clock(&offset_contract.to_clock)?;
        let validity = TimeInterval {
            start_ns: observation.timestamp_ns,
            end_ns: observation.timestamp_ns,
        };
        let sample = match *observation.payload {
            SessionUtcSample::Available {
                offset_ns,
                uncertainty_ns,
            } => OffsetSample::Available {
                offset_ns,
                uncertainty_ns: Some(u64::from(uncertainty_ns)),
                max_drift_ppb: None,
                validity,
            },
            SessionUtcSample::Unavailable { .. } => OffsetSample::Unavailable { validity },
        };
        let evidence = TemporalEvidence {
            product: product.reference(),
            output: observation.output,
            sequence,
            measurement_ns: observation.timestamp_ns,
            contract: offset_contract,
            sample,
            source_contract_hash: manifest_hash(contract),
            source_payload_hash: manifest_hash(&*observation.payload),
        };
        self.admit(evidence, authorize)
    }

    fn admit(
        &mut self,
        evidence: TemporalEvidence,
        authorize: impl FnOnce(&TemporalEvidence) -> bool,
    ) -> Result<(), TemporalError> {
        if !authorize(&evidence) {
            return Err(TemporalError::Denied);
        }
        if let Some(existing) = self.samples.iter().find(|existing| {
            existing.output == evidence.output && existing.sequence == evidence.sequence
        }) {
            // Retaining an observation in another Product does not create an
            // independent measurement. Preserve its first admitted provenance.
            if existing.measurement_ns != evidence.measurement_ns
                || existing.contract != evidence.contract
                || existing.sample != evidence.sample
                || existing.source_contract_hash != evidence.source_contract_hash
                || existing.source_payload_hash != evidence.source_payload_hash
            {
                return Err(invalid("immutable temporal evidence changed"));
            }
            return Ok(());
        }
        if self.samples.len() >= self.limits.max_samples {
            return Err(TemporalError::LimitExceeded {
                resource: "samples",
            });
        }
        self.samples.push(evidence);
        Ok(())
    }
}
