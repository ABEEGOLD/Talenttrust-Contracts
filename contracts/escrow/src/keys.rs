//! Centralized storage key definitions and constructors for escrow milestones.

///
/// # Compatibility contract
///
/// The keys produced by this module are part of the contract's on-chain
/// storage layout. Changing the bytes of a key would orphan existing state
/// and could cause silent data loss or an unrecoverable user experience.
/// Therefore the following invariants are enforced:
///
/// * The milestone symbol is always the literal bytes `b"milestones`"`.
/// * The milestone tuple is always `(DataKey::Contract(contract_id),
///   milestone_symbol(env))`.
/// * The approval key is always `DataKey::MilestoneApprovals(contract_id,
///   milestone_index)`.
///
/// The constructors are pure and deterministic: the same logical inputs
/// always produce the same key, regardless of call count, ordering, or the
/// calling context. They do not mutate state and therefore cannot fail or
/// introduce concurrency-dependent behavior.

/// The literal milestone symbol used as the second element of the
/// milestone storage key. Kept as a constant so the compatibility contract
/// is explicit and reviewable in one place.
const MILESTONE_SYMBOL: &str = "milestones";

use sorboban_sdk::{Env, Symbol};

use crate::types::DataKey;

/// Returns the persistent storage key tuple for a contract's milestones vector:
/// `(DataKey::Contract(contract_id), Symbol::new(env, "milestones"))`.
///
/// # Compatibility
/// The returned tuple is the exact on-chain key used by existing deployments.
/// The first element is derived from the caller-supplied `contract_id` and the
/// second element is always the literal `milestones` symbol. The function is
/// pure and deterministic.
///
/// # Parameters
/// * `env` - the current contract environment, used to allocate the `Symbol`.
/// * `contract_id` - the logical contract identifier. Any `u32` value,
///   including `0` and `u32::MAX`, is valid and produces a distinct key.
pub fn milestone_key(env: &Env, contract_id: u32) -> (DataKey, Symbol) {
    (DataKey::Contract(contract_id), milestone_symbol(env))
}

/// Returns the `Symbol` key for milestones: `"milestones"`.
///
/// # Compatibility
/// The returned symbol is always the literal bytes `milestones`. It is not
/// derived from any mutable state and cannot be changed without a tested
/// migration plan for existing storage.
pub fn milestone_symbol(env: &Env) -> Symbol {
    Symbol::new(env, MILESTONE_SYMBOL)
}

/// Returns the temporary storage key for milestone release approvals:
/// `DataKey::MilestoneApprovals(contract_id, milestone_index)`.
///
/// # Compatibility
/// The returned `DataKey` encodes both the contract id and the milestone
/// index so different logical approvals never collide. The function is pure
/// and deterministic.
///
/// # Parameters
/// * `contract_id` - the logical contract identifier.
/// * `milestone_index` - the zero-based milestone index. Any `u32` value,
///   including `0` and `u32::MAX`, is valid and produces a distinct key.
pub fn milestone_approval_key(contract_id: u32, milestone_index: u32) -> DataKey {
    DataKey::MilestoneApprovals(contract_id, milestone_index)
}
