//! Stage 6 `StoragePlan` types and helpers.

pub mod alias_engine;
pub mod cache;
pub mod diagnostics;
pub mod driver;
pub mod emitter;
pub mod invariants;
pub mod lifetime;
pub mod overlay_lens;
pub mod persist;
pub mod policy_view;
pub mod predicates;
pub mod rules;
pub mod types;

pub use alias_engine::*;
pub use cache::*;
pub use diagnostics::*;
pub use driver::*;
pub use emitter::*;
pub use invariants::*;
pub use lifetime::*;
pub use overlay_lens::*;
pub use persist::*;
pub use policy_view::*;
pub use predicates::*;
pub use rules::*;
pub use types::*;

#[cfg(test)]
mod tests {
    fn closed_key(parts: &[&str]) -> String {
        parts.concat()
    }

    #[test]
    fn sc11_static_scan_rejects_forbidden_storage_plan_type_surface_names() {
        let source = include_str!("types.rs");
        let forbidden = [
            closed_key(&["byte_", "offset"]),
            closed_key(&["byte_", "alignment"]),
            closed_key(&["byte_", "address"]),
            closed_key(&["concrete_", "bank"]),
            closed_key(&["rom_", "bank"]),
            closed_key(&["sram_", "bank"]),
            closed_key(&["slice_", "id"]),
            closed_key(&["lease_", "id"]),
            closed_key(&["overlay_", "region"]),
            closed_key(&["overlay_", "install"]),
            closed_key(&["page_", "byte_", "address"]),
            closed_key(&["kernel_", "residency"]),
            closed_key(&["SRAM page ", "family"]),
            closed_key(&["working-", "set"]),
            closed_key(&["Resource", "Vector"]),
            closed_key(&["Sched", "Slice"]),
            closed_key(&["Resid", "ency", "Epoch"]),
            closed_key(&["Overlay", "Id"]),
            closed_key(&["Overlay", "Install"]),
            closed_key(&["Kernel", "Resid", "ency"]),
            closed_key(&["Bank", "Class"]),
            closed_key(&["Resid", "ency"]),
        ];

        for key in forbidden {
            assert!(
                !source.contains(&key),
                "storage_plan::types contains forbidden SC11 type-surface name {key:?}"
            );
        }
    }
}
