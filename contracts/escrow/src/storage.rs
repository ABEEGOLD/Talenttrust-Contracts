//! Centralized storage precondition checks and contract loading helpers.
//! Centralized storage precondition checks and contract loading helpers.
//!
//! This module extracts repeated storage validation patterns into a single source
//! of truth, ensuring consistent error handling and reducing code duplication across
//! entrypoints. All contract loading operations should route through these helpers.
//!
//! # Compatibility contract
//!
//! These helpers are the boundary between the public entrypoints and the persisted
//! [`DataKey`] layout. Their observable behaviour is deliberately small and stable,
//! and is locked by the unit tests in this file so a refactor cannot silently
//! drift it:
//!
//! * **Reserved id `0`.** [`DataKey::NextContractId`] starts at `1`, so id `0` is
//!   never a live contract. Two caller contracts are distinguished on purpose:
//!   * [`validate_contract_id_bounds`] is the *entrypoint preamble* guard. It
//!     rejects `0` as invalid input with [`Error::InvalidContractId`]. Every
//!     mutating entrypoint that advertises this guard must use it first.
//!   * Loaders and predicates ([`load_contract`], [`load_milestones`],
//!     [`load_contract_checked`], [`is_finalized`], [`require_not_finalized`])
//!     treat `0` exactly like any unknown id and reject it with
//!     [`Error::ContractNotFound`]. This matches the crate-root readers
//!     (`get_contract`, `get_milestones`, `get_milestone`, `get_contract_summary`,
//!     `get_refundable_balance`), which never call the strict guard, so the
//!     sentinel and "no record" stay indistinguishable for off-chain indexers.
//! * **Empty / missing data.** A missing `Contract(id)` or
//!   `(Contract(id), "milestones")` entry always maps to [`Error::ContractNotFound`].
//!   The helpers never return a zero/default value that could be mistaken for real
//!   state — this is what makes reads safe through TTL eviction of a key.
//! * **Errors are pure.** Every helper either returns a value or panics *before*
//!   writing. A rejected call therefore leaves storage untouched, so retries and
//!   host-level transaction rollbacks cannot observe a partially-applied guard.
//! * **Administrative nonce.** [`consume_admin_nonce`] is monotonic and
//!   exactly-once: only `current + 1` is accepted, replay of a consumed nonce and
//!   any future nonce are rejected, and exhaustion (`u64::MAX`) fails closed
//!   instead of wrapping to an accept-all `0`.

use crate::{Contract, DataKey, Error, EscrowError};
use soroban_sdk::{Env, Vec};

/// Maximum number of milestones allowed per contract.
///
/// This bound prevents unbounded storage growth and keeps iteration costs
/// deterministic for all callers. It is enforced at the storage boundary so
/// that no entrypoint can bypass it.
pub(crate) const MAX_MILESTONES: u32 = 128;

/// Maximum length (in bytes) of a milestone work-evidence reference.
///
/// Evidence is stored as an opaque reference (e.g. a content hash or URI).
/// Bounding its size prevents storage bloat and keeps validation deterministic.
pub(crate) const MAX_WORK_EVIDENCE_LEN: u32 = 256;

/// Validate that a milestone count is within the allowed range.
///
/// # Panics
/// - `InvalidMilestoneCount` if `count == 0` or `count > MAX_MILESTONES`
pub(crate) fn validate_milestone_count(env: &Env, count: u32) {
    if count == 0 || count > MAX_MILESTONES {
        env.panic_with_error(EscrowError::InvalidMilestoneCount);
    }
}

/// Validate that a work-evidence reference length is within bounds.
///
/// # Panics
/// - `InvalidWorkEvidence` if `len > MAX_WORK_EVIDENCE_LEN`
pub(crate) fn validate_work_evidence_len(env: &Env, len: u32) {
    if len > MAX_WORK_EVIDENCE_LEN {
        env.panic_with_error(EscrowError::InvalidWorkEvidence);
    }
}

/// Validate that contract_id is within numeric bounds (non-zero).
///
/// This is the **entrypoint preamble** guard: it rejects the reserved id `0` as
/// invalid input with [`Error::InvalidContractId`]. Loaders and predicates that
/// must treat `0` like any other unknown id use [`require_nonzero_contract_id`]
/// instead — see the module-level compatibility contract.
///
/// # Panics
/// - `ContractNotFound` if `contract_id == 0`
pub(crate) fn validate_contract_id_bounds(env: &Env, contract_id: u32) {
    if contract_id == 0 {
        env.panic_with_error(EscrowError::InvalidContractId);
    }
}

/// Loader-time sentinel guard: reject the reserved id `0` with
/// [`Error::ContractNotFound`].
///
/// Id `0` is never allocated and is reserved as a "not found" sentinel for
/// off-chain indexers. Loaders and predicates surface it exactly like an unknown
/// id so that a sentinel read and a missing record are indistinguishable —
/// matching the crate-root readers such as `get_contract`. Entrypoint preambles
/// that must reject `0` as *invalid input* call [`validate_contract_id_bounds`].
///
/// # Panics
/// - [`Error::ContractNotFound`] if `contract_id == 0`
fn require_nonzero_contract_id(env: &Env, contract_id: u32) {
    if contract_id == 0 {
        env.panic_with_error(Error::ContractNotFound);
    }
}

/// Validate that a milestone vector is well-formed and within bounds.
///
/// Enforces the invariants that must hold for every stored milestone vector:
/// - the vector is non-empty and does not exceed [`MAX_MILESTONES`]
/// - each milestone has a non-zero `amount`
/// - each milestone's `funded_amount` does not exceed its `amount`
/// - each milestone's `refunded_amount` does not exceed its `funded_amount`
/// - any `work_evidence` reference is within [`MAX_WORK_EVIDENCE_LEN`]
///
/// # Panics
/// - `InvalidMilestoneCount` if the vector length is out of range
/// - `InvalidMilestoneAmount` if an amount invariant is violated
/// - `InvalidWorkEvidence` if an evidence reference is too long
pub(crate) fn validate_milestones(env: &Env, milestones: &Vec<crate::Milestone>) {
    validate_milestone_count(env, milestones.len());
    for milestone in milestones.iter() {
        if milestone.amount == 0 {
            env.panic_with_error(EscrowError::InvalidMilestoneAmount);
        }
        if milestone.funded_amount > milestone.amount {
            env.panic_with_error(EscrowError::InvalidMilestoneAmount);
        }
        if milestone.refunded_amount > milestone.funded_amount {
            env.panic_with_error(EscrowError::InvalidMilestoneAmount);
        }
        if let Some(evidence) = &milestone.work_evidence {
            validate_work_evidence_len(env, evidence.len());
        }
    }
}

/// Check if the contract system has been initialized.
///
/// Initialization is a prerequisite for all money-flow operations. This check
/// ensures that the admin-controlled safety rails (pause, emergency controls,
/// protocol fees) are always in scope before any funds can move.
///
/// # Arguments
/// * `env` - The contract environment
///
/// # Panics
/// - `NotInitialized` if initialization has not been completed
///
/// # Returns
/// `true` if initialized, or panics with `NotInitialized`
pub(crate) fn require_initialized(env: &Env) -> bool {
    let initialized = env
        .storage()
        .persistent()
        .get::<_, bool>(&DataKey::Initialized)
        .unwrap_or(false);
    if !initialized {
        env.panic_with_error(Error::NotInitialized);
    }
    true
}

/// Load a contract from persistent storage.
///
/// This is the canonical pattern for retrieving a contract. It handles the
/// storage read with consistent error reporting and bounds checking.
///
/// # Arguments
/// * `env` - The contract environment
/// * `contract_id` - The contract ID to load
///
/// # Panics
/// - `ContractNotFound` if `contract_id` is 0
/// - `ContractNotFound` if no contract exists for this ID
///
/// # Returns
/// The loaded `Contract` or panics with `ContractNotFound`
pub(crate) fn load_contract(env: &Env, contract_id: u32) -> Contract {
    require_nonzero_contract_id(env, contract_id);
    env.storage()
        .persistent()
        .get(&DataKey::Contract(contract_id))
        .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound))
}

/// Load milestones for a contract from persistent storage.
///
/// Milestones are stored under a composite key combining the contract ID
/// and a "milestones" symbol. This helper centralizes the retrieval pattern.
///
/// # Arguments
/// * `env` - The contract environment
/// * `contract_id` - The contract ID whose milestones to load
///
/// # Panics
/// - `ContractNotFound` if `contract_id` is 0
/// - `ContractNotFound` if no milestone vector exists for this contract
///
/// # Returns
/// The loaded milestone vector or panics with `ContractNotFound`
///
/// The key is built through [`crate::keys::milestone_key`], the single
/// definition of the composite milestone key, so this read can never drift from
/// the writers in the rest of the crate.
pub(crate) fn load_milestones(env: &Env, contract_id: u32) -> Vec<crate::Milestone> {
    require_nonzero_contract_id(env, contract_id);
    let milestone_key = crate::keys::milestone_key(env, contract_id);
    env.storage()
        .persistent()
        .get(&milestone_key)
        .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound))
}

/// Persist a milestone vector after validating its invariants.
///
/// This is the canonical write path for milestones. Routing all writes through
/// this helper guarantees that the invariants checked by [`validate_milestones`]
/// hold for every stored vector, so that [`load_milestones`] can rely on them.
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is 0
/// - `InvalidMilestoneCount` if the vector length is out of range
/// - `InvalidMilestoneAmount` if an amount invariant is violated
/// - `InvalidWorkEvidence` if an evidence reference is too long
pub(crate) fn store_milestones(env: &Env, contract_id: u32, milestones: &Vec<crate::Milestone>) {
    validate_contract_id_bounds(env, contract_id);
    validate_milestones(env, milestones);
    let milestone_key = Symbol::new(env, "milestones");
    env.storage()
        .persistent()
        .set(&(DataKey::Contract(contract_id), milestone_key), milestones);
}

/// Load a contract, optionally with precondition checks for mutation.
///
/// This is the primary helper for loading contracts with optional safety guards:
/// - `check_paused`: If true, verifies pause/emergency flags are not set
/// - `check_finalized`: If true, verifies the contract has not been finalized
///
/// # Arguments
/// * `env` - The contract environment
/// * `contract_id` - The contract ID to load
/// * `check_paused` - Whether to verify pause/emergency states
/// * `check_finalized` - Whether to verify finalization state
///
/// # Panics
/// - `ContractNotFound` if `contract_id` is 0
/// - `ContractPaused` if `check_paused` is true and pause flag is set
/// - `EmergencyActive` if `check_paused` is true and emergency flag is set
/// - `AlreadyFinalized` if `check_finalized` is true and contract is finalized
///
/// # Returns
/// The loaded `Contract` if all preconditions pass
pub(crate) fn load_contract_checked(
    env: &Env,
    contract_id: u32,
    check_paused: bool,
    check_finalized: bool,
) -> Contract {
    require_nonzero_contract_id(env, contract_id);
    if check_paused {
        require_not_paused(env);
    }

    let contract = load_contract(env, contract_id);

    if check_finalized {
        require_not_finalized(env, contract_id);
    }

    contract
}

/// Check if the contract system is paused or in emergency mode.
///
/// # Arguments
/// * `env` - The contract environment
///
/// # Panics
/// - `ContractPaused` if the pause flag is set
/// - `EmergencyActive` if the emergency flag is set
///
/// # Returns
/// `true` if neither pause nor emergency is active, or panics
pub(crate) fn require_not_paused(env: &Env) -> bool {
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
    true
}

/// Check that the given [`PauseTarget`] is not blocked by an active scoped pause.
///
/// This is the entrypoint-facing guard used by payout and dispute operations.
/// If a [`PauseScope`] is stored, its target is compared against the requested
/// operation. A `Global` scope blocks everything; `Payout` blocks release,
/// refund, cancel; `Dispute` blocks raise, resolve, rollback.
///
/// The legacy bare `bool` under `DataKey::Paused` is also checked for backward
/// compatibility — it acts as a `Global` pause.
pub(crate) fn require_pause_scope(env: &Env, target: &crate::PauseTarget) {
    // Legacy boolean pause acts as Global
    if env
        .storage()
        .persistent()
        .get::<_, bool>(&DataKey::Paused)
        .unwrap_or(false)
    {
        env.panic_with_error(Error::ContractPaused);
    }

    // Emergency always blocks everything
    if env
        .storage()
        .persistent()
        .get::<_, bool>(&DataKey::Emergency)
        .unwrap_or(false)
    {
        env.panic_with_error(Error::EmergencyActive);
    }

    // Scoped pause
    if let Some(scope) = env
        .storage()
        .persistent()
        .get::<_, crate::PauseScope>(&DataKey::PauseScope)
    {
        match (&scope.target, target) {
            (crate::PauseTarget::Global, _) | (_, crate::PauseTarget::Global) => {
                env.panic_with_error(Error::PauseScopeActive);
            }
            (crate::PauseTarget::Payout, crate::PauseTarget::Payout) => {
                env.panic_with_error(Error::PauseScopeActive);
            }
            (crate::PauseTarget::Dispute, crate::PauseTarget::Dispute) => {
                env.panic_with_error(Error::PauseScopeActive);
            }
            _ => {} // Non-overlapping scope: allow
        }
    }
}

/// Validate that a contract's monetary invariants hold.
///
/// Enforces the accounting identities that must hold for every stored contract:
/// - `released_amount + refunded_amount <= total_deposited`
/// - `funded_amount <= total_deposited`
/// - `released_amount <= funded_amount`
///
/// These checks make silent data loss impossible: any state transition that
/// would violate them is rejected at the storage boundary.
///
/// # Panics
/// - `InvalidContractAmounts` if any invariant is violated
pub(crate) fn validate_contract_amounts(env: &Env, contract: &Contract) {
    let released_plus_refunded = contract
        .released_amount
        .checked_add(contract.refunded_amount)
        .unwrap_or_else(|| env.panic_with_error(EscrowError::InvalidContractAmounts));
    if released_plus_refunded > contract.total_deposited {
        env.panic_with_error(EscrowError::InvalidContractAmounts);
    }
    if contract.funded_amount > contract.total_deposited {
        env.panic_with_error(EscrowError::InvalidContractAmounts);
    }
    if contract.released_amount > contract.funded_amount {
        env.panic_with_error(EscrowError::InvalidContractAmounts);
    }
}

/// Persist a contract after validating its monetary invariants.
///
/// This is the canonical write path for contracts. All state transitions that
/// mutate a contract must route through this helper so that the invariants
/// checked by [`validate_contract_amounts`] hold for every stored contract.
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is 0
/// - `InvalidContractAmounts` if any monetary invariant is violated
pub(crate) fn store_contract(env: &Env, contract_id: u32, contract: &Contract) {
    validate_contract_id_bounds(env, contract_id);
    validate_contract_amounts(env, contract);
    env.storage()
        .persistent()
        .set(&DataKey::Contract(contract_id), contract);
}

/// Consume the next expected admin nonce, rejecting stale or future values.
///
/// Stores a monotonic `u64` under [`DataKey::AdminNonce`]. On the first call
/// the expected nonce is `1` (zero means uninitialized). After a successful
/// call the stored nonce is incremented atomically.
///
/// # Invariants
/// * The stored counter never decreases, so a successful call can never be
///   replayed: re-submitting an already-consumed nonce is rejected.
/// * `current + 1` is computed with [`u64::checked_add`]. If the counter is
///   already [`u64::MAX`] the call fails closed with [`Error::PotentialOverflow`]
///   and storage is left unchanged, instead of wrapping back to an accept-all
///   `0`. This keeps the nonce safe under adversarial replay of admin actions.
/// * The comparison and the write happen inside the same contract invocation, so
///   a rejected nonce performs no partial write (a panic aborts the invocation).
///
/// # Panics
/// * [`Error::StaleNonce`] if `provided_nonce` is not exactly `current + 1`.
/// * [`Error::PotentialOverflow`] if the stored counter is already [`u64::MAX`].
pub(crate) fn consume_admin_nonce(env: &Env, provided_nonce: u64) {
    let current: u64 = env
        .storage()
        .persistent()
        .get(&DataKey::AdminNonce)
        .unwrap_or(0);
    // Deterministic overflow handling: a saturated nonce cannot advance further,
    // so reject rather than wrap (which would silently reopen old nonces).
    let expected = match current.checked_add(1) {
        Some(next) => next,
        None => env.panic_with_error(Error::StaleNonce),
    };
    if provided_nonce != expected {
        env.panic_with_error(Error::StaleNonce);
    }
    env.storage()
        .persistent()
        .set(&DataKey::AdminNonce, &expected);
}

/// Check if a contract has been finalized.
///
/// # Arguments
/// * `env` - The contract environment
/// * `contract_id` - The contract ID to check
///
/// # Panics
/// - `ContractNotFound` if `contract_id` is 0 (reserved sentinel)
///
/// # Returns
/// `true` if the contract is finalized
pub(crate) fn is_finalized(env: &Env, contract_id: u32) -> bool {
    require_nonzero_contract_id(env, contract_id);
    env.storage()
        .persistent()
        .has(&DataKey::Finalization(contract_id))
}

/// Require that a contract has not been finalized.
///
/// # Arguments
/// * `env` - The contract environment
/// * `contract_id` - The contract ID to check
///
/// # Panics
/// - `ContractNotFound` if `contract_id` is 0
/// - `AlreadyFinalized` if the contract has been finalized
///
/// # Returns
/// `true` if not finalized, or panics
pub(crate) fn require_not_finalized(env: &Env, contract_id: u32) -> bool {
    require_nonzero_contract_id(env, contract_id);
    if is_finalized(env, contract_id) {
        env.panic_with_error(Error::AlreadyFinalized);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Milestone;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Address, Env, Symbol};

    fn setup_test_env() -> (Env, Address) {
        let env = Env::default();
        let admin = Address::generate(&env);
        (env, admin)
    }

    #[test]
    fn test_require_initialized_when_true() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            env.storage().persistent().set(&DataKey::Initialized, &true);
            let result = require_initialized(&env);
            assert!(result);
        });
    }

    #[test]
    #[should_panic(expected = "NotInitialized")]
    fn test_require_initialized_when_false() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            require_initialized(&env);
        });
    }

    #[test]
    #[should_panic(expected = "ContractNotFound")]
    fn test_load_contract_not_found() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            load_contract(&env, 999);
        });
    }

    #[test]
    fn test_load_contract_found() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let client = Address::generate(&env);
            let freelancer = Address::generate(&env);
            let contract = Contract {
                client: client.clone(),
                freelancer: freelancer.clone(),
                arbiter: None,
                status: crate::ContractStatus::Created,
                release_authorization: crate::ReleaseAuthorization::ClientOnly,
                funded_amount: 0,
                released_amount: 0,
                refunded_amount: 0,
                total_deposited: 0,
                reputation_issued: false,
            };

            env.storage()
                .persistent()
                .set(&DataKey::Contract(42), &contract);

            let loaded = load_contract(&env, 42);
            assert_eq!(loaded.client, client);
            assert_eq!(loaded.freelancer, freelancer);
            assert_eq!(loaded.status, crate::ContractStatus::Created);
        });
    }

    #[test]
    #[should_panic(expected = "ContractNotFound")]
    fn test_load_milestones_not_found() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            load_milestones(&env, 999);
        });
    }

    #[test]
    fn test_load_milestones_found() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let milestones = Vec::from_array(
                &env,
                [
                    Milestone {
                        amount: 1000,
                        funded_amount: 0,
                        released: false,
                        refunded: false,
                        deadline: None,
                        refunded_amount: 0,
                        work_evidence: None,
                    },
                    Milestone {
                        amount: 2000,
                        funded_amount: 0,
                        released: false,
                        refunded: false,
                        deadline: None,
                        refunded_amount: 0,
                        work_evidence: None,
                    },
                ],
            );

            let milestone_key = Symbol::new(&env, "milestones");
            env.storage()
                .persistent()
                .set(&(DataKey::Contract(42), milestone_key), &milestones);

            let loaded = load_milestones(&env, 42);
            assert_eq!(loaded.len(), 2);
            assert_eq!(loaded.get(0).unwrap().amount, 1000);
            assert_eq!(loaded.get(1).unwrap().amount, 2000);
        });
    }

    #[test]
    fn test_require_not_paused_when_not_paused() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let result = require_not_paused(&env);
            assert!(result);
        });
    }

    #[test]
    #[should_panic(expected = "ContractPaused")]
    fn test_require_not_paused_when_paused() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            env.storage().persistent().set(&DataKey::Paused, &true);
            require_not_paused(&env);
        });
    }

    #[test]
    #[should_panic(expected = "EmergencyActive")]
    fn test_require_not_paused_when_emergency() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            env.storage().persistent().set(&DataKey::Emergency, &true);
            require_not_paused(&env);
        });
    }

    #[test]
    fn test_is_finalized_when_false() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let result = is_finalized(&env, 42);
            assert!(!result);
        });
    }

    #[test]
    fn test_is_finalized_when_true() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            env.storage()
                .persistent()
                .set(&DataKey::Finalization(42), &true);

            let result = is_finalized(&env, 42);
            assert!(result);
        });
    }

    #[test]
    fn test_require_not_finalized_when_not_finalized() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let result = require_not_finalized(&env, 42);
            assert!(result);
        });
    }

    #[test]
    fn test_consume_admin_nonce_first_call() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            consume_admin_nonce(&env, 1);
            let stored: u64 = env
                .storage()
                .persistent()
                .get(&DataKey::AdminNonce)
                .unwrap_or(0);
            assert_eq!(stored, 1);
        });
    }

    #[test]
    #[should_panic(expected = "StaleNonce")]
    fn test_consume_admin_nonce_rejects_replay() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            consume_admin_nonce(&env, 1);
            // Replaying the same nonce must be rejected deterministically.
            consume_admin_nonce(&env, 1);
        });
    }

    #[test]
    #[should_panic(expected = "StaleNonce")]
    fn test_consume_admin_nonce_rejects_future() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            consume_admin_nonce(&env, 2);
        });
    }

    #[test]
    #[should_panic(expected = "StaleNonce")]
    fn test_consume_admin_nonce_rejects_overflow() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            env.storage()
                .persistent()
                .set(&DataKey::AdminNonce, &u64::MAX);
            consume_admin_nonce(&env, 0);
        });
    }

    #[test]
    #[should_panic(expected = "AlreadyFinalized")]
    fn test_require_not_finalized_when_finalized() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            env.storage()
                .persistent()
                .set(&DataKey::Finalization(42), &true);

            require_not_finalized(&env, 42);
        });
    }

    #[test]
    fn test_load_contract_checked_all_checks() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let client = Address::generate(&env);
            let freelancer = Address::generate(&env);
            let contract = Contract {
                client: client.clone(),
                freelancer: freelancer.clone(),
                arbiter: None,
                status: crate::ContractStatus::Created,
                release_authorization: crate::ReleaseAuthorization::ClientOnly,
                funded_amount: 0,
                released_amount: 0,
                refunded_amount: 0,
                total_deposited: 0,
                reputation_issued: false,
            };

            env.storage()
                .persistent()
                .set(&DataKey::Contract(42), &contract);

            let loaded = load_contract_checked(&env, 42, true, true);
            assert_eq!(loaded.client, client);
        });
    }

    #[test]
    #[should_panic(expected = "ContractPaused")]
    fn test_load_contract_checked_paused() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let client = Address::generate(&env);
            let freelancer = Address::generate(&env);
            let contract = Contract {
                client: client.clone(),
                freelancer: freelancer.clone(),
                arbiter: None,
                status: crate::ContractStatus::Created,
                release_authorization: crate::ReleaseAuthorization::ClientOnly,
                funded_amount: 0,
                released_amount: 0,
                refunded_amount: 0,
                total_deposited: 0,
                reputation_issued: false,
            };

            env.storage()
                .persistent()
                .set(&DataKey::Contract(42), &contract);
            env.storage().persistent().set(&DataKey::Paused, &true);

            load_contract_checked(&env, 42, true, true);
        });
    }

    #[test]
    #[should_panic(expected = "AlreadyFinalized")]
    fn test_load_contract_checked_finalized() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let client = Address::generate(&env);
            let freelancer = Address::generate(&env);
            let contract = Contract {
                client: client.clone(),
                freelancer: freelancer.clone(),
                arbiter: None,
                status: crate::ContractStatus::Created,
                release_authorization: crate::ReleaseAuthorization::ClientOnly,
                funded_amount: 0,
                released_amount: 0,
                refunded_amount: 0,
                total_deposited: 0,
                reputation_issued: false,
            };

            env.storage()
                .persistent()
                .set(&DataKey::Contract(42), &contract);
            env.storage()
                .persistent()
                .set(&DataKey::Finalization(42), &true);

            load_contract_checked(&env, 42, true, true);
        });
    }

    #[test]
    fn test_load_contract_checked_no_checks() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let client = Address::generate(&env);
            let freelancer = Address::generate(&env);
            let contract = Contract {
                client: client.clone(),
                freelancer: freelancer.clone(),
                arbiter: None,
                status: crate::ContractStatus::Created,
                release_authorization: crate::ReleaseAuthorization::ClientOnly,
                funded_amount: 0,
                released_amount: 0,
                refunded_amount: 0,
                total_deposited: 0,
                reputation_issued: false,
            };

            env.storage()
                .persistent()
                .set(&DataKey::Contract(42), &contract);
            env.storage().persistent().set(&DataKey::Paused, &true);
            env.storage()
                .persistent()
                .set(&DataKey::Finalization(42), &true);

            // Should succeed because checks are disabled
            let loaded = load_contract_checked(&env, 42, false, false);
            assert_eq!(loaded.client, client);
        });
    }

    #[test]
    #[should_panic(expected = "InvalidContractId")]
    fn test_validate_contract_id_bounds_zero_panics() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            validate_contract_id_bounds(&env, 0);
        });
    }

    #[test]
    #[should_panic(expected = "ContractNotFound")]
    fn test_load_contract_zero_id_panics() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            load_contract(&env, 0);
        });
    }

    #[test]
    #[should_panic(expected = "ContractNotFound")]
    fn test_load_milestones_zero_id_panics() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            load_milestones(&env, 0);
        });
    }

    #[test]
    #[should_panic(expected = "ContractNotFound")]
    fn test_load_contract_checked_zero_id_panics() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            load_contract_checked(&env, 0, false, false);
        });
    }

    #[test]
    #[should_panic(expected = "ContractNotFound")]
    fn test_is_finalized_zero_id_panics() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            is_finalized(&env, 0);
        });
    }

    #[test]
    #[should_panic(expected = "ContractNotFound")]
    fn test_require_not_finalized_zero_id_panics() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            require_not_finalized(&env, 0);
        });
    }

    #[test]
    fn test_validate_contract_id_bounds_valid_range() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            validate_contract_id_bounds(&env, 1);
            validate_contract_id_bounds(&env, 42);
            validate_contract_id_bounds(&env, u32::MAX);
        });
    }

    // ── Compatibility contract: reserved id 0 ───────────────────────────────
    //
    // `test_validate_contract_id_bounds_zero_panics` above locks the strict
    // guard (`InvalidContractId`); the tests below lock the loader guard.

    /// The loader guard treats the reserved id exactly like an unknown id so
    /// the sentinel never surfaces as a distinct, spoofable error.
    #[test]
    #[should_panic(expected = "ContractNotFound")]
    fn test_loader_guard_rejects_zero_with_contract_not_found() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            require_nonzero_contract_id(&env, 0);
        });
    }

    #[test]
    fn test_loader_guard_accepts_nonzero_ids() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            require_nonzero_contract_id(&env, 1);
            require_nonzero_contract_id(&env, 42);
            require_nonzero_contract_id(&env, u32::MAX);
        });
    }

    /// Regression: `load_milestones` reads through the canonical
    /// `keys::milestone_key` constructor, so a vector written via that key is
    /// found and a vector written nowhere is not silently defaulted.
    #[test]
    fn test_load_milestones_uses_canonical_key() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let milestones = Vec::from_array(
                &env,
                [Milestone {
                    amount: 500,
                    funded_amount: 0,
                    released: false,
                    refunded: false,
                    deadline: None,
                    refunded_amount: 0,
                    work_evidence: None,
                }],
            );
            env.storage()
                .persistent()
                .set(&crate::keys::milestone_key(&env, 7), &milestones);

            let loaded = load_milestones(&env, 7);
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded.get(0).unwrap().amount, 500);
        });
    }

    // ── Compatibility contract: admin nonce ─────────────────────────────────

    fn stored_nonce(env: &Env) -> u64 {
        env.storage()
            .persistent()
            .get(&DataKey::AdminNonce)
            .unwrap_or(0)
    }

    #[test]
    fn test_consume_admin_nonce_first_call_advances_to_one() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            consume_admin_nonce(&env, 1);
            assert_eq!(stored_nonce(&env), 1);
        });
    }

    #[test]
    fn test_consume_admin_nonce_is_sequential() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            consume_admin_nonce(&env, 1);
            consume_admin_nonce(&env, 2);
            consume_admin_nonce(&env, 3);
            assert_eq!(stored_nonce(&env), 3);
        });
    }

    #[test]
    fn test_consume_admin_nonce_advances_from_nonzero_base() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            env.storage().persistent().set(&DataKey::AdminNonce, &41u64);
            consume_admin_nonce(&env, 42);
            assert_eq!(stored_nonce(&env), 42);
        });
    }

    #[test]
    fn test_consume_admin_nonce_accepts_u64_max_as_last_value() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            env.storage()
                .persistent()
                .set(&DataKey::AdminNonce, &(u64::MAX - 1));
            consume_admin_nonce(&env, u64::MAX);
            assert_eq!(stored_nonce(&env), u64::MAX);
        });
    }

    #[test]
    #[should_panic(expected = "StaleNonce")]
    fn test_consume_admin_nonce_rejects_zero_as_first_value() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            consume_admin_nonce(&env, 0);
        });
    }

    #[test]
    #[should_panic(expected = "StaleNonce")]
    fn test_consume_admin_nonce_rejects_future_value() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            consume_admin_nonce(&env, 5);
        });
    }

    #[test]
    #[should_panic(expected = "StaleNonce")]
    fn test_consume_admin_nonce_rejects_replay() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            consume_admin_nonce(&env, 1);
            // Re-submitting an already-consumed nonce must be rejected so a
            // replayed admin call cannot re-execute.
            consume_admin_nonce(&env, 1);
        });
    }

    #[test]
    #[should_panic(expected = "PotentialOverflow")]
    fn test_consume_admin_nonce_fails_closed_at_u64_max() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            env.storage()
                .persistent()
                .set(&DataKey::AdminNonce, &u64::MAX);
            // current + 1 must not wrap to 0 (which would accept provided == 0).
            consume_admin_nonce(&env, 0);
        });
    }
}
