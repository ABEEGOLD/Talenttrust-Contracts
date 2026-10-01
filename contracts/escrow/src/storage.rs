//! Centralized storage precondition checks and contract loading helpers.
//! Centralized storage precondition checks and contract loading helpers.
//!
//! This module extracts repeated storage validation patterns into a single source
//! of truth, ensuring consistent error handling and reducing code duplication across
//! entrypoints. All contract loading operations should route through these helpers.
//!
//! ## State invariants
//!
//! The helpers in this module are the single choke point for the following
//! invariants. Any change to these helpers must preserve them:
//!
//! 1. **Initialization gate**: no money-flow entrypoint may proceed unless
//!    `DataKey::Initialized` is `true`. `require_initialized` is the only
//!    sanctioned check.
//! 2. **Contract identity**: `contract_id == 0` is never a valid key. All
//!    loaders call `validate_contract_id_bounds` before touching storage so
//!    that a zero ID can never alias a real record.
//! 3. **Pause / emergency precedence**: emergency always blocks, then legacy
//!    boolean pause, then scoped pause. `require_not_paused` and
//!    `require_pause_scope` must agree on this ordering.
//! 4. **Finalization is terminal**: once `DataKey::Finalization(id)` exists,
//!    no mutation helper may return a contract for that ID when
//!    `check_finalized` is requested.
//! 5. **Monotonic admin nonce**: `consume_admin_nonce` must reject any value
//!    other than `current + 1` and must persist the increment on success so
//!    that retries and replays cannot double-apply an admin action.
//!
//! These invariants are enforced by panics (via `env.panic_with_error`) so
//! that a failed precondition aborts the whole transaction and leaves no
//! partial state behind.

use crate::{Contract, DataKey, Error};
use soroban_sdk::{Env, Symbol, Vec};

/// Maximum number of retry attempts for recoverable storage operations.
///
/// Bounds the retry loop so a persistently failing storage backend cannot
/// cause an unbounded loop. Chosen to be small enough to fail fast while
/// still tolerating transient read/write hiccups.
pub(crate) const MAX_STORAGE_RETRIES: u32 = 3;

/// Deterministic recovery outcome for a storage operation.
///
/// Used by [`recover_or_panic`] to make failure handling explicit and
/// observable. Callers can log or branch on the outcome without relying on
/// panic side effects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryOutcome {
    /// Operation succeeded on the first attempt.
    Success,
    /// Operation succeeded after one or more retries.
    Recovered { attempts: u32 },
    /// Operation failed after exhausting all retries.
    Exhausted { attempts: u32 },
}

/// Run a fallible storage operation with deterministic retry semantics.
///
/// The closure is invoked up to [`MAX_STORAGE_RETRIES`] times. The first
/// successful invocation returns `Ok(RecoveryOutcome::Success)` or
/// `Ok(RecoveryOutcome::Recovered { attempts })`. If every attempt fails,
/// the last error is returned as `Err`.
///
/// This helper is intentionally pure with respect to storage: it does not
/// mutate state on failure, so partial failures cannot leave the contract
/// in an inconsistent state. Callers must ensure the closure itself is
/// idempotent (reads are always safe; writes should be guarded by
/// precondition checks performed before entering the retry loop).
pub(crate) fn recover_or_panic<F, T, E>(
    env: &Env,
    mut op: F,
) -> Result<RecoveryOutcome, E>
where
    F: FnMut() -> Result<T, E>,
    E: core::fmt::Debug,
{
    let mut last_err: Option<E> = None;
    let mut attempts: u32 = 0;
    while attempts < MAX_STORAGE_RETRIES {
        attempts += 1;
        match op() {
            Ok(_) => {
                return Ok(if attempts == 1 {
                    RecoveryOutcome::Success
                } else {
                    RecoveryOutcome::Recovered { attempts }
                });
            }
            Err(err) => {
                last_err = Some(err);
            }
        }
    }
    let _ = env;
    match last_err {
        Some(err) => Err(err),
        None => unreachable!("retry loop must execute at least once"),
    }
}

/// Deterministically load a contract, retrying transient storage failures.
///
/// Unlike [`load_contract`], this variant does not panic on the first
/// missing read. It retries up to [`MAX_STORAGE_RETRIES`] times and only
/// panics with `ContractNotFound` once all attempts are exhausted. This
/// makes recovery observable and prevents a single transient miss from
/// aborting an otherwise valid operation.
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is 0
/// - `ContractNotFound` if the contract is still missing after all retries
pub(crate) fn load_contract_recoverable(env: &Env, contract_id: u32) -> Contract {
    validate_contract_id_bounds(env, contract_id);
    let outcome = recover_or_panic(env, || {
        env.storage()
            .persistent()
            .get::<_, Contract>(&DataKey::Contract(contract_id))
            .ok_or(Error::ContractNotFound)
    });
    match outcome {
        Ok(_) => env
            .storage()
            .persistent()
            .get(&DataKey::Contract(contract_id))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound)),
        Err(err) => env.panic_with_error(err),
    }
}

/// Deterministically load milestones, retrying transient storage failures.
///
/// Mirrors [`load_milestones`] but retries transient misses before
/// panicking, so recovery is deterministic and observable.
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is 0
/// - `ContractNotFound` if milestones are still missing after all retries
pub(crate) fn load_milestones_recoverable(
    env: &Env,
    contract_id: u32,
) -> Vec<crate::Milestone> {
    validate_contract_id_bounds(env, contract_id);
    let milestone_key = Symbol::new(env, "milestones");
    let outcome = recover_or_panic(env, || {
        env.storage()
            .persistent()
            .get::<_, Vec<crate::Milestone>>(&(
                DataKey::Contract(contract_id),
                milestone_key.clone(),
            ))
            .ok_or(Error::ContractNotFound)
    });
    match outcome {
        Ok(_) => env
            .storage()
            .persistent()
            .get(&(DataKey::Contract(contract_id), milestone_key))
            .unwrap_or_else(|| env.panic_with_error(Error::ContractNotFound)),
        Err(err) => env.panic_with_error(err),
    }
}

/// Validate that contract_id is within numeric bounds (non-zero).
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
        env.panic_with_error(Error::InvalidContractId);
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
    // Invariant 1: initialization is a hard prerequisite. We read the flag
    // directly (no caching) so that a pause/emergency set in the same
    // transaction is always observed. Missing key is treated as
    // uninitialized, never as "ok".
    env.storage()
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
    // Invariant 2: reject the zero ID before any storage access so that a
    // missing record and an invalid key are distinguishable in tests and
    // logs.
    validate_contract_id_bounds(env, contract_id);
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
    // Invariant 2: same zero-ID guard as load_contract. Milestones are keyed
    // by the same contract ID, so an invalid ID must never reach storage.
    validate_contract_id_bounds(env, contract_id);
    // Invariant: milestones cannot exist without their parent contract.
    // Verify the contract record first so a stale/orphaned milestone vector
    // cannot be read after the contract has been removed or never created.
    if !env
        .storage()
        .persistent()
        .has(&DataKey::Contract(contract_id))
    {
        env.panic_with_error(Error::ContractNotFound);
    }
    let milestone_key = Symbol::new(env, "milestones");
    env.storage()
        .persistent()
        .set(&(DataKey::Contract(contract_id), milestone_key), milestones);
}

/// Load milestones for a contract, requiring that the contract itself exists.
///
/// Unlike [`load_milestones`], this variant first loads the parent contract so
/// that a missing contract surfaces as `ContractNotFound` even if a stale
/// milestone vector happens to be present. This prevents orphaned milestone
/// data from being observed after a contract has been removed or never created.
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is 0
/// - `ContractNotFound` if the contract or its milestones are missing
pub(crate) fn load_milestones_checked(
    env: &Env,
    contract_id: u32,
) -> Vec<crate::Milestone> {
    validate_contract_id_bounds(env, contract_id);
    let _ = load_contract(env, contract_id);
    load_milestones(env, contract_id)
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
///
/// # Concurrency
/// All guards are evaluated against the current stored state in this call. The
/// returned contract is the value observed at load time; callers must not cache
/// it across invocations.
pub(crate) fn load_contract_checked(
    env: &Env,
    contract_id: u32,
    check_paused: bool,
    check_finalized: bool,
) -> Contract {
    // Invariant 2 + 3 + 4: order matters. Bounds first (cheapest, no I/O),
    // then pause/emergency (global safety rails), then load, then
    // finalization. Reordering these would let a paused or finalized
    // contract be observed by a caller that requested the guard.
    validate_contract_id_bounds(env, contract_id);
    if check_paused {
        require_not_paused(env);
    }

    let contract = load_contract(env, contract_id);

    if check_finalized {
        require_not_finalized(env, contract_id);
        // Re-check after load to close the race window where a concurrent
        // finalize could have committed between the load and the guard.
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
    // Invariant 3: emergency takes precedence over the legacy boolean pause
    // so that operators can escalate without first clearing the pause flag.
    // Both are read fresh from persistent storage on every call.
    if env
        .storage()
        .persistent()
        .get::<_, bool>(&DataKey::Paused)
        .unwrap_or(false)
    {
        env.panic_with_error(Error::ContractPaused);
    }
    // Emergency always blocks everything.
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
    // Invariant 3: the precedence here must match require_not_paused.
    // Legacy bool == Global, emergency == Global, then scoped pause is
    // compared against the requested target.
    // Legacy boolean pause acts as Global
    if env
        .storage()
        .persistent()
        .get::<_, bool>(&DataKey::Paused)
        .unwrap_or(false)
    {
        env.panic_with_error(Error::ContractPaused);
    }

    // Emergency always blocks everything.
    // Emergency always blocks everything
    if env
        .storage()
        .persistent()
        .get::<_, bool>(&DataKey::Emergency)
        .unwrap_or(false)
    {
        env.panic_with_error(Error::EmergencyActive);
    }

    // Scoped pause.
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
/// Panics with [`Error::StaleNonce`] if the provided nonce does not match.
///
/// # Concurrency
/// The read-modify-write of [`DataKey::AdminNonce`] happens within a single
/// invocation, so two racing admin calls cannot both observe the same expected
/// nonce. A retried call that reuses an already-consumed nonce is rejected,
/// which makes admin operations safe to retry only with a fresh nonce.
pub(crate) fn consume_admin_nonce(env: &Env, provided_nonce: u64) {
    // Invariant 5: monotonic nonce. `current == 0` means uninitialized, so
    // the first accepted value is 1. Any other value (stale or future) is
    // rejected before the write, so a failed call cannot advance the nonce.
    // On success the increment is persisted atomically with the check.
    let current: u64 = env
        .storage()
        .persistent()
        .get(&DataKey::AdminNonce)
        .unwrap_or(0);
    // Invariant: the admin nonce is strictly monotonic and must never wrap.
    // Refuse to advance past u64::MAX so a replay window cannot be reopened.
    if current == u64::MAX {
        env.panic_with_error(Error::StaleNonce);
    }
    let expected = current + 1;
    if provided_nonce != expected {
        env.panic_with_error(Error::StaleNonce);
    }
    env.storage()
        .persistent()
        .set(&DataKey::AdminNonce, &expected);
    true
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
    // Invariant 2: zero ID is never a valid finalization key.
    validate_contract_id_bounds(env, contract_id);
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
///
/// # Concurrency
/// Finalization is a terminal latch. Once set, this helper rejects all further
/// mutations, so duplicate or delayed retries observe a consistent terminal
/// state instead of partially applying a second transition.
pub(crate) fn require_not_finalized(env: &Env, contract_id: u32) -> bool {
    // Invariant 2 + 4: bounds check first, then the terminal-state check.
    validate_contract_id_bounds(env, contract_id);
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

    // Test helpers below exercise each invariant in isolation and in
    // combination. Panics are asserted with `#[should_panic]` so that a
    // regression that silently returns instead of aborting will fail CI.

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
    #[should_panic(expected = "ContractNotFound")]
    fn test_load_milestones_orphaned_vector_rejected() {
        // Regression: a milestone vector must not be readable when its
        // parent contract record is absent. This protects the invariant
        // that milestones and their contract are always co-present.
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let milestones = Vec::from_array(
                &env,
                [Milestone {
                    amount: 1000,
                    funded_amount: 0,
                    released: false,
                    refunded: false,
                    deadline: None,
                    refunded_amount: 0,
                    work_evidence: None,
                }],
            );
            let milestone_key = Symbol::new(&env, "milestones");
            env.storage()
                .persistent()
                .set(&(DataKey::Contract(42), milestone_key), &milestones);

            // No DataKey::Contract(42) written — must still panic.
            load_milestones(&env, 42);
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
    fn test_store_milestones_round_trip() {
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
            store_milestones(&env, 7, &milestones);
            let loaded = load_milestones(&env, 7);
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded.get(0).unwrap().amount, 500);
        });
    }

    #[test]
    fn test_store_milestones_overwrite_is_idempotent() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let first = Vec::from_array(
                &env,
                [Milestone {
                    amount: 100,
                    funded_amount: 0,
                    released: false,
                    refunded: false,
                    deadline: None,
                    refunded_amount: 0,
                    work_evidence: None,
                }],
            );
            let second = Vec::from_array(
                &env,
                [Milestone {
                    amount: 200,
                    funded_amount: 0,
                    released: false,
                    refunded: false,
                    deadline: None,
                    refunded_amount: 0,
                    work_evidence: None,
                }],
            );
            store_milestones(&env, 9, &first);
            store_milestones(&env, 9, &second);
            store_milestones(&env, 9, &second);
            let loaded = load_milestones(&env, 9);
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded.get(0).unwrap().amount, 200);
        });
    }

    #[test]
    #[should_panic(expected = "ContractNotFound")]
    fn test_store_milestones_zero_id_panics() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let milestones = Vec::new(&env);
            store_milestones(&env, 0, &milestones);
        });
    }

    #[test]
    #[should_panic(expected = "ContractNotFound")]
    fn test_load_milestones_checked_missing_contract() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            let milestones = Vec::from_array(
                &env,
                [Milestone {
                    amount: 100,
                    funded_amount: 0,
                    released: false,
                    refunded: false,
                    deadline: None,
                    refunded_amount: 0,
                    work_evidence: None,
                }],
            );
            let milestone_key = Symbol::new(&env, "milestones");
            env.storage()
                .persistent()
                .set(&(DataKey::Contract(77), milestone_key), &milestones);
            load_milestones_checked(&env, 77);
        });
    }

    #[test]
    fn test_load_milestones_checked_with_contract() {
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
                .set(&DataKey::Contract(11), &contract);
            let milestones = Vec::from_array(
                &env,
                [Milestone {
                    amount: 42,
                    funded_amount: 0,
                    released: false,
                    refunded: false,
                    deadline: None,
                    refunded_amount: 0,
                    work_evidence: None,
                }],
            );
            store_milestones(&env, 11, &milestones);
            let loaded = load_milestones_checked(&env, 11);
            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded.get(0).unwrap().amount, 42);
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

    #[test]
    fn test_consume_admin_nonce_first_call() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            consume_admin_nonce(&env, 1);
            let stored: u64 = env
                .storage()
                .persistent()
                .get(&DataKey::AdminNonce)
                .unwrap();
            assert_eq!(stored, 1);
        });
    }

    #[test]
    #[should_panic(expected = "StaleNonce")]
    fn test_consume_admin_nonce_rejects_replay() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            consume_admin_nonce(&env, 1);
            // Replaying the same nonce must be rejected.
            consume_admin_nonce(&env, 1);
        });
    }

    #[test]
    #[should_panic(expected = "StaleNonce")]
    fn test_consume_admin_nonce_rejects_future() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            consume_admin_nonce(&env, 5);
        });
    }

    #[test]
    #[should_panic(expected = "StaleNonce")]
    fn test_consume_admin_nonce_overflow_guard() {
        let (env, admin) = setup_test_env();
        env.as_contract(&admin, || {
            env.storage()
                .persistent()
                .set(&DataKey::AdminNonce, &u64::MAX);
            // Must refuse to advance past u64::MAX rather than wrap to 0.
            consume_admin_nonce(&env, 0);
        });
    }
}
