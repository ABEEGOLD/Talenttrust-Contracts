//! Bounds validation for storage entrypoint inputs.
//!
//! This module extracts numeric and length bound checks for storage-mutating
//! entrypoints into a single source of truth. Each function validates one
//! logical parameter and panics with the appropriate typed [`EscrowError`]
//! on rejection.
//!
//! # Invariants
//!
//! * Every validator is pure: it performs no storage reads/writes and no
//!   external calls. Callers may invoke them in any order without side-effects.
//! * Every validator is total over its input domain: for any input it either
//!   returns `()` (accepted) or panics with a typed error (rejected). There is
//!   no third outcome and no silent clamping.
//! * Rejection is deterministic and idempotent: the same input always yields
//!   the same outcome, so retries and concurrent invocations cannot observe
//!   divergent validation results.
//! * Validators MUST be invoked before any state mutation in the corresponding
//!   entrypoint. Because they are pure, a rejected call leaves storage
//!   untouched — partial failure cannot produce inconsistent state.
//! * Panics carry only a typed error code; no user-supplied values are
//!   embedded in the panic payload, so failures are diagnosable without
//!   leaking sensitive data.
//!
//! All functions are pure (no side-effects) and intended to be called at the
//! top of the corresponding entrypoint, before any state mutation occurs.

use crate::milestones_consts::{
    MAX_FEE_BPS, MAX_MILESTONES, MAX_RATING, MAX_REPUTATION_CONFIG_COMMENT_BYTES_CEILING,
    MAX_REPUTATION_CONFIG_RATING_CEILING, MIN_COMMENT_BYTES, MIN_RATING,
};
use crate::{Error, EscrowError};
use soroban_sdk::Env;

/// Maximum number of milestones permitted in a single contract creation.
///
/// Re-exported here as a named constant so tests and callers can reference the
/// boundary without importing the underlying consts module. Kept in sync with
/// [`crate::milestones_consts::MAX_MILESTONES`].
pub(crate) const MILESTONE_COUNT_MAX: u32 = MAX_MILESTONES;

/// Validate the governed total escrow cap in stroops.
///
/// # Accepted values
/// * Any `i128` in `(0, i128::MAX]`.
///
/// # Rejected values
/// * `0` — a zero cap would block every contract creation.
/// * Negative values — amounts must be positive.
///
/// # Invariant
/// The accepted range is exactly `(0, i128::MAX]`; the upper bound is the
/// representable maximum and is therefore inclusive.
///
/// # Panics
/// Panics with [`Error::InvalidProtocolParameters`] when the cap is out
/// of range.
pub(crate) fn validate_escrow_total_cap(env: &Env, max_escrow_total_stroops: i128) {
    if max_escrow_total_stroops <= 0 {
        env.panic_with_error(Error::InvalidProtocolParameters);
    }
}

/// Validate reputation configuration parameters.
///
/// # Accepted values
/// * `min_rating` in `[1, 10]`
/// * `max_rating` in `[min_rating, 10]`
/// * `max_comment_bytes` in `[1, 1_000]`
///
/// # Invariant
/// `min_rating <= max_rating` is enforced before the ceiling check so that an
/// inverted range is rejected even when both values are individually in range.
/// `max_comment_bytes` is bounded on both ends; a zero-length comment is
/// rejected to avoid ambiguous empty-comment semantics.
///
/// # Panics
/// Panics with [`Error::InvalidProtocolParameters`] when any bound is violated.
pub(crate) fn validate_reputation_config_params(
    env: &Env,
    min_rating: u32,
    max_rating: u32,
    max_comment_bytes: u32,
) {
    if min_rating < MIN_RATING
        || max_rating < min_rating
        || max_rating > MAX_REPUTATION_CONFIG_RATING_CEILING
        || max_comment_bytes < MIN_COMMENT_BYTES
        || max_comment_bytes > MAX_REPUTATION_CONFIG_COMMENT_BYTES_CEILING
    {
        env.panic_with_error(Error::InvalidProtocolParameters);
    }
}

/// Validate the number of milestones for a contract creation call.
///
/// # Accepted values
/// * `count` in `[1, MAX_MILESTONES]`
///
/// # Rejected values
/// * `0` — at least one milestone is required.
/// * Values > `MAX_MILESTONES` (10).
///
/// # Invariant
/// The accepted range is exactly `[1, MAX_MILESTONES]`. `u32::MAX` is
/// rejected by the upper-bound check, so no overflow can occur downstream.
///
/// # Panics
/// Panics with [`EscrowError::EmptyMilestones`] when `count == 0` or
/// [`EscrowError::TooManyMilestones`] when `count > MAX_MILESTONES`.
pub(crate) fn validate_milestone_count(env: &Env, count: u32) {
    if count == 0 {
        env.panic_with_error(EscrowError::EmptyMilestones);
    }
    if count > MAX_MILESTONES {
        env.panic_with_error(EscrowError::TooManyMilestones);
    }
}

/// Validate a protocol fee basis-points value.
///
/// # Accepted values
/// * `bps` in `[0, MAX_FEE_BPS]` (0–10 000).
///
/// # Invariant
/// Zero is explicitly accepted (no fee). The upper bound is inclusive so
/// `MAX_FEE_BPS` is a valid configuration value.
///
/// # Panics
/// Panics with [`Error::InvalidProtocolParameters`] when `bps > MAX_FEE_BPS`.
pub(crate) fn validate_protocol_fee_bps(env: &Env, bps: u32) {
    if bps > MAX_FEE_BPS {
        env.panic_with_error(Error::InvalidProtocolParameters);
    }
}

/// Validate a single stroop amount for positivity and maximum bounds.
///
/// # Accepted values
/// * `amount` in `(0, MAX_SINGLE_AMOUNT_STROOPS]`.
///
/// # Invariant
/// Positivity is checked before the upper bound so that negative and zero
/// inputs produce [`EscrowError::AmountMustBePositive`] rather than the
/// less-specific [`EscrowError::InvalidMilestoneAmount`]. This ordering is
/// part of the observable contract and must not be reversed.
///
/// # Panics
/// Panics with [`EscrowError::AmountMustBePositive`] when `amount <= 0` or
/// [`EscrowError::InvalidMilestoneAmount`] when the amount exceeds the cap.
pub(crate) fn validate_stroop_amount(env: &Env, amount: i128) {
    if amount <= 0 {
        env.panic_with_error(crate::EscrowError::AmountMustBePositive);
    }
    if amount > crate::amount_validation::MAX_SINGLE_AMOUNT_STROOPS {
        env.panic_with_error(crate::EscrowError::InvalidMilestoneAmount);
    }
}

/// Validate a batch of stroop amounts, rejecting empty batches and any
/// individual amount that fails [`validate_stroop_amount`].
///
/// # Accepted values
/// * `amounts.len()` in `[1, MAX_MILESTONES]`
/// * every element in `(0, MAX_SINGLE_AMOUNT_STROOPS]`
///
/// # Invariant
/// Validation is all-or-nothing: if any element is rejected the function
/// panics before returning, so callers never observe a partially-validated
/// batch. Duplicate amounts are permitted (they are not semantically
/// duplicate submissions at this layer).
///
/// # Panics
/// Panics with [`EscrowError::EmptyMilestones`] for an empty batch,
/// [`EscrowError::TooManyMilestones`] for oversized batches, or the typed
/// error from [`validate_stroop_amount`] for an invalid element.
pub(crate) fn validate_stroop_amounts(env: &Env, amounts: &soroban_sdk::Vec<i128>) {
    let len = amounts.len();
    validate_milestone_count(env, len);
    for amount in amounts.iter() {
        validate_stroop_amount(env, amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    fn env() -> Env {
        Env::default()
    }

    // ── validate_escrow_total_cap ────────────────────────────────────────────

    #[test]
    fn validate_escrow_total_cap_accepts_1() {
        let e = env();
        validate_escrow_total_cap(&e, 1);
    }

    #[test]
    fn validate_escrow_total_cap_accepts_i128_max() {
        let e = env();
        validate_escrow_total_cap(&e, i128::MAX);
    }

    #[test]
    #[should_panic]
    fn validate_escrow_total_cap_rejects_zero() {
        let e = env();
        validate_escrow_total_cap(&e, 0);
    }

    #[test]
    #[should_panic]
    fn validate_escrow_total_cap_rejects_negative() {
        let e = env();
        validate_escrow_total_cap(&e, -1);
    }

    #[test]
    #[should_panic]
    fn validate_escrow_total_cap_rejects_i128_min() {
        let e = env();
        validate_escrow_total_cap(&e, i128::MIN);
    }

    // ── validate_reputation_config_params ─────────────────────────────────────

    #[test]
    fn validate_reputation_config_params_accepts_default() {
        let e = env();
        validate_reputation_config_params(&e, 1, 5, 200);
    }

    #[test]
    fn validate_reputation_config_params_accepts_min_equal_max_rating() {
        let e = env();
        validate_reputation_config_params(&e, 3, 3, 1);
    }

    #[test]
    fn validate_reputation_config_params_accepts_max_comment_1000() {
        let e = env();
        validate_reputation_config_params(&e, 1, 10, 1_000);
    }

    #[test]
    #[should_panic]
    fn validate_reputation_config_params_rejects_zero_min_rating() {
        let e = env();
        validate_reputation_config_params(&e, 0, 5, 200);
    }

    #[test]
    #[should_panic]
    fn validate_reputation_config_params_rejects_max_below_min() {
        let e = env();
        validate_reputation_config_params(&e, 5, 3, 200);
    }

    #[test]
    #[should_panic]
    fn validate_reputation_config_params_rejects_max_rating_over_10() {
        let e = env();
        validate_reputation_config_params(&e, 1, 11, 200);
    }

    #[test]
    #[should_panic]
    fn validate_reputation_config_params_rejects_zero_comment_bytes() {
        let e = env();
        validate_reputation_config_params(&e, 1, 5, 0);
    }

    #[test]
    #[should_panic]
    fn validate_reputation_config_params_rejects_comment_over_1000() {
        let e = env();
        validate_reputation_config_params(&e, 1, 5, 1_001);
    }

    // ── validate_milestone_count ──────────────────────────────────────────────

    #[test]
    fn validate_milestone_count_accepts_1() {
        let e = env();
        validate_milestone_count(&e, 1);
    }

    #[test]
    fn validate_milestone_count_accepts_max() {
        let e = env();
        validate_milestone_count(&e, MAX_MILESTONES);
    }

    #[test]
    #[should_panic]
    fn validate_milestone_count_rejects_zero() {
        let e = env();
        validate_milestone_count(&e, 0);
    }

    #[test]
    #[should_panic]
    fn validate_milestone_count_rejects_over_max() {
        let e = env();
        validate_milestone_count(&e, MAX_MILESTONES + 1);
    }

    #[test]
    #[should_panic]
    fn validate_milestone_count_rejects_u32_max() {
        let e = env();
        validate_milestone_count(&e, u32::MAX);
    }

    // ── validate_protocol_fee_bps ─────────────────────────────────────────────

    #[test]
    fn validate_protocol_fee_bps_accepts_zero() {
        let e = env();
        validate_protocol_fee_bps(&e, 0);
    }

    #[test]
    fn validate_protocol_fee_bps_accepts_max() {
        let e = env();
        validate_protocol_fee_bps(&e, MAX_FEE_BPS);
    }

    #[test]
    #[should_panic]
    fn validate_protocol_fee_bps_rejects_over_max() {
        let e = env();
        validate_protocol_fee_bps(&e, MAX_FEE_BPS + 1);
    }

    #[test]
    #[should_panic]
    fn validate_protocol_fee_bps_rejects_u32_max() {
        let e = env();
        validate_protocol_fee_bps(&e, u32::MAX);
    }

    // ── validate_stroop_amount ────────────────────────────────────────────────

    #[test]
    fn validate_stroop_amount_accepts_1() {
        let e = env();
        validate_stroop_amount(&e, 1);
    }

    #[test]
    fn validate_stroop_amount_accepts_max() {
        let e = env();
        validate_stroop_amount(&e, crate::amount_validation::MAX_SINGLE_AMOUNT_STROOPS);
    }

    #[test]
    #[should_panic]
    fn validate_stroop_amount_rejects_zero() {
        let e = env();
        validate_stroop_amount(&e, 0);
    }

    #[test]
    #[should_panic]
    fn validate_stroop_amount_rejects_negative() {
        let e = env();
        validate_stroop_amount(&e, -1);
    }

    #[test]
    #[should_panic]
    fn validate_stroop_amount_rejects_over_max() {
        let e = env();
        validate_stroop_amount(&e, crate::amount_validation::MAX_SINGLE_AMOUNT_STROOPS + 1);
    }

    // ── validate_stroop_amounts ───────────────────────────────────────────────

    #[test]
    fn validate_stroop_amounts_accepts_single() {
        let e = env();
        let mut v = soroban_sdk::Vec::new(&e);
        v.push_back(1);
        validate_stroop_amounts(&e, &v);
    }

    #[test]
    fn validate_stroop_amounts_accepts_max_length_and_boundary_values() {
        let e = env();
        let mut v = soroban_sdk::Vec::new(&e);
        v.push_back(1);
        v.push_back(crate::amount_validation::MAX_SINGLE_AMOUNT_STROOPS);
        for _ in 2..MAX_MILESTONES {
            v.push_back(1);
        }
        validate_stroop_amounts(&e, &v);
    }

    #[test]
    fn validate_stroop_amounts_accepts_duplicates() {
        let e = env();
        let mut v = soroban_sdk::Vec::new(&e);
        v.push_back(7);
        v.push_back(7);
        v.push_back(7);
        validate_stroop_amounts(&e, &v);
    }

    #[test]
    #[should_panic]
    fn validate_stroop_amounts_rejects_empty() {
        let e = env();
        let v: soroban_sdk::Vec<i128> = soroban_sdk::Vec::new(&e);
        validate_stroop_amounts(&e, &v);
    }

    #[test]
    #[should_panic]
    fn validate_stroop_amounts_rejects_over_max_length() {
        let e = env();
        let mut v = soroban_sdk::Vec::new(&e);
        for _ in 0..(MAX_MILESTONES + 1) {
            v.push_back(1);
        }
        validate_stroop_amounts(&e, &v);
    }

    #[test]
    #[should_panic]
    fn validate_stroop_amounts_rejects_zero_element() {
        let e = env();
        let mut v = soroban_sdk::Vec::new(&e);
        v.push_back(1);
        v.push_back(0);
        validate_stroop_amounts(&e, &v);
    }

    #[test]
    #[should_panic]
    fn validate_stroop_amounts_rejects_negative_element() {
        let e = env();
        let mut v = soroban_sdk::Vec::new(&e);
        v.push_back(-1);
        validate_stroop_amounts(&e, &v);
    }

    #[test]
    #[should_panic]
    fn validate_stroop_amounts_rejects_over_max_element() {
        let e = env();
        let mut v = soroban_sdk::Vec::new(&e);
        v.push_back(crate::amount_validation::MAX_SINGLE_AMOUNT_STROOPS + 1);
        validate_stroop_amounts(&e, &v);
    }
}
