//! Deterministic argmax decoder over S3 model artifacts.

use gbf_artifact::{CharId, ModelArtifact, TextCharSeq, is_text_char_id};

use super::{
    OracleError, ResolvedLogitTensor, argmax_lowest_index, logits_from_resolved,
    quant_spec_resolved_logit_tensors, validate_model_artifact_contract,
};

/// Tracing target for artifact decoder evaluation.
pub const ARTIFACT_DECODER_LOG_TARGET: &str = "gbf_oracle::artifact_decoder";

/// Decode started event name.
pub const EVENT_NAME_DECODE_STARTED: &str = "s3::artifact_decoder::decode_started";
/// Per-step decode event name.
pub const EVENT_NAME_DECODE_STEP: &str = "s3::artifact_decoder::step";
/// Decode complete event name.
pub const EVENT_NAME_DECODE_COMPLETE: &str = "s3::artifact_decoder::decode_complete";

/// Autoregressive argmax decoder for a canonical S3 model artifact.
#[derive(Debug, Clone)]
pub struct ArtifactDecoder<'a> {
    artifact: &'a ModelArtifact,
    resolved_weights: Vec<ResolvedLogitTensor>,
}

impl<'a> ArtifactDecoder<'a> {
    /// Wrap a valid model artifact for deterministic decode.
    ///
    /// This constructor asserts the S3 artifact-input contract. Use
    /// [`Self::try_new`] when callers need to surface validation errors.
    #[must_use]
    pub fn new(artifact: &'a ModelArtifact) -> Self {
        Self::try_new(artifact).expect("ArtifactDecoder requires a valid S3 artifact contract")
    }

    /// Fallibly wrap a model artifact for deterministic decode.
    pub fn try_new(artifact: &'a ModelArtifact) -> Result<Self, OracleError> {
        validate_model_artifact_contract(artifact)?;
        let (resolved_weights, _) = quant_spec_resolved_logit_tensors(artifact)?;
        Ok(Self {
            artifact,
            resolved_weights,
        })
    }

    /// Decode by deterministic argmax. The S3 decoder has `rng_spec = NoRng`.
    #[must_use]
    pub fn decode_argmax(
        &self,
        prompt: &TextCharSeq,
        max_chars: usize,
        stop_on_eos: bool,
    ) -> ArtifactDecodeResult {
        tracing::info!(
            target: ARTIFACT_DECODER_LOG_TARGET,
            event_name = EVENT_NAME_DECODE_STARTED,
            prompt_char_count = prompt.len() as u64,
            max_chars = max_chars as u64,
            stop_on_eos,
        );

        let mut current_prompt = prompt.clone();
        let mut generated = Vec::new();
        let mut decode_log = Vec::new();
        let mut terminal_eos_seen = false;
        let eos = self.artifact.core.lexical.control_tokens.eos;

        for step in 0..max_chars {
            let logits =
                logits_from_resolved(self.artifact, &current_prompt, &self.resolved_weights);
            let (token, logit_max) = argmax_lowest_index(&logits);
            decode_log.push(DecodeStep {
                step: step as u32,
                token,
                logit_max,
            });
            tracing::trace!(
                target: ARTIFACT_DECODER_LOG_TARGET,
                event_name = EVENT_NAME_DECODE_STEP,
                step = step as u64,
                token = token as u64,
                logit_max = logit_max as f64,
            );

            if token == eos {
                terminal_eos_seen = true;
                if stop_on_eos {
                    break;
                }
            }
            if !is_text_char_id(token) {
                break;
            }

            generated.push(token);
            let mut next_prompt = current_prompt.as_slice().to_vec();
            next_prompt.push(token);
            current_prompt =
                TextCharSeq::new(next_prompt).expect("decoder emits normalized text chars");
        }

        let generated =
            TextCharSeq::new(generated).expect("decoder generated chars remain normalized text");
        tracing::info!(
            target: ARTIFACT_DECODER_LOG_TARGET,
            event_name = EVENT_NAME_DECODE_COMPLETE,
            generated_char_count = generated.len() as u64,
            terminal_eos_seen,
        );
        ArtifactDecodeResult {
            generated,
            decode_log,
            terminal_eos_seen,
        }
    }
}

/// Decode result for deterministic artifact argmax generation.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactDecodeResult {
    /// Generated normalized text characters, excluding terminal EOS.
    pub generated: TextCharSeq,
    /// Per-step argmax trace.
    pub decode_log: Vec<DecodeStep>,
    /// Whether decode observed EOS before returning.
    pub terminal_eos_seen: bool,
}

/// One deterministic decode step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecodeStep {
    /// Zero-based decode step.
    pub step: u32,
    /// Argmax token selected at this step.
    pub token: CharId,
    /// Maximum logit value at this step.
    pub logit_max: f32,
}
