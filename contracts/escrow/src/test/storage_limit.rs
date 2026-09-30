//! Tests for the admin-configurable storage limit (#901).
//! Tests for the admin-configurable storage limit (#901).
//!
//! Coverage matrix
//! ───────────────
//! * `get_storage_limit` returns `DEFAULT_STORAGE_LIMIT` before any admin call.
//! * `set_storage_limit` persists the value and `get_storage_limit` reflects it.
//! * In-bounds boundary values (MIN, MAX, DEFAULT) are accepted.
//! * Zero → `StorageLimitOutOfRange`.
//! * One above maximum → `StorageLimitOutOfRange`.
//! * Non-admin caller → `UnauthorizedRole`.
//! * Uninitialized contract → `NotInitialized`.
//! * Multiple sequential calls: last write wins.
//! * Event is emitted with the `"storage_limit"` topic.
//! * `get_storage_limit` is auth-free (no mock needed).

use super::assert_contract_error;
use soroban_sdk::testutils::Ledger;
use soroban_sdk::testutils::LedgerInfo;
use soroban_sdk::{
    testutils::{Address as _, Events},
    Address, Env, Symbol, TryFromVal,
};

use crate::{
    Error, Escrow, EscrowClient, DEFAULT_STORAGE_LIMIT, MAX_STORAGE_LIMIT, MIN_STORAGE_LIMIT,
};

use crate::storage::{get_storage_limit, set_storage_limit};

// ── Shared fixture ────────────────────────────────────────────────────────────

struct Ctx {
    env: Env,
    client_addr: Address,
    admin: Address,
}

impl Ctx {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(Escrow, ());
        let admin = Address::generate(&env);
        let client = EscrowClient::new(&env, &contract_id);
        client.initialize(&admin);
        Ctx {
            env,
            client_addr: contract_id,
            admin,
        }
    }

    fn escrow(&self) -> EscrowClient<'_> {
        EscrowClient::new(&self.env, &self.client_addr)
    }
}

// ── Validation boundary constants ─────────────────────────────────────────────

/// The storage limit is a `u32` measured in bytes. Valid values are
/// `MIN_STORAGE_LIMIT..=MAX_STORAGE_LIMIT`. `DEFAULT_STORAGE_LIMIT` is the
/// value returned before any admin call and must lie within the valid range.
///
/// Invariants enforced by `set_storage_limit`:
/// * `MIN_STORAGE_LIMIT >= 1` (zero is never a valid limit).
/// * `MAX_STORAGE_LIMIT > MIN_STORAGE_LIMIT`.
/// * `MIN_STORAGE_LIMIT <= DEFAULT_STORAGE_LIMIT <= MAX_STORAGE_LIMIT`.
/// * Rejected calls must not mutate stored state nor emit events.
/// * Only the initialized admin may mutate the limit.
/// * `get_storage_limit` is auth-free and never panics.

// ── Validation boundary helper ────────────────────────────────────────────────

/// Returns `true` iff `value` is within the inclusive valid range.
fn is_valid_storage_limit(value: u32) -> bool {
    value >= MIN_STORAGE_LIMIT && value <= MAX_STORAGE_LIMIT
}

// ── Default value ─────────────────────────────────────────────────────────────

#[test]
fn get_storage_limit_returns_default_before_any_set() {
    let ctx = Ctx::new();
    assert_eq!(ctx.escrow().get_storage_limit(), DEFAULT_STORAGE_LIMIT);
}

// ── Happy-path set / get ──────────────────────────────────────────────────────

#[test]
fn set_storage_limit_persists_and_get_reflects_it() {
    let ctx = Ctx::new();
    let new_limit: u32 = 128_000;
    assert!(ctx.escrow().set_storage_limit(&ctx.admin, &new_limit));
    assert_eq!(ctx.escrow().get_storage_limit(), new_limit);
}

#[test]
fn set_storage_limit_min_boundary_accepted() {
    let ctx = Ctx::new();
    assert!(ctx
        .escrow()
        .set_storage_limit(&ctx.admin, &MIN_STORAGE_LIMIT));
    assert_eq!(ctx.escrow().get_storage_limit(), MIN_STORAGE_LIMIT);
}

#[test]
fn set_storage_limit_max_boundary_accepted() {
    let ctx = Ctx::new();
    assert!(ctx
        .escrow()
        .set_storage_limit(&ctx.admin, &MAX_STORAGE_LIMIT));
    assert_eq!(ctx.escrow().get_storage_limit(), MAX_STORAGE_LIMIT);
}

#[test]
fn set_storage_limit_default_value_accepted() {
    let ctx = Ctx::new();
    // Explicit set to default must succeed (it's in-range)
    assert!(ctx
        .escrow()
        .set_storage_limit(&ctx.admin, &DEFAULT_STORAGE_LIMIT));
    assert_eq!(ctx.escrow().get_storage_limit(), DEFAULT_STORAGE_LIMIT);
}

// ── Rejection: out-of-range ───────────────────────────────────────────────────

#[test]
fn set_storage_limit_rejects_zero() {
    let ctx = Ctx::new();
    let result = ctx.escrow().try_set_storage_limit(&ctx.admin, &0u32);
    assert_contract_error(result, Error::StorageLimitOutOfRange);
}

#[test]
fn set_storage_limit_rejects_one_above_max() {
    let ctx = Ctx::new();
    let over_max = MAX_STORAGE_LIMIT + 1;
    let result = ctx.escrow().try_set_storage_limit(&ctx.admin, &over_max);
    assert_contract_error(result, Error::StorageLimitOutOfRange);
}

#[test]
fn set_storage_limit_rejects_u32_max() {
    let ctx = Ctx::new();
    let result = ctx.escrow().try_set_storage_limit(&ctx.admin, &u32::MAX);
    assert_contract_error(result, Error::StorageLimitOutOfRange);
}

#[test]
fn set_storage_limit_rejects_min_minus_one() {
    if MIN_STORAGE_LIMIT == 0 {
        return;
    }
    let ctx = Ctx::new();
    let below_min = MIN_STORAGE_LIMIT - 1;
    let result = ctx.escrow().try_set_storage_limit(&ctx.admin, &below_min);
    assert_contract_error(result, Error::StorageLimitOutOfRange);
}

#[test]
fn set_storage_limit_rejects_max_plus_one() {
    let ctx = Ctx::new();
    let over_max = MAX_STORAGE_LIMIT + 1;
    let result = ctx.escrow().try_set_storage_limit(&ctx.admin, &over_max);
    assert_contract_error(result, Error::StorageLimitOutOfRange);
}

// ── Rejection: wrong caller ───────────────────────────────────────────────────

#[test]
fn set_storage_limit_rejects_non_admin() {
    let ctx = Ctx::new();
    let impostor = Address::generate(&ctx.env);
    let result = ctx
        .escrow()
        .try_set_storage_limit(&impostor, &DEFAULT_STORAGE_LIMIT);
    assert_contract_error(result, Error::UnauthorizedRole);
}

// ── Rejection: uninitialized ──────────────────────────────────────────────────

#[test]
fn set_storage_limit_rejects_when_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);
    // Contract is NOT initialized — no `client.initialize(...)` call

    let admin = Address::generate(&env);
    let result = client.try_set_storage_limit(&admin, &DEFAULT_STORAGE_LIMIT);
    assert_contract_error(result, Error::NotInitialized);
}

// ── Multiple sequential calls ─────────────────────────────────────────────────

#[test]
fn set_storage_limit_last_write_wins() {
    let ctx = Ctx::new();
    ctx.escrow().set_storage_limit(&ctx.admin, &10_000u32);
    assert_eq!(ctx.escrow().get_storage_limit(), 10_000);

    ctx.escrow().set_storage_limit(&ctx.admin, &20_000u32);
    assert_eq!(ctx.escrow().get_storage_limit(), 20_000);

    ctx.escrow()
        .set_storage_limit(&ctx.admin, &MIN_STORAGE_LIMIT);
    assert_eq!(ctx.escrow().get_storage_limit(), MIN_STORAGE_LIMIT);
}

#[test]
fn set_storage_limit_same_value_twice_succeeds() {
    let ctx = Ctx::new();
    ctx.escrow().set_storage_limit(&ctx.admin, &50_000u32);
    // Setting the identical value again must not error
    assert!(ctx.escrow().set_storage_limit(&ctx.admin, &50_000u32));
    assert_eq!(ctx.escrow().get_storage_limit(), 50_000);
}

#[test]
fn set_storage_limit_duplicate_submission_is_idempotent() {
    let ctx = Ctx::new();
    let limit: u32 = 75_000;
    ctx.escrow().set_storage_limit(&ctx.admin, &limit);
    let first = ctx.escrow().get_storage_limit();
    ctx.escrow().set_storage_limit(&ctx.admin, &limit);
    let second = ctx.escrow().get_storage_limit();
    assert_eq!(first, second);
    assert_eq!(second, limit);
}

// ── Failed sets leave state unchanged ────────────────────────────────────────

#[test]
fn rejected_out_of_range_set_does_not_change_stored_value() {
    let ctx = Ctx::new();
    ctx.escrow().set_storage_limit(&ctx.admin, &100_000u32);

    // Attempt an out-of-range set (zero)
    let _ = ctx.escrow().try_set_storage_limit(&ctx.admin, &0u32);

    // Original value must be unchanged
    assert_eq!(ctx.escrow().get_storage_limit(), 100_000);
}

#[test]
fn rejected_non_admin_set_does_not_change_stored_value() {
    let ctx = Ctx::new();
    ctx.escrow().set_storage_limit(&ctx.admin, &100_000u32);

    let impostor = Address::generate(&ctx.env);
    let _ = ctx.escrow().try_set_storage_limit(&impostor, &200_000u32);

    assert_eq!(ctx.escrow().get_storage_limit(), 100_000);
}

// ── Auth-free read ────────────────────────────────────────────────────────────

#[test]
fn get_storage_limit_requires_no_auth() {
    // Deliberately omit mock_all_auths — get_storage_limit must not require auth
    let env = Env::default();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    // Should return the compile-time default without panicking
    assert_eq!(client.get_storage_limit(), DEFAULT_STORAGE_LIMIT);
}

// ── Event emission ────────────────────────────────────────────────────────────

#[test]
fn set_storage_limit_emits_storage_limit_event() {
    let ctx = Ctx::new();
    let new_limit: u32 = 200_000;
    ctx.escrow().set_storage_limit(&ctx.admin, &new_limit);

    let events = ctx.env.events().all();
    let topic = Symbol::new(&ctx.env, "storage_limit");
    let found = events.iter().any(|event| {
        !event.1.is_empty()
            && Symbol::try_from_val(&ctx.env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&topic)
    });
    assert!(
        found,
        "storage_limit event must be emitted on a successful set"
    );
}

#[test]
fn set_storage_limit_no_event_on_rejected_call() {
    let ctx = Ctx::new();
    // Trigger a rejection (zero is out of range)
    let _ = ctx.escrow().try_set_storage_limit(&ctx.admin, &0u32);

    let events = ctx.env.events().all();
    let topic = Symbol::new(&ctx.env, "storage_limit");
    let found = events.iter().any(|event| {
        !event.1.is_empty()
            && Symbol::try_from_val(&ctx.env, &event.1.get(0).unwrap())
                .ok()
                .as_ref()
                == Some(&topic)
    });
    assert!(
        !found,
        "no storage_limit event should be emitted when the call is rejected"
    );
}

// ── Boundary exactness ────────────────────────────────────────────────────────

#[test]
fn set_storage_limit_one_below_min_rejected() {
    if MIN_STORAGE_LIMIT == 0 {
        // MIN is already 0; nothing to test below it — skip
        return;
    }
    let ctx = Ctx::new();
    let below_min = MIN_STORAGE_LIMIT - 1;
    let result = ctx.escrow().try_set_storage_limit(&ctx.admin, &below_min);
    assert_contract_error(result, Error::StorageLimitOutOfRange);
}

#[test]
fn set_storage_limit_one_above_max_rejected_via_constant() {
    let ctx = Ctx::new();
    let result = ctx
        .escrow()
        .try_set_storage_limit(&ctx.admin, &(MAX_STORAGE_LIMIT + 1));
    assert_contract_error(result, Error::StorageLimitOutOfRange);
}

// ── Validation boundary helper consistency ────────────────────────────────────

#[test]
fn is_valid_storage_limit_matches_contract_acceptance() {
    let ctx = Ctx::new();
    let cases: [u32; 6] = [
        0,
        MIN_STORAGE_LIMIT,
        DEFAULT_STORAGE_LIMIT,
        MAX_STORAGE_LIMIT,
        MAX_STORAGE_LIMIT + 1,
        u32::MAX,
    ];
    for value in cases {
        let accepted = ctx
            .escrow()
            .try_set_storage_limit(&ctx.admin, &value)
            .is_ok();
        assert_eq!(
            accepted,
            is_valid_storage_limit(value),
            "helper disagrees with contract for value {value}"
        );
    }
}

// ── Constants ordering invariant ──────────────────────────────────────────────

#[test]
fn constants_satisfy_ordering_invariant() {
    assert!(
        MIN_STORAGE_LIMIT >= 1,
        "MIN_STORAGE_LIMIT must be at least 1"
    );
    assert!(
        MAX_STORAGE_LIMIT > MIN_STORAGE_LIMIT,
        "MAX_STORAGE_LIMIT must exceed MIN"
    );
    assert!(
        DEFAULT_STORAGE_LIMIT >= MIN_STORAGE_LIMIT,
        "DEFAULT must be >= MIN"
    );
    assert!(
        DEFAULT_STORAGE_LIMIT <= MAX_STORAGE_LIMIT,
        "DEFAULT must be <= MAX"
    );
}

// ── Direct storage module boundary tests ──────────────────────────────────────

#[test]
fn storage_module_set_and_get_roundtrip() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let admin = Address::generate(&env);
    let client = EscrowClient::new(&env, &contract_id);
    client.initialize(&admin);

    env.as_contract(&contract_id, || {
        set_storage_limit(&env, &admin, 123_456).unwrap();
        assert_eq!(get_storage_limit(&env), 123_456);
    });
}

#[test]
fn storage_module_rejects_out_of_range() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let admin = Address::generate(&env);
    let client = EscrowClient::new(&env, &contract_id);
    client.initialize(&admin);

    env.as_contract(&contract_id, || {
        let err = set_storage_limit(&env, &admin, 0).unwrap_err();
        assert_eq!(err, Error::StorageLimitOutOfRange);
        let err = set_storage_limit(&env, &admin, MAX_STORAGE_LIMIT + 1).unwrap_err();
        assert_eq!(err, Error::StorageLimitOutOfRange);
    });
}

#[test]
fn storage_module_rejects_non_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let admin = Address::generate(&env);
    let impostor = Address::generate(&env);
    let client = EscrowClient::new(&env, &contract_id);
    client.initialize(&admin);

    env.as_contract(&contract_id, || {
        let err = set_storage_limit(&env, &impostor, DEFAULT_STORAGE_LIMIT).unwrap_err();
        assert_eq!(err, Error::UnauthorizedRole);
    });
}

#[test]
fn storage_module_get_default_before_set() {
    let env = Env::default();
    let contract_id = env.register(Escrow, ());
    env.as_contract(&contract_id, || {
        assert_eq!(get_storage_limit(&env), DEFAULT_STORAGE_LIMIT);
    });
}

#[test]
fn storage_module_rejected_set_preserves_previous_value() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let admin = Address::generate(&env);
    let client = EscrowClient::new(&env, &contract_id);
    client.initialize(&admin);

    env.as_contract(&contract_id, || {
        set_storage_limit(&env, &admin, 100_000).unwrap();
        let _ = set_storage_limit(&env, &admin, 0);
        assert_eq!(get_storage_limit(&env), 100_000);
    });
}
