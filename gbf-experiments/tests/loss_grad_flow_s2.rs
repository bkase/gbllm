#![cfg(any(
    feature = "phase-a",
    feature = "ablation",
    feature = "s2-full",
    feature = "s2-ablation",
    feature = "s3-phase-d",
    feature = "falsify"
))]

mod common;

#[path = "loss_grad_flow_s2/h5_4b.rs"]
mod h5_4b;

#[cfg(feature = "s2-full")]
#[path = "loss_grad_flow_s2/h5_5.rs"]
mod h5_5;
