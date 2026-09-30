//! Centralized storage key definitions and constructors for escrow milestones.

/// ## Deterministic failure recovery invariants
///
/// This module is the single source of truth for every storage key that
/// the escrow contract uses to persist milestone and approval state. Because
/// failure recovery depends on re-deriving the exact same key after a
/// partial failure or retry, the following invariants must always hold:
///
/// 1. **Determinism**: for any given contract id and milestone index,
///    every constructor returns a byte-identical key across invocations,
///    ledgers, and execution contexts. No clock, randomness, or address
///    dependent input is allowed.
/// 2. **Collision resistance**: the milestone vector key and the
///    per-milestone approval key occupy distinct namespaces, so a
///    partially written approval can never corrupt the milestone vector.
/// 3. **Idempotency**: re-deriving a key after a failure is side-effect
///    free. Constructors must not mutate storage, emit events, or depend
///    on mutable global state.
/// 4. **Validation**: out-of-range or degenerate identifiers are rejected
///    at the boundary so that a failed call cannot silently write to a
///    wrong or shared slot. The error is deterministic and observable.
///
/// These invariants are what make retry and recovery safe: a caller that
/// fails after writing one key can always re-derive the same keys and
/// complete the remaining work without double-writing or losing data.

use soroban_sdk::{Env, Symbol};

use crate::types::DataKey;

/// Static symbol name used for the milestone vector key.
///
/// Keeping this as a constant guarantees that the symbol is identical across
/// all code paths and cannot drift between write and recovery reads.
const MILESTONE_SYMBOL: &str = "milestones";

/// Maximum number of milestones allowed per contract.
///
/// This bound is part of the deterministic contract: any index at or above
/// this value is rejected before a key is ever constructed, so a failed
/// call cannot accidentally target a shared or unbounded slot.
const MAX_MILESTONES: u32 = 1_000;

/// Error returned when a key constructor is given an invalid identifier.
///
/// The code is stable and documented so callers and operators can recognize
/// and recover from it deterministically.
const ERR_INVALID_KEY_INPUT: &str = "invalid key input";

/// Returns the persistent storage key tuple for a contract's milestones vector:
/// `(DataKey::Contract(contract_id), Symbol::new(env, "milestones"))`.
///
/// # Panics
///
/// Panics with `ERR_INVALID_KEY_INPUT` if `contract_id` is zero. A zero
/// contract id is not a valid identifier and allowing it would let a failed
/// call write into a shared, unintentional slot. Rejecting it at the
/// boundary keeps failure recovery deterministic and observable.
pub fn milestone_key(env: &Env, contract_id: u32) -> (DataKey, Symbol) {
    assert!(contract_id != 0, "{}", ERR_INVALID_KEY_INPUT);
    (DataKey::Contract(contract_id), milestone_symbol(env))
}

/// Returns the `Symbol` key for milestones: `"milestones"`.
///
/// The symbol is derived from a static constant so it is byte-identical on
/// every invocation, including recovery re-reads after a partial failure.
pub fn milestone_symbol(env: &Env) -> Symbol {
    Symbol::new(env, MILESTONE_SYMBOLL)
}

/// Returns the temporary storage key for milestone release approvals:
/// `DataKey::MilestoneApprovals(contract_id, milestone_index)`.
///
/// # Panics
///
/// Panics with `ERR_INVALID_KEY_INPUT` if `contract_id` is zero or if
/// `milestone_index` is greater than or equal to `MAX_MILESTONES`. This
/// ensures a failed call cannot silently target a shared or out-of-bounds
/// approval slot, and that retries re-derive the same key.
pub fn milestone_approval_key(contract_id: u32, milestone_index: u32) -> DataKey {
    assert!(contract_id != 0, "{}", ERR_INVALID_KEY_INPUT);
    assert!(
        milestone_index < MAX_MILESTONES,
        "{}",
        ERR_INVALID_KEY_INPUT
    );
    DataKey::MilestoneApprovals(contract_id, milestone_index)
}

/// Returns the maximum number of milestones allowed per contract.
///
/// Exposed so callers can validate inputs before invoking a key constructor
/// and so the bound is observable and testable without duplicating it.
pub fn max_milestones() -> u32 {
    MAX_MILESTONES
}

/// Returns the stable, user-facing error message for invalid key inputs.
///
/// The message is deliberately free of any identifier or sensitive data so
/// it can be surfaced in errors and logs without leaking internal state.
pub fn invalid_key_input_message() -> &'static str {
    ERR_INVALID_KEY_INPUT
}

/// Returns `true` if the given identifiers form a valid milestone approval
/// key input pair, and `false` otherwise.
///
/// This is a non-panicking companion to `milestone_approval_key` for callers
/// that want to validate inputs and report a deterministic error without
/// relying on panic behavior. It must remain exactly consistent with the
/// assertions in the key constructors.
pub fn is_valid_milestone_approval_input(contract_id: u32, milestone_index: u32) -> bool {
    contract_id != 0 && milestone_index < MAX_MILESTONES
}
