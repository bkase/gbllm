//! Kernel specs, calling conventions, compatibility identifiers, implementations, and autotune knobs.
//!
//! Current contents: the ternary matvec bake-off substrate (bd-rzq5n) —
//! [`spec`] shapes/weights/packings, [`ref_impl`] exact reference, and
//! [`asm_impl`] ROM-emitting kernel builders. The durable `KernelSpec`
//! contract is owned by F-H1 (bd-2f32); production kernel families by F-H2
//! (bd-3se9).

pub mod asm_impl;
pub mod autotune;
pub mod calibration;
pub mod compat;
pub mod ref_impl;
pub mod signature;
pub mod spec;
