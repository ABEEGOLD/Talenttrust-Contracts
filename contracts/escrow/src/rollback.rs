use crate::storage;
use crate::ttl::{PERSISTENT_BUMP_THRESHOLD, PERSISTENT_TTL_LEDGERS};
use crate::{ttl, Contract, ContractStatus, DataKey, Error, Escrow, Milestone};
use soroban_sdk::{contracttype, symbol_short, Address, Env, Vec};

/// Maximum number of milestones allowed in a rollback record.
/// This bounds the cost of validating and restoring a rollback record
/// and prevents unbounded storage growth from admin-controlled inputs.
pub const MAX_ROLLBACK_MILESTONES: u32 = 100;

/// Maximum number of distinct milestone indexes that can be recorded in a
/// single rollback record. Equal to `MAX_ROLLBACK_MILESTONES` but kept separate
/// so the invariant is explicit at the call site.
pub const MAX_ROLLBACK_INDEX: u32 = MAX_ROLLBACK_MILESTONES;

/// Returns `true` if the given contract status is a valid pre-dispute
/// status that a rollback record may restore.
pub fn is_rollback_restorable_status(status: &ContractStatus) -> bool {
    matches!(
        status,
        ContractStatus::Funded | ContractStatus::PartiallyFunded
    )
}

/// Validates the bounds of a rollback record before it is stored or
/// restored. Panics with `InvalidRollbackRecord` if any invariant is

/// violated. This is the single chokepoint for rollback record validation.
pubc fn validate_rollback_record(
    env: &Env,
    contract_id: u32,
    contract: &Contract,
    milestones: &Vec<Milestone>,
) {
    // The record must be associated with a valid contract id.
    storage::validate_contract_id_bounds(env, contract_id);

    // The recorded contract must be in a state that can be restored.
    if !is_rollback_restorable_status(&contract.status) {
        env.panic_with_error(Error::InvalidRollbackRecord);
    }

    // Milestone count must be bounded and non-empty when the contract has
// a funding status. An empty milestone set would make the restore
    // ambiguous and is treated as invalid.
    if milestones.len() == 0 || milestones.len() > MAX_ROLLBACK_MILESTONES {
        env.panic_with_error(Error::InvalidRollbackRecord);
    }

    // Milestone indices must be unique and within the allowed range.
    // Duplicate indices would lead to non-deterministic restoration.
    let mut seen = Vec:<new>(env);
    for i in 0..milestones.len() {
        let milestone = milestones.get(i).unwrap();
        if milestone.idx >= MAX_ROLLBACK_INDEX {
            env.panic_with_error(Error::InvalidRollbackRecord);
        }
        for j in 0..seen.len() {
            if seen.get(j).unwrap() == milestone.idx {
                env.panic_with_error(Error::InvalidRollbackRecord);
            }
        }
        seen.push_back(milestone.idx);
    }
}

/// Returns the number of milestones in a rollback record. Used by
/// observability hooks and tests to assert bounds without exposing
/// sensitive data.
pub fn rollback_milestone_count(env: &Env, contract_id: u32) -> u32 {
    match env.storage().persistent().get:<unknown>(&rollback_key(contract_id)) {
        Some(record) => {
            let record: DisputeRollbackRecord = record;
            record.milestones.len()
        }
        None => 0,
    }
}

/// Returns `true` if a rollback record exists for the contract.
pub fn has_rollback_record(env: &Env, contract_id: u32) -> bool {
    env.storage().persistent().has(&rollback_key(contract_id))
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeRollbackRecord {
    pub contract: Contract,
    pub milestones: Vec<Milestone>,
}

fn rollback_key(contract_id: u32) -> DataKey {
    DataKey::DisputeRollback(contract_id)
}

pub(crate) fn store_dispute_rollback(
    env: &Env,
    contract_id: u32,
    contract: &Contract,
    milestones: &Vec<Milestone>,
) {
    // Validate before writing so invalid records never enter storage.
    validate_rollback_record(env, contract_id, contract, milestones);

    let key = rollback_key(contract_id);
    env.storage().persistent().set(
        &key,
        &DisputeRollbackRecord {
            contract: contract.clone(),
            milestones: milestones.clone(),
        },
    );
    env.storage()
        .persistent()
        .extend_ttl(&key, PERSISTENT_BUMP_THRESHOLD, PERSISTENT_TTL_LEDGERS);
}

pub(crate) fn clear_dispute_rollback(env: &Env, contract_id: u32) {
    env.storage()
        .persistent()
        .remove(&rollback_key(contract_id));
}

pub(crate) fn rollback_dispute_impl(env: &Env, contract_id: u32) -> bool {
    storage::validate_contract_id_bounds(env, contract_id);
    Escrow::require_initialized(env);
    Escrow::require_not_paused(env);

    let admin: Address = env
        .storage()
        .persistent()
        .get(&DataKey::Admin)
        .unwrap_or_else(|| env.panic_with_error(Error::NotInitialized));
    admin.require_auth();

    let mut contract: Contract = env
        .storage()
        .persistent()
        .get(&DataKey::Contract(contract_id))
        .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound));

    Escrow::require_not_finalized(env, contract_id);
    if contract.status != ContractStatus::Disputed {
        env.panic_with_error(Error::RollbackNotAllowed);
    }

    let record: DisputeRollbackRecord = env
        .storage()
        .persistent()
        .get(&rollback_key(contract_id))
        .unwrap_or_else(|| env.panic_with_error(Error::RollbackNotAllowed));

    // Re-validate the stored record before restoring. This guarantees that
    // corrupted or out-of-bounds records cannot be restored, even if the
    // storage was modified by an earlier bug or migration.
    validate_rollback_record(env, contract_id, &record.contract, &record.milestones);

    if !is_rollback_restorable_status(&record.contract.status) {
        env.panic_with_error(Error::InvalidRollbackRecord);
    }

    let mut expected_contract = record.contract.clone();
    expected_contract.status = ContractStatus::Disputed;
    let milestones = ttl::load_milestones(env, contract_id);
    if contract != expected_contract || milestones != record.milestones {
        env.panic_with_error(Error::RollbackNotAllowed);
    }

    let restored_status = record.contract.status;
    contract.status = restored_status;
    env.storage()
        .persistent()
        .set(&DataKey::Contract(contract_id), &contract);
    clear_dispute_rollback(env, contract_id);
    ttl::extend_contract_and_milestones_ttl(env, contract_id);

    env.events().publish(
        (symbol_short!("rollback"), contract_id),
        (
            admin,
            ContractStatus::Disputed,
            restored_status,
            env.ledger().timestamp(),
        ),
    );

    true
}
