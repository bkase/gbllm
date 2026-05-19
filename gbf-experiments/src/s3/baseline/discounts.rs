//! D-rule discount fitting for S3 modified Kneser-Ney.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::BaselineError;

/// Three-discount modified Kneser-Ney parameters for one order.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnDiscounts {
    /// Chen-Goodman Y term for this effective table.
    pub y_k: f64,
    /// Discount for count 1.
    pub d_1: f64,
    /// Discount for count 2.
    pub d_2: f64,
    /// Discount for counts 3 and above.
    pub d_3p: f64,
}

/// Fit discounts without an order label. Prefer `fit_discounts_for_order` in production paths.
pub fn fit_discounts(coc: &BTreeMap<u64, u64>) -> Result<KnDiscounts, BaselineError> {
    fit_discounts_for_order(0, coc)
}

/// Fit D-rule discounts for one interpolated order.
pub fn fit_discounts_for_order(
    order: u64,
    coc: &BTreeMap<u64, u64>,
) -> Result<KnDiscounts, BaselineError> {
    let n1 = *coc.get(&1).unwrap_or(&0);
    let n2 = *coc.get(&2).unwrap_or(&0);
    let n3 = *coc.get(&3).unwrap_or(&0);
    let n4 = *coc.get(&4).unwrap_or(&0);
    let missing = [(1, n1), (2, n2), (3, n3)]
        .into_iter()
        .filter_map(|(n, count)| (count == 0).then_some(n))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(BaselineError::DiscountPreconditionsViolated { order, missing });
    }

    let n1 = n1 as f64;
    let n2 = n2 as f64;
    let n3 = n3 as f64;
    let n4 = n4 as f64;
    let y_k = n1 / (n1 + 2.0 * n2);
    let d_1 = 1.0 - 2.0 * y_k * (n2 / n1);
    let d_2 = 2.0 - 3.0 * y_k * (n3 / n2);
    let d_3p = 3.0 - 4.0 * y_k * (n4 / n3);
    let discounts = KnDiscounts {
        y_k,
        d_1,
        d_2,
        d_3p,
    };
    check_bounds(order, "y_k", discounts.y_k, 0.0, 1.0)?;
    check_bounds(order, "d_1", discounts.d_1, 0.0, 1.0)?;
    check_bounds(order, "d_2", discounts.d_2, 0.0, 2.0)?;
    check_bounds(order, "d_3p", discounts.d_3p, 0.0, 3.0)?;
    Ok(discounts)
}

fn check_bounds(
    order: u64,
    field: &'static str,
    value: f64,
    lower: f64,
    upper: f64,
) -> Result<(), BaselineError> {
    let in_range = if field == "y_k" {
        value > lower && value < upper
    } else {
        value >= lower && value <= upper
    };
    if value.is_finite() && in_range {
        Ok(())
    } else {
        Err(BaselineError::DiscountOutOfBounds {
            order,
            field,
            value,
            lower,
            upper,
        })
    }
}
