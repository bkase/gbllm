//! Compiler pipeline from artifact import through scheduling, assembly lowering, ROM emission, and reports.

pub mod arena;
pub mod f_b1;
pub mod import;
pub mod kernel_select;
pub mod legalize;
pub mod lower_asm;
pub mod lower_infer;
pub mod lower_quant;
pub mod observe;
pub mod place;
pub mod range;
pub mod reachability;
pub mod report;
pub mod rom;
pub mod schedule;
pub mod stage_cache;
pub mod storage;
pub mod validate;
pub mod window;
