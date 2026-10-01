use soroban_sdk::{contracttype, symbol_short, Address, Env, Vec};

use crate::{
    settlement, Contract, ContractStatus, ContractSummary, DataKey, Error, Escrow, EscrowError,
    Milestone, MilestoneSummary,
};

/// Immutable metadata written when an escrow contract is closed.
///
/// The record is stored once under `DataKey::Finalization(contract_id)`.
/// After it exists, all contract-specific mutating entrypoints reject with
/// `Error::AlreadyFinalized`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalizationRecord {
    /// Authorized client, freelancer, or assigned arbiter that finalized.
    pub finalizer: Address,
    /// Ledger timestamp at finalization time.
    pub timestamp: u64,
    /// Snapshot of participant, milestone, and accounting state.
    pub summary: ContractSummary,
}

/// Schema version for the finalization record layout. Bump when the
/// `FinalizationRecord` shape changes so off-chain consumers can detect
/// incompatible snapshots.
pub const FINALIZATION_SCHEMA_VERSION: u32 = 1;

impl Escrow {
    fn load_contract_for_finalization(env: &Env, contract_id: u32) -> Contract {
        crate::storage::validate_contract_id_bounds(env, contract_id);
        env.storage()
            .persistent()
            .get::<_, Contract>(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound))
    }

    /// Load a contract for finalization without extending TTL. Finalization
    /// is a terminal transition; the record itself is what must outlive the
    /// contract, so we do not refresh the contract TTL here.
    fn load_contract_for_finalization_checked(
        env: &Env,
        contract_id: u32,
    ) -> Contract {
        Self::load_contract_for_finalization(env, contract_id)
    }

    pub(crate) fn is_finalized(env: &Env, contract_id: u32) -> bool {
        settlement::is_finalized(env, contract_id)
    }

    pub(crate) fn require_not_finalized(env: &Env, contract_id: u32) {
        settlement::require_not_finalized(env, contract_id);
    }

    /// Returns true when the contract status is a terminal, non-mutable
    /// state that must never be resurrected by lifecycle entrypoints.
    pub(crate) fn is_terminal_status(status: ContractStatus) -> bool {
        matches!(
            status,
            ContractStatus::Cancelled | ContractStatus::Refunded
        )
    }

    /// Load a contract, verify it's in an active (mutable) state, and extend
    /// its TTL. Rejects `Cancelled`, `Refunded`, and finalized contracts.
    ///
    /// This is the canonical preamble for all lifecycle entrypoints that need a
    /// live, mutable contract. Calls `load_contract` from `storage.rs`, extends
    /// the TTL, checks finalization, and rejects terminal statuses.
    ///
    /// # Panics
    /// - `ContractNotFound` when `contract_id` is unknown.
    /// - `AlreadyFinalized` when the contract has been finalized.
    /// - `InvalidState` when the contract status is `Cancelled` or `Refunded`.
    ///
    /// # Returns
    /// The loaded `Contract`.
    pub(crate) fn require_active_contract(env: &Env, contract_id: u32) -> Contract {
        let contract = crate::storage::load_contract(env, contract_id);
        crate::ttl::extend_contract_ttl(env, contract_id);
        Self::require_not_finalized(env, contract_id);
        if Self::is_terminal_status(contract.status) {
            env.panic_with_error(Error::InvalidState);
        }
        contract
    }

    pub(crate) fn require_not_paused(env: &Env) {
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Paused)
            .unwrap_or(false)
        {
            env.panic_with_error(Error::ContractPaused);
        }
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Emergency)
            .unwrap_or(false)
        {
            env.panic_with_error(Error::EmergencyActive);
        }
    }

    fn require_finalizer_role(env: &Env, contract: &Contract, finalizer: &Address) {
        let is_client = *finalizer == contract.client;
        let is_freelancer = *finalizer == contract.freelancer;
        let is_arbiter = contract.arbiter.clone().is_some_and(|a| a == *finalizer);
        if !is_client && !is_freelancer && !is_arbiter {
            env.panic_with_error(Error::UnauthorizedRole);
        }
    }

    fn summarize_contract(env: &Env, contract_id: u32, contract: &Contract) -> ContractSummary {
        let milestone_key = crate::keys::milestone_key(env, contract_id);
        let milestones: Vec<Milestone> = env
            .storage()
            .persistent()
            .get(&milestone_key)
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

        // Invariant: milestone count must be non-zero for a finalized
        // contract. A zero-milestone contract cannot have a meaningful
        // accounting snapshot and indicates corrupted state.
        if milestones.is_empty() {
            env.panic_with_error(Error::InvalidState);
        }

        let mut total_amount: i128 = 0;
        let mut released_milestone_count: u32 = 0;
        let mut milestone_summaries = Vec::new(env);

        for (index, ms) in milestones.iter().enumerate() {
            let idx = index as u32;
            total_amount = total_amount
                .checked_add(ms.amount)
                .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));

            // Invariant: a milestone cannot be both released and refunded.
            // Allowing both would double-count funds in the summary and
            // break downstream accounting.
            if ms.released && ms.refunded {
                env.panic_with_error(Error::InvalidState);
            }

            if ms.released {
                released_milestone_count = released_milestone_count
                    .checked_add(1)
                    .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
            }

            milestone_summaries.push_back(MilestoneSummary {
                index: idx,
                amount: ms.amount,
                released: ms.released,
                refunded: ms.refunded,
            });
        }

        // Invariant: released + refunded amounts must never exceed the
        // funded amount. Violations indicate corrupted accounting state.
        let accounted = contract
            .released_amount
            .checked_add(contract.refunded_amount)
            .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
        if accounted > contract.funded_amount {
            env.panic_with_error(Error::InvalidState);
        }

        let refundable_balance = contract
            .funded_amount
            .checked_sub(accounted)
            .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));

        ContractSummary {
            schema_version: FINALIZATION_SCHEMA_VERSION,
            client: contract.client.clone(),
            freelancer: contract.freelancer.clone(),
            arbiter: contract.arbiter.clone(),
            status: contract.status,
            reputation_issued: contract.reputation_issued,
            total_amount,
            funded_amount: contract.funded_amount,
            released_amount: contract.released_amount,
            refundable_balance,
            released_milestone_count,
            milestones: milestone_summaries,
        }
    }
}

/// Finalize an escrow contract by writing immutable close metadata.
///
/// `finalizer` must authorize the call and must be the stored client,
/// freelancer, or assigned arbiter. Finalization is allowed only while the
/// contract is `Completed` or `Disputed`. Once finalized, future
/// contract-specific mutations fail with `AlreadyFinalized`.
///
/// # Errors
/// - `ContractPaused` when pause or emergency controls are active.
/// - `ContractNotFound` when `contract_id` is unknown.
/// - `AlreadyFinalized` when a close record already exists.
/// - `UnauthorizedRole` when `finalizer` is not a contract participant.
/// - `InvalidStatusTransition` unless status is `Completed` or `Disputed`.
pub fn finalize_contract_impl(env: &Env, contract_id: u32, finalizer: Address) -> bool {
    if Escrow::is_finalized(env, contract_id) {
        env.panic_with_error(Error::AlreadyFinalized);
    }

    let contract = Escrow::load_contract_for_finalization_checked(env, contract_id);
    if contract.status != ContractStatus::Completed && contract.status != ContractStatus::Disputed {
        env.panic_with_error(EscrowError::InvalidStatusTransition);
    }

    Escrow::require_not_paused(env);
    finalizer.require_auth();
    Escrow::require_finalizer_role(env, &contract, &finalizer);

    // Re-check finalization immediately before writing to close the
    // check-then-act window. Soroban executes transactions atomically, but
    // this guard makes the invariant explicit and protects against future
    // refactors that might introduce intermediate writes.
    Escrow::require_not_finalized(env, contract_id);

    let record = FinalizationRecord {
        finalizer: finalizer.clone(),
        timestamp: env.ledger().timestamp(),
        summary: Escrow::summarize_contract(env, contract_id, &contract),
    };

    let _ = settlement::commit_finalization(env, contract_id, &record)
        .unwrap_or_else(|error| env.panic_with_error(error));

    // Post-write invariant: the record must be readable and match the
    // finalizer we just wrote. This catches storage-layer regressions.
    let stored: FinalizationRecord = env
        .storage()
        .persistent()
        .get(&Escrow::finalization_key(contract_id))
        .unwrap_or_else(|| env.panic_with_error(Error::InvalidState));
    if stored.finalizer != finalizer {
        env.panic_with_error(Error::InvalidState);
    }

    if contract.status == ContractStatus::Disputed {
        crate::rollback::clear_dispute_rollback(env, contract_id);
    }

    env.events().publish(
        (symbol_short!("finalized"), contract_id),
        (finalizer, record.timestamp),
    );

    true
}

/// Return immutable close metadata for `contract_id`, if it has been finalized.
pub fn get_finalization_record_impl(env: &Env, contract_id: u32) -> Option<FinalizationRecord> {
    settlement::read_finalization(env, contract_id)
}
