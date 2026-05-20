//! F-S5 shadow compile artifact schema re-exports.

pub use gbf_policy::{
    H13_SHADOW_FINAL_STRICT_PASS_MAX_BYTES, H13_SHADOW_FINAL_WARNING_MAX_BYTES,
    H13ShadowFinalByteCostGap, H13ShadowFinalByteCostStatus, S5_SHADOW_CADENCE_STEPS,
    S5_SHADOW_COMPILE_SAMPLE_SCHEMA, S5_SHADOW_PIPELINE_STAGES, ShadowCompileSampleExpectation,
    ShadowCompileSampleReal, ShadowEmissionId, ShadowStep, Shr1ValidationError,
    h13_shadow_final_byte_cost_gap, h13_shadow_final_byte_cost_status,
    h13_shadow_sample_final_byte_cost_gap, shadow_compile_sample_real_emission_order,
    shadow_compile_sample_real_path, validate_shr1_shadow_sample,
};
