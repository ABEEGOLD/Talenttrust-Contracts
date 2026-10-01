//! Backward-compatible refund helpers.
//!
//! The escrow ABI entrypoint lives on `crate::Escrow` and remains the
//! canonical implementation. This module intentionally exposes the historical
//! borrowed-`Env` helper shape so downstream Rust callers that imported
//! `escrow::refund::refund_unreleased_milestones` continue to compile after
//! the implementation moved into the contract entrypoint.
//!
//! Failure semantics are deliberately identical to the contract ABI:
//! validation/authentication errors panic with the same typed `EscrowError`,
//! and Soroban transaction rollback guarantees that no partial refund state is
//! persisted when the delegated call fails.

use crate::Escrow;
use soroban_sdk::{Env, Vec};

/// Compatibility wrapper around `Escrow::refund_unreleased_milestones`.
///
/// This wrapper performs no independent validation or state mutation. Keeping
/// a single validation/state-transition implementation is important: it
/// prevents the compatibility API and contract ABI from drifting and ensures a
/// retry observes exactly the same state/error until some other successful
/// transaction changes the escrow.
pub fn refund_unreleased_milestones(
    env: &Env,
    contract_id: u32,
    milestone_indices: &Vec<u32>,
) -> i128 {
    Escrow::refund_unreleased_milestones(env.clone(), contract_id, milestone_indices.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_signature_is_stable() {
        let _: fn(&Env, u32, &Vec<u32>) -> i128 = refund_unreleased_milestones;
    }
}
