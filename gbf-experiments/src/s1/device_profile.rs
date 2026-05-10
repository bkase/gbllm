//! Deterministic device profile enforcement for S1.
//!
//! This module validates the canonical profile and environment snapshot. Active
//! interception of GPU selection, network access, and host-clock reads is owned
//! by the S1 runner before tensor allocation.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::process;

use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};

use crate::s1::logging::{
    DeviceProfileEnforceFailEvent, DeviceProfileEnforceOkEvent, DeviceProfileEnforceStartEvent,
    RunPreconditionFailedEvent, S1LogEmitter,
};
use crate::s1::schema::{DomainHash, S1SchemaError};

/// Burn CPU backend label pinned by the S1 device profile.
pub const S1_CPU_BACKEND: &str = "burn-cpu-lockfile-pinned";

/// Exit code used by runner wrappers when device-profile enforcement fails.
pub const DEVICE_PROFILE_ENFORCEMENT_EXIT_CODE: i32 = 2;

/// Required S1 CPU deterministic process environment.
pub const S1_ENV_EXACT: &[(&str, &str)] = &[
    ("BURN_NDARRAY_NUM_THREADS", "1"),
    ("BURN_DETERMINISTIC", "1"),
    ("OMP_NUM_THREADS", "1"),
    ("RAYON_NUM_THREADS", "1"),
];

/// RFC §5/§16.4 deterministic CPU execution profile for S1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct S1CpuDeterministic {
    /// Burn CPU backend, pinned by the dependency lockfile.
    pub backend: String,
    /// Thread count allowed for deterministic replay.
    pub thread_count: u8,
    /// Whether deterministic reductions are required.
    pub deterministic_reductions: bool,
    /// Whether GPU devices may be selected.
    pub gpu_allowed: bool,
    /// Whether network access may be used by training artifacts.
    pub network_allowed: bool,
    /// Whether host-clock reads may influence training artifacts.
    pub host_clock_allowed_for_training_artifacts: bool,
    /// Exact process environment required before any tensor allocation.
    pub env_exact: BTreeMap<String, String>,
    /// Whether all variables outside [`Self::env_exact`] are forbidden.
    pub env_forbidden_unless_listed: bool,
}

impl Default for S1CpuDeterministic {
    fn default() -> Self {
        Self {
            backend: S1_CPU_BACKEND.to_owned(),
            thread_count: 1,
            deterministic_reductions: true,
            gpu_allowed: false,
            network_allowed: false,
            host_clock_allowed_for_training_artifacts: false,
            env_exact: S1_ENV_EXACT
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            env_forbidden_unless_listed: true,
        }
    }
}

impl S1CpuDeterministic {
    /// Construct the canonical S1 CPU deterministic profile.
    #[must_use]
    pub fn canonical() -> Self {
        Self::default()
    }
}

/// Successful proof that S1 device-profile checks ran before tensor allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceProfileEnforcement {
    device_profile_hash: Hash256,
}

impl DeviceProfileEnforcement {
    /// Hash of the enforced device profile for checkpoint metadata.
    #[must_use]
    pub const fn device_profile_hash(self) -> Hash256 {
        self.device_profile_hash
    }
}

/// Compute the canonical hash of an S1 deterministic device profile.
pub fn device_profile_hash(profile: &S1CpuDeterministic) -> Result<Hash256, S1SchemaError> {
    DomainHash::new(
        "gbf-experiments",
        "S1CpuDeterministic",
        "s1_device_profile.v1",
        "1.0.0",
    )
    .hash(profile)
}

/// Enforce the profile against the current process environment.
///
/// This check is shaped as a pre-tensor-allocation gate: callers should do no
/// Burn tensor construction until this function returns `Ok`.
pub fn enforce(
    profile: &S1CpuDeterministic,
) -> Result<DeviceProfileEnforcement, DeviceProfileEnforceError> {
    enforce_with_environment(profile, current_process_environment())
}

/// Enforce the profile and abort the process with a non-zero exit on failure.
///
/// This helper is for pre-subscriber, human-facing runner startup. JSON/logging
/// CLIs should call [`enforce`] directly and route the returned error through
/// their profile-safe diagnostic layer instead of writing raw stderr.
pub fn enforce_or_exit(profile: &S1CpuDeterministic) -> DeviceProfileEnforcement {
    match enforce(profile) {
        Ok(enforcement) => enforcement,
        Err(error) => {
            eprintln!("{error}");
            process::exit(DEVICE_PROFILE_ENFORCEMENT_EXIT_CODE);
        }
    }
}

/// Enforce the profile against an explicit environment snapshot.
pub fn enforce_with_environment<I, K, V>(
    profile: &S1CpuDeterministic,
    environment: I,
) -> Result<DeviceProfileEnforcement, DeviceProfileEnforceError>
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<OsString>,
    V: Into<OsString>,
{
    let profile_hash = device_profile_hash(profile)
        .expect("S1CpuDeterministic contains only finite canonical JSON scalars");
    let profile_hash_string = profile_hash.to_string();
    let emitter = S1LogEmitter::new();
    let _ = emitter.device_profile_enforce_start(&DeviceProfileEnforceStartEvent {
        device_profile_hash: profile_hash_string.clone(),
    });

    let (environment, mut violations) = normalize_environment(environment);
    violations.extend(collect_violations(profile, &environment));

    if violations.is_empty() {
        let _ = emitter.device_profile_enforce_ok(&DeviceProfileEnforceOkEvent {
            device_profile_hash: profile_hash_string,
        });
        Ok(DeviceProfileEnforcement {
            device_profile_hash: profile_hash,
        })
    } else {
        for violation in &violations {
            let _ = emitter.device_profile_enforce_fail(&DeviceProfileEnforceFailEvent {
                device_profile_hash: profile_hash_string.clone(),
                rejected_var: violation.rejected_var(),
                expected: violation.expected(),
                observed: violation.observed(),
            });
            let _ = emitter.run_precondition_failed(&RunPreconditionFailedEvent {
                seed: 0,
                reason: violation.to_string(),
            });
        }
        Err(DeviceProfileEnforceError { violations })
    }
}

/// Errors returned by S1 device-profile enforcement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceProfileEnforceError {
    violations: Vec<DeviceProfileViolation>,
}

impl DeviceProfileEnforceError {
    /// Ordered list of profile and environment violations.
    #[must_use]
    pub fn violations(&self) -> &[DeviceProfileViolation] {
        &self.violations
    }

    /// Exit code a runner should use for this precondition failure.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        DEVICE_PROFILE_ENFORCEMENT_EXIT_CODE
    }
}

impl fmt::Display for DeviceProfileEnforceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.violations.len();
        writeln!(
            f,
            "S1CpuDeterministic enforcement failed with {count} violation(s)"
        )?;
        for violation in &self.violations {
            writeln!(f, "- {violation}")?;
        }
        Ok(())
    }
}

impl Error for DeviceProfileEnforceError {}

/// A single S1 device-profile enforcement violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceProfileViolation {
    /// A profile field does not match the S1CpuDeterministic contract.
    ProfileMismatch {
        /// Field name.
        field: &'static str,
        /// Required value.
        expected: String,
        /// Observed value.
        observed: String,
    },
    /// A required environment variable is unset.
    MissingEnv {
        /// Environment variable name.
        var: String,
        /// Required value.
        expected: String,
    },
    /// A required environment variable has the wrong value.
    EnvMismatch {
        /// Environment variable name.
        var: String,
        /// Required value.
        expected: String,
        /// Observed value.
        observed: String,
    },
    /// An environment variable outside `env_exact` is set.
    ForbiddenEnv {
        /// Environment variable name.
        var: String,
        /// Observed value.
        observed: String,
    },
    /// An environment variable name is not valid Unicode.
    NonUnicodeEnvKey {
        /// Debug representation of the rejected key.
        key_debug: String,
    },
    /// An environment variable value is not valid Unicode.
    NonUnicodeEnvValue {
        /// Environment variable name.
        var: String,
        /// Debug representation of the rejected value.
        value_debug: String,
    },
}

impl DeviceProfileViolation {
    fn rejected_var(&self) -> String {
        match self {
            Self::ProfileMismatch { field, .. } => (*field).to_owned(),
            Self::MissingEnv { var, .. }
            | Self::EnvMismatch { var, .. }
            | Self::ForbiddenEnv { var, .. }
            | Self::NonUnicodeEnvValue { var, .. } => var.clone(),
            Self::NonUnicodeEnvKey { key_debug } => key_debug.clone(),
        }
    }

    fn expected(&self) -> String {
        match self {
            Self::ProfileMismatch { expected, .. }
            | Self::MissingEnv { expected, .. }
            | Self::EnvMismatch { expected, .. } => expected.clone(),
            Self::ForbiddenEnv { .. } => "unset".to_owned(),
            Self::NonUnicodeEnvKey { .. } | Self::NonUnicodeEnvValue { .. } => {
                "valid UTF-8".to_owned()
            }
        }
    }

    fn observed(&self) -> String {
        match self {
            Self::ProfileMismatch { observed, .. } | Self::EnvMismatch { observed, .. } => {
                observed.clone()
            }
            Self::MissingEnv { .. } => "<unset>".to_owned(),
            Self::ForbiddenEnv { observed, .. } => observed.clone(),
            Self::NonUnicodeEnvKey { key_debug } => key_debug.clone(),
            Self::NonUnicodeEnvValue { value_debug, .. } => value_debug.clone(),
        }
    }
}

impl fmt::Display for DeviceProfileViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProfileMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "profile field {field} expected {expected:?}, observed {observed:?}"
            ),
            Self::MissingEnv { var, expected } => {
                write!(
                    f,
                    "environment variable {var} expected {expected:?}, observed unset"
                )
            }
            Self::EnvMismatch {
                var,
                expected,
                observed,
            } => write!(
                f,
                "environment variable {var} expected {expected:?}, observed {observed:?}"
            ),
            Self::ForbiddenEnv { var, observed } => {
                write!(
                    f,
                    "environment variable {var} expected unset, observed {observed:?}"
                )
            }
            Self::NonUnicodeEnvKey { key_debug } => {
                write!(
                    f,
                    "environment variable key expected valid UTF-8, observed {key_debug}"
                )
            }
            Self::NonUnicodeEnvValue { var, value_debug } => {
                write!(
                    f,
                    "environment variable {var} expected valid UTF-8, observed {value_debug}"
                )
            }
        }
    }
}

fn normalize_environment<I, K, V>(
    environment: I,
) -> (BTreeMap<String, String>, Vec<DeviceProfileViolation>)
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<OsString>,
    V: Into<OsString>,
{
    let mut normalized = BTreeMap::new();
    let mut violations = Vec::new();

    for (key, value) in environment {
        let key = key.into();
        let key = match key.into_string() {
            Ok(key) => key,
            Err(key) => {
                violations.push(DeviceProfileViolation::NonUnicodeEnvKey {
                    key_debug: format!("{key:?}"),
                });
                continue;
            }
        };

        let value = value.into();
        let value = match value.into_string() {
            Ok(value) => value,
            Err(value) => {
                violations.push(DeviceProfileViolation::NonUnicodeEnvValue {
                    var: key,
                    value_debug: format!("{value:?}"),
                });
                continue;
            }
        };

        normalized.insert(key, value);
    }

    (normalized, violations)
}

fn collect_violations(
    profile: &S1CpuDeterministic,
    environment: &BTreeMap<String, String>,
) -> Vec<DeviceProfileViolation> {
    let mut violations = Vec::new();

    push_profile_mismatch(&mut violations, "backend", S1_CPU_BACKEND, &profile.backend);
    push_profile_mismatch(
        &mut violations,
        "thread_count",
        "1",
        &profile.thread_count.to_string(),
    );
    push_profile_mismatch(
        &mut violations,
        "deterministic_reductions",
        "true",
        &profile.deterministic_reductions.to_string(),
    );
    push_profile_mismatch(
        &mut violations,
        "gpu_allowed",
        "false",
        &profile.gpu_allowed.to_string(),
    );
    push_profile_mismatch(
        &mut violations,
        "network_allowed",
        "false",
        &profile.network_allowed.to_string(),
    );
    push_profile_mismatch(
        &mut violations,
        "host_clock_allowed_for_training_artifacts",
        "false",
        &profile
            .host_clock_allowed_for_training_artifacts
            .to_string(),
    );
    push_profile_mismatch(
        &mut violations,
        "env_forbidden_unless_listed",
        "true",
        &profile.env_forbidden_unless_listed.to_string(),
    );

    for (key, expected) in &profile.env_exact {
        match environment.get(key) {
            None => violations.push(DeviceProfileViolation::MissingEnv {
                var: key.clone(),
                expected: expected.clone(),
            }),
            Some(observed) if observed != expected => {
                violations.push(DeviceProfileViolation::EnvMismatch {
                    var: key.clone(),
                    expected: expected.clone(),
                    observed: observed.clone(),
                });
            }
            Some(_) => {}
        }
    }

    for (key, observed) in environment {
        if !profile.env_exact.contains_key(key) {
            violations.push(DeviceProfileViolation::ForbiddenEnv {
                var: key.clone(),
                observed: observed.clone(),
            });
        }
    }

    violations
}

fn push_profile_mismatch(
    violations: &mut Vec<DeviceProfileViolation>,
    field: &'static str,
    expected: &str,
    observed: &str,
) {
    if observed != expected {
        violations.push(DeviceProfileViolation::ProfileMismatch {
            field,
            expected: expected.to_owned(),
            observed: observed.to_owned(),
        });
    }
}

fn current_process_environment() -> impl Iterator<Item = (OsString, OsString)> {
    env::vars_os()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    use crate::s1::schema::S1CanonicalJson;

    #[test]
    fn default_profile_pins_rfc_values() {
        let profile = S1CpuDeterministic::canonical();

        assert_eq!(profile.backend, S1_CPU_BACKEND);
        assert_eq!(profile.thread_count, 1);
        assert!(profile.deterministic_reductions);
        assert!(!profile.gpu_allowed);
        assert!(!profile.network_allowed);
        assert!(!profile.host_clock_allowed_for_training_artifacts);
        assert!(profile.env_forbidden_unless_listed);
        assert_eq!(profile.env_exact.len(), S1_ENV_EXACT.len());
        for (key, value) in S1_ENV_EXACT {
            assert_eq!(
                profile.env_exact.get(*key).map(String::as_str),
                Some(*value)
            );
        }
    }

    #[test]
    fn canonical_profile_bytes_are_stable() {
        let bytes = S1CanonicalJson::to_vec(&S1CpuDeterministic::canonical()).unwrap();

        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "{\"backend\":\"burn-cpu-lockfile-pinned\",\"deterministic_reductions\":true,\"env_exact\":{\"BURN_DETERMINISTIC\":\"1\",\"BURN_NDARRAY_NUM_THREADS\":\"1\",\"OMP_NUM_THREADS\":\"1\",\"RAYON_NUM_THREADS\":\"1\"},\"env_forbidden_unless_listed\":true,\"gpu_allowed\":false,\"host_clock_allowed_for_training_artifacts\":false,\"network_allowed\":false,\"thread_count\":1}"
        );
    }

    #[test]
    fn profile_hash_matches_pinned_vector() {
        assert_eq!(
            device_profile_hash(&S1CpuDeterministic::canonical()).unwrap(),
            Hash256::from_str(
                "sha256:24a3f310d912f21f542d3eba8f42120fd835e964683c64dadc2515652888845d"
            )
            .unwrap()
        );
    }

    #[test]
    fn enforce_with_environment_rejects_missing_mismatch_and_extra_vars() {
        let profile = S1CpuDeterministic::canonical();

        let missing = enforce_with_environment(
            &profile,
            [
                ("BURN_NDARRAY_NUM_THREADS", "1"),
                ("OMP_NUM_THREADS", "1"),
                ("RAYON_NUM_THREADS", "1"),
            ],
        )
        .unwrap_err();
        assert!(matches!(
            missing.violations(),
            [DeviceProfileViolation::MissingEnv { var, expected }]
                if var == "BURN_DETERMINISTIC" && expected == "1"
        ));

        let mismatch = enforce_with_environment(
            &profile,
            [
                ("BURN_NDARRAY_NUM_THREADS", "1"),
                ("BURN_DETERMINISTIC", "1"),
                ("OMP_NUM_THREADS", "1"),
                ("RAYON_NUM_THREADS", "8"),
            ],
        )
        .unwrap_err();
        assert!(matches!(
            mismatch.violations(),
            [DeviceProfileViolation::EnvMismatch { var, expected, observed }]
                if var == "RAYON_NUM_THREADS" && expected == "1" && observed == "8"
        ));

        let extra = enforce_with_environment(
            &profile,
            [
                ("BURN_NDARRAY_NUM_THREADS", "1"),
                ("BURN_DETERMINISTIC", "1"),
                ("OMP_NUM_THREADS", "1"),
                ("RAYON_NUM_THREADS", "1"),
                ("RAYON_NUM_THREADS_HACK", "1"),
            ],
        )
        .unwrap_err();
        assert!(matches!(
            extra.violations(),
            [DeviceProfileViolation::ForbiddenEnv { var, observed }]
                if var == "RAYON_NUM_THREADS_HACK" && observed == "1"
        ));
    }

    #[test]
    fn enforcement_violation_display_strings_are_pinned() {
        assert_eq!(
            DeviceProfileViolation::MissingEnv {
                var: "BURN_DETERMINISTIC".to_owned(),
                expected: "1".to_owned(),
            }
            .to_string(),
            "environment variable BURN_DETERMINISTIC expected \"1\", observed unset"
        );
        assert_eq!(
            DeviceProfileViolation::EnvMismatch {
                var: "RAYON_NUM_THREADS".to_owned(),
                expected: "1".to_owned(),
                observed: "8".to_owned(),
            }
            .to_string(),
            "environment variable RAYON_NUM_THREADS expected \"1\", observed \"8\""
        );
        assert_eq!(
            DeviceProfileViolation::ForbiddenEnv {
                var: "RAYON_NUM_THREADS_HACK".to_owned(),
                observed: "1".to_owned(),
            }
            .to_string(),
            "environment variable RAYON_NUM_THREADS_HACK expected unset, observed \"1\""
        );
    }

    #[test]
    fn forbidden_env_scanning_is_unconditional_for_noncanonical_profile() {
        let mut profile = S1CpuDeterministic::canonical();
        profile.env_forbidden_unless_listed = false;

        let err = enforce_with_environment(
            &profile,
            [
                ("BURN_NDARRAY_NUM_THREADS", "1"),
                ("BURN_DETERMINISTIC", "1"),
                ("OMP_NUM_THREADS", "1"),
                ("RAYON_NUM_THREADS", "1"),
                ("RAYON_NUM_THREADS_HACK", "1"),
            ],
        )
        .unwrap_err();

        assert!(err.violations().iter().any(|violation| matches!(
            violation,
            DeviceProfileViolation::ProfileMismatch {
                field: "env_forbidden_unless_listed",
                expected,
                observed,
            } if expected == "true" && observed == "false"
        )));
        assert!(err.violations().iter().any(|violation| matches!(
            violation,
            DeviceProfileViolation::ForbiddenEnv { var, observed }
                if var == "RAYON_NUM_THREADS_HACK" && observed == "1"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_environment_keys_and_values_are_rejected() {
        use std::os::unix::ffi::OsStringExt;

        let invalid_key = OsString::from_vec(b"BAD_\xff_KEY".to_vec());
        let invalid_value = OsString::from_vec(b"BAD_\xff_VALUE".to_vec());
        let err = enforce_with_environment(
            &S1CpuDeterministic::canonical(),
            [
                (
                    OsString::from("BURN_NDARRAY_NUM_THREADS"),
                    OsString::from("1"),
                ),
                (OsString::from("BURN_DETERMINISTIC"), OsString::from("1")),
                (OsString::from("OMP_NUM_THREADS"), OsString::from("1")),
                (OsString::from("RAYON_NUM_THREADS"), invalid_value),
                (invalid_key, OsString::from("1")),
            ],
        )
        .unwrap_err();

        assert!(err.violations().iter().any(|violation| matches!(
            violation,
            DeviceProfileViolation::NonUnicodeEnvKey { key_debug }
                if key_debug.contains("BAD_")
        )));
        assert!(err.violations().iter().any(|violation| matches!(
            violation,
            DeviceProfileViolation::NonUnicodeEnvValue { var, value_debug }
                if var == "RAYON_NUM_THREADS" && value_debug.contains("BAD_")
        )));
    }

    #[test]
    fn enforce_with_environment_accepts_exact_env_and_returns_hash() {
        let enforcement = enforce_with_environment(
            &S1CpuDeterministic::canonical(),
            [
                ("BURN_NDARRAY_NUM_THREADS", "1"),
                ("BURN_DETERMINISTIC", "1"),
                ("OMP_NUM_THREADS", "1"),
                ("RAYON_NUM_THREADS", "1"),
            ],
        )
        .unwrap();

        assert_eq!(
            enforcement.device_profile_hash(),
            device_profile_hash(&S1CpuDeterministic::canonical()).unwrap()
        );
    }

    #[test]
    fn profile_flags_reject_hidden_inputs_and_parallelism() {
        let mut profile = S1CpuDeterministic::canonical();
        profile.gpu_allowed = true;
        profile.network_allowed = true;
        profile.host_clock_allowed_for_training_artifacts = true;
        profile.thread_count = 2;

        let err = enforce_with_environment(
            &profile,
            [
                ("BURN_NDARRAY_NUM_THREADS", "1"),
                ("BURN_DETERMINISTIC", "1"),
                ("OMP_NUM_THREADS", "1"),
                ("RAYON_NUM_THREADS", "1"),
            ],
        )
        .unwrap_err();

        assert!(err.violations().iter().any(|violation| matches!(
            violation,
            DeviceProfileViolation::ProfileMismatch {
                field: "gpu_allowed",
                ..
            }
        )));
        assert!(err.violations().iter().any(|violation| matches!(
            violation,
            DeviceProfileViolation::ProfileMismatch {
                field: "network_allowed",
                ..
            }
        )));
        assert!(err.violations().iter().any(|violation| matches!(
            violation,
            DeviceProfileViolation::ProfileMismatch {
                field: "host_clock_allowed_for_training_artifacts",
                ..
            }
        )));
        assert!(err.violations().iter().any(|violation| matches!(
            violation,
            DeviceProfileViolation::ProfileMismatch {
                field: "thread_count",
                ..
            }
        )));
    }

    #[test]
    fn enforcement_failures_have_nonzero_runner_exit_code() {
        let err = enforce_with_environment(
            &S1CpuDeterministic::canonical(),
            std::iter::empty::<(&str, &str)>(),
        )
        .unwrap_err();

        assert_ne!(err.exit_code(), 0);
        assert_eq!(err.exit_code(), DEVICE_PROFILE_ENFORCEMENT_EXIT_CODE);
    }
}
