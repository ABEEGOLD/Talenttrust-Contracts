//! Typed storage keys and read/write helpers for settlement entries.
//!
//! This module replaces ad-hoc key construction for settlement-related
//! persistent storage with a single, auditable layer. Every settlement
//! read or write in the contract goes through the helpers defined here,
//! guaranteeing that the correct `DataKey` variant and storage bucket
//! (persistent vs. temporary) are always used.
//!
//! # Storage keys
//!
//! | Entry | `DataKey` variant | Bucket |
//! | --- | --- | --- |
//! | Settlement token address | `SettlementToken` | `persistent()` |
//! | Finalization record | `Finalization(contract_id)` | `persistent()` |
//!
//! # Persistence invariants
//!
//! Every `write_*` followed by the corresponding `read_*` returns the
//! same value.  The `test_settlement_storage` module in `test/` verifies
//! this invariant plus absent-key behaviour.
//!
//! # Concurrency and idempotency invariants
//!
//! Settlement state transitions must be safe under concurrent or repeated
//! execution.  The helpers below enforce the following invariants:
//!
//! 1. **Write-once settlement token.** [`write_settlement_token`] is
//!    guarded by [`require_settlement_token_unbound`], which panics with
//!    [`Error::SettlementTokenAlreadyBound`] if a token is already bound.
//!    This prevents a racing or retried bind from silently rebinding the
//!    token to a different address.
//! 2. **Write-once finalization.** [`write_finalization`] is guarded by
//!    [`require_not_finalized`], which panics with
//!    [`Error::AlreadyFinalized`] if a record already exists.  A racing or
//!    retried finalize therefore cannot overwrite a prior record.
//! 3. **Atomic check-and-write.** The guard and the write are performed in
//!    the same contract invocation, so Soroban's transactional execution
//!    model guarantees that either both happen or neither does.

use crate::{finalize::FinalizationRecord, DataKey, Error};
use soroban_sdk::{Address, Env};

/// Result of a commit-once settlement write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommitOutcome {
    /// No value existed, so this invocation persisted it.
    Committed,
    /// The same value was already persisted; no storage mutation was needed.
    Recovered,
}

fn validate_contract_id(contract_id: u32) -> Result<(), Error> {
    if contract_id == 0 {
        return Err(Error::InvalidContractId);
    }
    Ok(())
}

fn panic_on_error<T>(env: &Env, result: Result<T, Error>) -> T {
    result.unwrap_or_else(|error| env.panic_with_error(error))
}

// ── Settlement token ────────────────────────────────────────────────────────

/// Read the bound settlement token address from persistent storage.
///
/// Returns `None` when no token has been bound yet (`bind_settlement_token`
/// has not been called).
///
/// # Arguments
///
/// * `env` – The Soroban environment.
///
/// # Returns
///
/// `Some(Address)` of the bound SAC token, or `None` if the token has not
/// been bound yet.
///
/// # Example
///
/// ```no_run
/// use soroban_sdk::{testutils::Address as _, Address, Env};
/// use escrow::{Escrow, DataKey};
/// use escrow::settlement::read_settlement_token;
///
/// let env = Env::default();
/// let contract = env.register(Escrow, ());
/// env.as_contract(&contract, || {
///     // Before any binding, the result is None.
///     assert!(read_settlement_token(&env).is_none());
///
///     // After writing a token address it is returned.
///     let token = Address::generate(&env);
///     env.storage().persistent().set(&DataKey::SettlementToken, &token);
///     assert_eq!(read_settlement_token(&env), Some(token));
/// });
/// ```
pub fn read_settlement_token(env: &Env) -> Option<Address> {
    env.storage().persistent().get(&DataKey::SettlementToken)
}

/// Commit the settlement token address under the canonical storage key.
///
/// An identical retry is a no-op. A retry with a different address returns
/// [`Error::SettlementTokenAlreadyBound`] without changing the original
/// binding.
pub(crate) fn commit_settlement_token(env: &Env, token: &Address) -> Result<CommitOutcome, Error> {
    match read_settlement_token(env) {
        None => {
            env.storage()
                .persistent()
                .set(&DataKey::SettlementToken, token);
            Ok(CommitOutcome::Committed)
        }
        Some(existing) if existing == *token => Ok(CommitOutcome::Recovered),
        Some(_) => Err(Error::SettlementTokenAlreadyBound),
    }
}

/// Persist the settlement token address under the canonical storage key.
///
/// This compatibility wrapper preserves the existing `()` return type while
/// delegating overwrite prevention and retry handling to
/// [`commit_settlement_token`].
///
/// # Arguments
///
/// * `env`   – The Soroban environment.
/// * `token` – The SAC token [`Address`] to bind.
///
/// # Example
///
/// ```no_run
/// use soroban_sdk::{testutils::Address as _, Address, Env};
/// use escrow::{Escrow, DataKey};
/// use escrow::settlement::{write_settlement_token, read_settlement_token};
///
/// let env = Env::default();
/// let contract = env.register(Escrow, ());
/// let token = Address::generate(&env);
///
/// env.as_contract(&contract, || {
///     write_settlement_token(&env, &token);
///     assert_eq!(read_settlement_token(&env), Some(token));
/// });
/// ```
pub fn write_settlement_token(env: &Env, token: &Address) {
    let _ = panic_on_error(env, commit_settlement_token(env, token));
}

/// Panic with [`Error::SettlementTokenAlreadyBound`] if a settlement token
/// is already bound.
///
/// Callers must invoke this guard immediately before
/// [`write_settlement_token`] to enforce write-once semantics under
/// concurrent or repeated execution.  Because the guard and the write run
/// in the same invocation, Soroban's transactional execution model makes
/// the check-and-write atomic: a racing bind either observes the token as
/// unbound and writes, or observes it as bound and panics — never both.
///
/// # Arguments
///
/// * `env` – The Soroban environment.
///
/// # Errors
///
/// Panics with [`Error::SettlementTokenAlreadyBound`] when
/// [`is_settlement_token_bound`] returns `true`.
pub fn require_settlement_token_unbound(env: &Env) {
    if is_settlement_token_bound(env) {
        env.panic_with_error(Error::SettlementTokenAlreadyBound);
    }
}

/// Return `true` when a settlement token has been bound.
///
/// # Arguments
///
/// * `env` – The Soroban environment.
///
/// # Returns
///
/// `true` if a token address is present in persistent storage, `false`
/// otherwise.
///
/// # Example
///
/// ```no_run
/// use soroban_sdk::{testutils::Address as _, Address, Env};
/// use escrow::Escrow;
/// use escrow::settlement::{is_settlement_token_bound, write_settlement_token};
///
/// let env = Env::default();
/// let contract = env.register(Escrow, ());
///
/// env.as_contract(&contract, || {
///     assert!(!is_settlement_token_bound(&env));
///
///     let token = Address::generate(&env);
///     write_settlement_token(&env, &token);
///     assert!(is_settlement_token_bound(&env));
/// });
/// ```
pub fn is_settlement_token_bound(env: &Env) -> bool {
    read_settlement_token(env).is_some()
}

/// Read the bound settlement token, panicking with [`Error::SettlementTokenNotConfigured`]
/// when absent.  Use this in money-flow paths that require a bound token.
///
/// # Arguments
///
/// * `env` – The Soroban environment.
///
/// # Returns
///
/// The [`Address`] of the bound settlement token.
///
/// # Errors
///
/// Panics with [`Error::SettlementTokenNotConfigured`] when no token has
/// been bound via [`write_settlement_token`].
///
/// # Example
///
/// ```no_run
/// use soroban_sdk::{testutils::Address as _, Address, Env};
/// use escrow::Escrow;
/// use escrow::settlement::{require_settlement_token, write_settlement_token};
///
/// let env = Env::default();
/// let contract = env.register(Escrow, ());
/// let token = Address::generate(&env);
///
/// env.as_contract(&contract, || {
///     write_settlement_token(&env, &token);
///
///     // Returns the bound address when one is present.
///     let bound = require_settlement_token(&env);
///     assert_eq!(bound, token);
/// });
/// ```
///
/// Calling this without a prior [`write_settlement_token`] panics:
///
/// ```no_run
/// use soroban_sdk::Env;
/// use escrow::Escrow;
/// use escrow::settlement::require_settlement_token;
///
/// let env = Env::default();
/// let contract = env.register(Escrow, ());
/// env.as_contract(&contract, || {
///     let _ = require_settlement_token(&env); // panics: SettlementTokenNotConfigured
/// });
/// ```
pub fn require_settlement_token(env: &Env) -> Address {
    read_settlement_token(env)
        .unwrap_or_else(|| env.panic_with_error(Error::SettlementTokenNotConfigured))
}

// ── Finalization record ─────────────────────────────────────────────────────

/// Construct the canonical [`DataKey`] for a finalization record.
///
/// # Arguments
///
/// * `contract_id` – The numeric contract identifier.
///
/// # Returns
///
/// `DataKey::Finalization(contract_id)`.
///
/// # Example
///
/// ```no_run
/// use escrow::{DataKey, settlement::finalization_key};
///
/// let key = finalization_key(7);
/// assert_eq!(key, DataKey::Finalization(7));
/// ```
pub fn finalization_key(contract_id: u32) -> DataKey {
    DataKey::Finalization(contract_id)
}

/// Read a finalization record for `contract_id`, if it exists.
///
/// # Arguments
///
/// * `env`         – The Soroban environment.
/// * `contract_id` – The numeric contract identifier.
///
/// # Returns
///
/// `Some(FinalizationRecord)` when the contract has been finalized, `None`
/// otherwise.
///
/// # Example
///
/// ```no_run
/// use soroban_sdk::{testutils::Address as _, Address, Env};
/// use escrow::{
///     Escrow, ContractStatus, ContractSummary, CONTRACT_SUMMARY_SCHEMA_VERSION,
///     settlement::{read_finalization, write_finalization},
/// };
/// use escrow::finalize::FinalizationRecord;
///
/// let env = Env::default();
/// let contract = env.register(Escrow, ());
///
/// env.as_contract(&contract, || {
///     // Returns None before any record is written.
///     assert!(read_finalization(&env, 1).is_none());
/// });
/// ```
pub fn read_finalization(env: &Env, contract_id: u32) -> Option<FinalizationRecord> {
    panic_on_error(env, validate_contract_id(contract_id));
    env.storage()
        .persistent()
        .get(&finalization_key(contract_id))
}

/// Return `true` when a finalization record already exists for `contract_id`.
///
/// # Arguments
///
/// * `env`         – The Soroban environment.
/// * `contract_id` – The numeric contract identifier.
///
/// # Returns
///
/// `true` if a [`FinalizationRecord`] is stored for `contract_id`, `false`
/// otherwise.
///
/// # Example
///
/// ```no_run
/// use soroban_sdk::{testutils::Address as _, Address, Env};
/// use escrow::{
///     Escrow, ContractStatus, ContractSummary, CONTRACT_SUMMARY_SCHEMA_VERSION,
///     settlement::{is_finalized, write_finalization},
/// };
/// use escrow::finalize::FinalizationRecord;
///
/// let env = Env::default();
/// let contract = env.register(Escrow, ());
///
/// env.as_contract(&contract, || {
///     assert!(!is_finalized(&env, 42));
///
///     let record = FinalizationRecord {
///         finalizer: Address::generate(&env),
///         timestamp: 9999,
///         summary: ContractSummary {
///             schema_version: CONTRACT_SUMMARY_SCHEMA_VERSION,
///             client: Address::generate(&env),
///             freelancer: Address::generate(&env),
///             arbiter: None,
///             status: ContractStatus::Completed,
///             reputation_issued: false,
///             total_amount: 500,
///             funded_amount: 500,
///             released_amount: 500,
///             refundable_balance: 0,
///             released_milestone_count: 1,
///             milestones: soroban_sdk::Vec::new(&env),
///         },
///     };
///     write_finalization(&env, 42, &record);
///     assert!(is_finalized(&env, 42));
/// });
/// ```
pub fn is_finalized(env: &Env, contract_id: u32) -> bool {
    panic_on_error(env, validate_contract_id(contract_id));
    env.storage()
        .persistent()
        .has(&finalization_key(contract_id))
}

/// Commit a finalization record without allowing an existing record to change.
///
/// An identical retry is a no-op. A retry with a different record returns
/// [`Error::AlreadyFinalized`]. Invalid contract ID `0` returns
/// [`Error::InvalidContractId`]. Every error is reported before a storage write.
pub(crate) fn commit_finalization(
    env: &Env,
    contract_id: u32,
    record: &FinalizationRecord,
) -> Result<CommitOutcome, Error> {
    validate_contract_id(contract_id)?;

    let key = finalization_key(contract_id);
    match env
        .storage()
        .persistent()
        .get::<_, FinalizationRecord>(&key)
    {
        None => {
            env.storage().persistent().set(&key, record);
            Ok(CommitOutcome::Committed)
        }
        Some(existing) if existing == *record => Ok(CommitOutcome::Recovered),
        Some(_) => Err(Error::AlreadyFinalized),
    }
}

/// Persist a finalization record under the canonical storage key.
///
/// This compatibility wrapper preserves the existing `()` return type while
/// delegating validation, overwrite prevention, and retry handling to
/// [`commit_finalization`].
///
/// # Arguments
///
/// * `env`         – The Soroban environment.
/// * `contract_id` – The numeric contract identifier.
/// * `record`      – The [`FinalizationRecord`] to persist.
///
/// # Example
///
/// ```no_run
/// use soroban_sdk::{testutils::Address as _, Address, Env};
/// use escrow::{
///     Escrow, ContractStatus, ContractSummary, CONTRACT_SUMMARY_SCHEMA_VERSION,
///     settlement::{read_finalization, write_finalization},
/// };
/// use escrow::finalize::FinalizationRecord;
///
/// let env = Env::default();
/// let contract = env.register(Escrow, ());
/// let finalizer = Address::generate(&env);
///
/// env.as_contract(&contract, || {
///     let record = FinalizationRecord {
///         finalizer: finalizer.clone(),
///         timestamp: 1_000_000,
///         summary: ContractSummary {
///             schema_version: CONTRACT_SUMMARY_SCHEMA_VERSION,
///             client: Address::generate(&env),
///             freelancer: Address::generate(&env),
///             arbiter: None,
///             status: ContractStatus::Completed,
///             reputation_issued: false,
///             total_amount: 1_000,
///             funded_amount: 1_000,
///             released_amount: 1_000,
///             refundable_balance: 0,
///             released_milestone_count: 1,
///             milestones: soroban_sdk::Vec::new(&env),
///         },
///     };
///     write_finalization(&env, 5, &record);
///
///     let loaded = read_finalization(&env, 5).unwrap();
///     assert_eq!(loaded.finalizer, finalizer);
///     assert_eq!(loaded.timestamp, 1_000_000);
/// });
/// ```
pub fn write_finalization(env: &Env, contract_id: u32, record: &FinalizationRecord) {
    let _ = panic_on_error(env, commit_finalization(env, contract_id, record));
}

/// Atomically write a finalization record, panicking with
/// [`Error::AlreadyFinalized`] if one already exists.
///
/// This is the preferred entry point for finalization under concurrent or
/// repeated execution: it combines the [`require_not_finalized`] guard and
/// the [`write_finalization`] write into a single call so callers cannot
/// accidentally skip the guard.  Because both operations run in the same
/// invocation, Soroban's transactional execution model guarantees that a
/// racing or retried finalize either writes exactly once or panics without
/// mutating state.
///
/// # Arguments
///
/// * `env`         – The Soroban environment.
/// * `contract_id` – The numeric contract identifier.
/// * `record`      – The [`FinalizationRecord`] to persist.
///
/// # Errors
///
/// Panics with [`Error::AlreadyFinalized`] when a record already exists for
/// `contract_id`.
pub fn write_finalization_once(env: &Env, contract_id: u32, record: &FinalizationRecord) {
    require_not_finalized(env, contract_id);
    write_finalization(env, contract_id, record);
}

/// Panic with [`Error::AlreadyFinalized`] if a record already exists for
/// `contract_id`.
///
/// # Arguments
///
/// * `env`         – The Soroban environment.
/// * `contract_id` – The numeric contract identifier to guard.
///
/// # Errors
///
/// Panics with [`Error::AlreadyFinalized`] when [`is_finalized`] returns
/// `true` for the given `contract_id`.
///
/// # Example
///
/// ```no_run
/// use soroban_sdk::{testutils::Address as _, Address, Env};
/// use escrow::{
///     Escrow, ContractStatus, ContractSummary, CONTRACT_SUMMARY_SCHEMA_VERSION,
///     settlement::{require_not_finalized, write_finalization},
/// };
/// use escrow::finalize::FinalizationRecord;
///
/// let env = Env::default();
/// let contract = env.register(Escrow, ());
///
/// env.as_contract(&contract, || {
///     // No record yet — guard passes silently.
///     require_not_finalized(&env, 10);
/// });
/// ```
///
/// Once a record is written, the guard panics:
///
/// ```no_run
/// use soroban_sdk::{testutils::Address as _, Address, Env};
/// use escrow::{
///     Escrow, ContractStatus, ContractSummary, CONTRACT_SUMMARY_SCHEMA_VERSION,
///     settlement::{require_not_finalized, write_finalization},
/// };
/// use escrow::finalize::FinalizationRecord;
///
/// let env = Env::default();
/// let contract = env.register(Escrow, ());
///
/// env.as_contract(&contract, || {
///     let record = FinalizationRecord {
///         finalizer: Address::generate(&env),
///         timestamp: 1,
///         summary: ContractSummary {
///             schema_version: CONTRACT_SUMMARY_SCHEMA_VERSION,
///             client: Address::generate(&env),
///             freelancer: Address::generate(&env),
///             arbiter: None,
///             status: ContractStatus::Completed,
///             reputation_issued: false,
///             total_amount: 0,
///             funded_amount: 0,
///             released_amount: 0,
///             refundable_balance: 0,
///             released_milestone_count: 0,
///             milestones: soroban_sdk::Vec::new(&env),
///         },
///     };
///     write_finalization(&env, 10, &record);
///     require_not_finalized(&env, 10); // panics: AlreadyFinalized
/// });
/// ```
pub fn require_not_finalized(env: &Env, contract_id: u32) {
    if is_finalized(env, contract_id) {
        env.panic_with_error(Error::AlreadyFinalized);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::finalize::FinalizationRecord;
    use crate::{
        ContractStatus, ContractSummary, Escrow, EscrowClient, CONTRACT_SUMMARY_SCHEMA_VERSION,
    };
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env};

    /// Passes the first external token probe but fails the later `decimals`
    /// dependency call used during binding.
    #[contract]
    struct BalanceOnlyToken;

    #[contractimpl]
    impl BalanceOnlyToken {
        pub fn balance(_env: Env, _id: Address) -> i128 {
            0
        }
    }

    fn setup_contract(env: &Env) -> Address {
        env.register(Escrow, ())
    }

    fn dummy_summary(env: &Env) -> ContractSummary {
        ContractSummary {
            schema_version: CONTRACT_SUMMARY_SCHEMA_VERSION,
            client: Address::generate(env),
            freelancer: Address::generate(env),
            arbiter: None,
            status: ContractStatus::Completed,
            reputation_issued: false,
            total_amount: 1_000,
            funded_amount: 1_000,
            released_amount: 1_000,
            refundable_balance: 0,
            released_milestone_count: 1,
            milestones: soroban_sdk::Vec::new(env),
        }
    }

    fn assert_contract_error<T: core::fmt::Debug, E: core::fmt::Debug>(
        result: Result<Result<T, E>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
        expected: Error,
    ) {
        match result {
            Err(Ok(actual)) => {
                let expected: soroban_sdk::Error = expected.into();
                assert_eq!(actual, expected);
            }
            other => panic!("expected Error::{expected:?}, got {other:?}"),
        }
    }

    // ── Settlement token round-trip ────────────────────────────────────────

    #[test]
    fn settlement_token_absent_returns_none() {
        let env = Env::default();
        let contract = setup_contract(&env);
        env.as_contract(&contract, || {
            assert!(read_settlement_token(&env).is_none());
            assert!(!is_settlement_token_bound(&env));
        });
    }

    #[test]
    fn settlement_token_round_trip() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let token = Address::generate(&env);

        env.as_contract(&contract, || {
            write_settlement_token(&env, &token);
            assert_eq!(read_settlement_token(&env), Some(token));
            assert!(is_settlement_token_bound(&env));
        });
    }

    #[test]
    fn settlement_token_identical_retry_is_idempotent() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let token = Address::generate(&env);

        env.as_contract(&contract, || {
            assert_eq!(
                commit_settlement_token(&env, &token),
                Ok(CommitOutcome::Committed)
            );
            assert_eq!(
                commit_settlement_token(&env, &token),
                Ok(CommitOutcome::Recovered)
            );
            assert_eq!(read_settlement_token(&env), Some(token));
        });
    }

    #[test]
    fn settlement_token_conflicting_retry_preserves_original() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let original = Address::generate(&env);
        let conflicting = Address::generate(&env);

        env.as_contract(&contract, || {
            assert_eq!(
                commit_settlement_token(&env, &original),
                Ok(CommitOutcome::Committed)
            );
            assert_eq!(
                commit_settlement_token(&env, &conflicting),
                Err(Error::SettlementTokenAlreadyBound)
            );
            assert_eq!(read_settlement_token(&env), Some(original));
        });
    }

    #[test]
    fn token_dependency_failure_rolls_back_and_retry_succeeds() {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        let contract = setup_contract(&env);
        let client = EscrowClient::new(&env, &contract);
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let partial_token = env.register(BalanceOnlyToken, ());
        assert!(client
            .try_bind_settlement_token(&admin, &partial_token)
            .is_err());

        assert_eq!(client.get_settlement_token(), None);
        assert_eq!(client.get_token_scale(), None);

        let valid_token = env.register_stellar_asset_contract(admin.clone());
        assert!(client.bind_settlement_token(&admin, &valid_token));
        assert_eq!(client.get_settlement_token(), Some(valid_token));
        assert_eq!(client.get_token_scale(), Some(7));
    }

    // ── Finalization round-trip ────────────────────────────────────────────

    #[test]
    fn finalization_absent_returns_none() {
        let env = Env::default();
        let contract = setup_contract(&env);
        env.as_contract(&contract, || {
            assert!(!is_finalized(&env, 1));
            assert!(read_finalization(&env, 1).is_none());
        });
    }

    #[test]
    fn finalization_round_trip() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let record = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 12345,
            summary: dummy_summary(&env),
        };

        env.as_contract(&contract, || {
            write_finalization(&env, 42, &record);
            assert!(is_finalized(&env, 42));
            let loaded = read_finalization(&env, 42).unwrap();
            assert_eq!(loaded.finalizer, record.finalizer);
            assert_eq!(loaded.timestamp, 12345);
        });
    }

    #[test]
    fn finalization_different_ids_are_independent() {
        let env = Env::default();
        let contract = setup_contract(&env);

        let record_a = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 100,
            summary: dummy_summary(&env),
        };
        let record_b = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 200,
            summary: dummy_summary(&env),
        };

        env.as_contract(&contract, || {
            write_finalization(&env, 1, &record_a);
            write_finalization(&env, 2, &record_b);

            assert_eq!(read_finalization(&env, 1).unwrap().timestamp, 100);
            assert_eq!(read_finalization(&env, 2).unwrap().timestamp, 200);
        });
    }

    #[test]
    fn finalization_identical_retry_is_idempotent() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let record = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 100,
            summary: dummy_summary(&env),
        };

        env.as_contract(&contract, || {
            assert_eq!(
                commit_finalization(&env, 1, &record),
                Ok(CommitOutcome::Committed)
            );
            assert_eq!(
                commit_finalization(&env, 1, &record),
                Ok(CommitOutcome::Recovered)
            );
            assert_eq!(read_finalization(&env, 1), Some(record));
        });
    }

    #[test]
    fn finalization_conflicting_retry_preserves_original() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let original = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 100,
            summary: dummy_summary(&env),
        };
        let conflicting = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 200,
            summary: dummy_summary(&env),
        };

        env.as_contract(&contract, || {
            assert_eq!(
                commit_finalization(&env, 1, &original),
                Ok(CommitOutcome::Committed)
            );
            assert_eq!(
                commit_finalization(&env, 1, &conflicting),
                Err(Error::AlreadyFinalized)
            );
            assert_eq!(read_finalization(&env, 1), Some(original));
        });
    }

    #[test]
    fn finalization_zero_id_is_rejected_without_storage() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let record = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 100,
            summary: dummy_summary(&env),
        };

        env.as_contract(&contract, || {
            assert_eq!(
                commit_finalization(&env, 0, &record),
                Err(Error::InvalidContractId)
            );
            assert!(!env.storage().persistent().has(&DataKey::Finalization(0)));
        });
    }

    #[test]
    fn finalization_max_id_round_trip() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let record = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: u64::MAX,
            summary: dummy_summary(&env),
        };

        env.as_contract(&contract, || {
            assert_eq!(
                commit_finalization(&env, u32::MAX, &record),
                Ok(CommitOutcome::Committed)
            );
            assert_eq!(read_finalization(&env, u32::MAX), Some(record));
        });
    }

    #[test]
    #[should_panic(expected = "HostError: Error(Contract, #4)")]
    fn finalization_zero_id_read_panics_with_typed_error() {
        let env = Env::default();
        let contract = setup_contract(&env);
        env.as_contract(&contract, || {
            let _ = read_finalization(&env, 0);
        });
    }

    #[test]
    fn public_finalization_boundaries_return_typed_errors() {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        let contract = setup_contract(&env);
        let client = EscrowClient::new(&env, &contract);
        let finalizer = Address::generate(&env);

        assert_contract_error(
            client.try_finalize_contract(&0, &finalizer),
            Error::InvalidContractId,
        );
        assert_contract_error(
            client.try_get_finalization_record(&0),
            Error::InvalidContractId,
        );
        assert_contract_error(
            client.try_finalize_contract(&u32::MAX, &finalizer),
            Error::ContractNotFound,
        );
    }

    #[test]
    fn require_not_finalized_passes_when_absent() {
        let env = Env::default();
        let contract = setup_contract(&env);
        env.as_contract(&contract, || {
            require_not_finalized(&env, 99);
        });
    }

    #[test]
    #[should_panic(expected = "HostError: Error(Contract, #46)")]
    fn require_not_finalized_panics_when_present() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let record = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 1,
            summary: dummy_summary(&env),
        };
        env.as_contract(&contract, || {
            write_finalization(&env, 1, &record);
            require_not_finalized(&env, 1);
        });
    }

    // ── Concurrency / idempotency guards ───────────────────────────────────

    #[test]
    fn require_settlement_token_unbound_passes_when_absent() {
        let env = Env::default();
        let contract = setup_contract(&env);
        env.as_contract(&contract, || {
            require_settlement_token_unbound(&env);
        });
    }

    #[test]
    #[should_panic(expected = "HostError: Error(Contract, #")]
    fn require_settlement_token_unbound_panics_when_bound() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let token = Address::generate(&env);
        env.as_contract(&contract, || {
            write_settlement_token(&env, &token);
            require_settlement_token_unbound(&env);
        });
    }

    #[test]
    fn write_finalization_once_writes_when_absent() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let record = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 7,
            summary: dummy_summary(&env),
        };
        env.as_contract(&contract, || {
            write_finalization_once(&env, 7, &record);
            assert!(is_finalized(&env, 7));
            assert_eq!(read_finalization(&env, 7).unwrap().timestamp, 7);
        });
    }

    #[test]
    #[should_panic(expected = "HostError: Error(Contract, #46)")]
    fn write_finalization_once_panics_on_duplicate() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let record = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 1,
            summary: dummy_summary(&env),
        };
        env.as_contract(&contract, || {
            write_finalization_once(&env, 1, &record);
            // A racing or retried finalize must not overwrite the record.
            write_finalization_once(&env, 1, &record);
        });
    }

    #[test]
    fn write_finalization_once_does_not_mutate_on_duplicate() {
        let env = Env::default();
        let contract = setup_contract(&env);
        let first = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 100,
            summary: dummy_summary(&env),
        };
        let second = FinalizationRecord {
            finalizer: Address::generate(&env),
            timestamp: 200,
            summary: dummy_summary(&env),
        };
        env.as_contract(&contract, || {
            write_finalization_once(&env, 5, &first);
            // Attempt a duplicate write; it must panic and leave state intact.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                write_finalization_once(&env, 5, &second);
            }));
            assert!(result.is_err());
            assert_eq!(read_finalization(&env, 5).unwrap().timestamp, 100);
        });
    }
}
