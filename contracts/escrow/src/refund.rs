// Refund entrypoints are implemented in `contracts/escrow/src/lib.rs`.
// This module retains refund-related helpers only.

use soroban_sdk::{contracterror, contracttype, Env};

/// Errors surfaced by refund invariant checks.
///
/// These mirror the failure modes enforced by the refund entrypoints in
/// `lib.rs` so that off-chain callers can distinguish validation failures
/// from authorization failures without inspecting ledger state.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum RefundInvariantError {
    /// The requested refund amount is zero or negative.
    InvalidAmount = 1,
    /// The requested refund exceeds the escrow's remaining balance.
    AmountExceedsBalance = 2,
    /// The escrow is not in a state that permits refunds.
    InvalidState = 3,
    /// The caller is not authorized to trigger the refund.
    Unauthorized = 4,
    /// The refund has already been processed (idempotency guard).
    AlreadyRefunded = 5,
}

/// Lifecycle states relevant to refund transitions.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RefundState {
    /// Escrow is funded and refunds are permitted.
    Funded = 0,
    /// Escrow has been released to the beneficiary; refunds are forbidden.
    Released = 1,
    /// Escrow has been refunded; further refunds are forbidden.
    Refunded = 2,
    /// Escrow has been cancelled; refunds are forbidden.
    Cancelled = 3,
}

/// Validates the amount and remaining balance invariants for a refund.
///
/// Invariants enforced:
/// - `amount > 0`
/// - `amount <= remaining_balance`
///
/// Deterministic for all inputs; no state is mutated.
pub fn validate_refund_amount(
    amount: i128,
    remaining_balance: i128,
) -> Result<(), RefundInvariantError> {
    if amount <= 0 {
        return Err(RefundInvariantError::InvalidAmount);
    }
    if amount > remaining_balance {
        return Err(RefundInvariantError::AmountExceedsBalance);
    }
    Ok(())
}

/// Validates that a refund transition is permitted from `current`.
///
/// Only `Funded` may transition to `Refunded`. Any other state is rejected
/// so that repeated or out-of-order refunds cannot corrupt escrow state.
pub fn validate_refund_transition(
    current: RefundState,
) -> Result<(), RefundInvariantError> {
    match current {
        RefundState::Funded => Ok(()),
        RefundState::Refunded => Err(RefundInvariantError::AlreadyRefunded),
        RefundState::Released | RefundState::Cancelled => {
            Err(RefundInvariantError::InvalidState)
        }
    }
}

/// Emits a structured, non-sensitive diagnostic for a rejected refund.
///
/// Only the error discriminant is logged; amounts, addresses, and other
/// potentially sensitive values are intentionally omitted.
pub fn log_refund_rejection(env: &Env, err: RefundInvariantError) {
    env.events().publish(
        (soroban_sdk::symbol_short!("refund"), soroban_sdk::symbol_short!("reject")),
        err as u32,
    );
}
