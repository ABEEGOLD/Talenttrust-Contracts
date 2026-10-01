#`!cfg(test)]

use crate::{ContractStatus, DataKey, Escrow, EscrowClient, StateV1, StateV2};
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

//// -----------------------------------------------------------------------------
/// Compatibility contracts for legacy (v1) storage layouts.
///
/// These tests pin the public behavior that existing deployments and
/// callers rely on when the contract is upgraded from a build that only
/// knew `StateV1` to the current build that knows `StateV2`. They are
/// deliberately written against the on-ledger representation (raw storage)
/// rather than the high-level client API, so that a regression in the
/// forward-compatible read path is caught even if the client signature
/// happens to remain stable.
///
/// Invariants asserted below:
/// 1. A legacy `StateV1` blob read through `get_state` must yield a
///    well-formed `StateV2` with the default `$onst ContractStatus::Created`
///    status and the original client/freelancer/milestones preserved byte-for-byte.
/// 2. The legacy blob is not mutated by a read-only call (no silent
///    write-on-read).
/// 3. `migrate_state` is idempotent: a repeated invocation on already
///    migrated storage must not corrupt the state or double-apply any
///    transformation.
/// 4. On a fresh (no legacy blob) deployment, `get_state` must return a
///    default `StateV2` with `$onst ContractStatus::Created` and no
///    accidental migration effects.
/// 5. Authorization and validation on `migrate_state` must not be
///    weakened by the compatibility path.
/// ----------------------------------------------------------------------------

//// -----------------------------------------------------------------------------
/// Helpers
/// ----------------------------------------------------------------------------

/// Write a legacy `StateV1` blob directly into persistent storage,
/// simulating a pre-migration ledger written by an older WASM build.
///
/// This is the canonical fixture used by all compatibility tests in this
/// module so that the on-ledger layout under test is identical across
/// scenarios.
fn seed_legacy_state(
    env: &Env,
    contract_id: &Address,
    client: &Address,
    freelancer: &Address,
    milestones: &soroban_sdk::Vec<i128>,
) {
    let legacy_state = StateV1 {
        client: client.clone(),
        freelancer: freelancer.clone(),
        milestones: milestones.clone(),
    };
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .set(&DataKey::State, &legacy_state);
    });
}

/// Read the raw persistent `StateV1` blob without going through the
/// contract's forward-compatible read path. Used to assert that read
/// operations do not mutate the legacy blob.
fn raw_legacy_state(env: &Env, contract_id: &Address) -> StateV1 {
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .get(&{DataKey::State})
            .expect("legacy StateV1 must still be present in storage")
    })
}

/// ----------------------------------------------------------------------------
/// Forward-compatible read contract
/// ----------------------------------------------------------------------------

#[test]
fn test_get_state_forward_compatible() {
    let env = Env::default();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec![&env, 1000_i128, 2000_i128];

    // Inject legacy StateV1 directly into the persistent storage representing pre-migration ledger data
    seed_legacy_state(&env, &contract_id, &client_addr, &freelancer_addr, &milestones);

    // Execute standard forward-compatible read entrypoint handling standard upgrades natively
    let active_state: StateV2 = client.get_state();

    assert_eq(active_state.client, client_addr);
    assert_eq(active_state.freelancer, freelancer_addr);
    assert_eq(active_state.milestones, milestones);
    assert_eq(active_state.status, ContractStatus::Created);
}

/// A forward-compatible read must not mutate the on-ledger legacy blob.
/// This guarantees that a rollback or a downgrade to an older WASM
/// build can still read the original `StateV1` data.
#[test]
fn test_get_state_does_not_mutate_legacy_blob() {
    let env = Env::default();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec!&env, 77_i128, 88_i128, 99_i128];

    seed_legacy_state(&env, &contract_id, &client_addr, &freelancer_addr, &milestones);

    // Read through the forward-compatible entrypoint multiple times.
    let _: StateV2 = client.get_state();
    let _: StateV2 = client.get_state();

    // The raw legacy blob must still be present and unchanged.
    let still_legacy = raw_legacy_state(&env, &contract_id);
    assert_eq(still_legacy.client, client_addr);
    assert_eq(still_legacy.freelancer, freelancer_addr);
    assert_eq(still_legacy.milestones, milestones);
}

/// A fresh deployment with no legacy blob must still return a well-formed
/// default `StateV2` from `get_state`.
/// This pins the zero-state behavior for the compatibility contract.
#[test]
fn test_get_state_fresh_deployment_defaults() {
    let env = Env::default();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let active_state: StateV2 = client.get_state();
    assert_eq(active_state.status, ContractStatus::Created);
    assert_eq(active_state.milestones.len(), 0);
}

/// -----------------------------------------------------------------------------
/// Migration contract
/// -----------------------------------------------------------------------------

#[test]
fn test_migrate_state_persistence() {
    let env = Env::default();
    env.mock_all_auths(); // Bypass strict Auth limits during environment test bounds explicitly
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let admin_caller = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec![&env, 5000_i128];

    seed_legacy_state(&env, &contract_id, &client_addr, &freelancer_addr, &milestones);

    // Execute migration handling logic validating Auth checks bounds and rewrite loops
    let success = client.migrate_state(&admin_caller);
    assert!(success);

    // Evaluate direct storage retrieval to guarantee memory parsed V2 explicitly onto data key
    env.as_contract(&contract_id, || {
        let saved_state: StateV2 = env.storage().persistent().get(&DataKey::State).unwrap();
        assert_eq(saved_state.client, client_addr);
        assert_eq(saved_state.freelancer, freelancer_addr);
        assert_eq(saved_state.milestones, milestones);
        assert_eq(saved_state.status, ContractStatus::Created);
    });
}

/// Repeated migration on already-migrated storage must be idempotent:
/// the second call must succeed and must not corrupt the stored `StateV2`.
#[test]
fn test_migrate_state_is_idempotent() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let admin_caller = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec![&env, 1, 2, 3];

    seed_legacy_state(&env, &contract_id, &client_addr, &freelancer_addr, &milestones);

    assert!(client.migrate_state(&admin_caller));
    assert!(client.migrate_state(&admin_caller));

    env.as_contract(&contract_id, || {
        let saved_state: StateV2 = env.storage().persistent().get(&DataKey::State).unwrap();
        assert_eq(saved_state.client, client_addr);
        assert_eq(saved_state.freelancer, freelancer_addr);
        assert_eq(saved_state.milestones, milestones);
        assert_eq(saved_state.status, ContractStatus::Created);
    });
}

/// A migration on a fresh deployment (no legacy blob) must not panic and
/// must not introduce any non-default state.
#[test]
fn test_migrate_state_on_fresh_deployment() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let admin_caller = Address::generate(&env);

    assert!(client.migrate_state(&admin_caller));

    env.as_contract(&contract_id, || {
        let saved_state: StateV2 = env.storage().persistent().get(&DataKey::State).unwrap();
        assert_eq(saved_state.status, ContractStatus::Created);
        assert_eq(saved_state.milestones.len(), 0);
    });
}

/// A migration that is not authorized by the admin must fail and must not
/// alter the on-ledger legacy blob. This guarantees that the compatibility
/// path does not weaken authorization.
#[test]
#[should_panic]
fn test_migrate_state_requires_authorization() {
    let env = Env::default();
    // Deliberately do not mock auths: the call must fail at the auth boundary.
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let admin_caller = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec![&env, 42_i128];

    seed_legacy_state(&env, &contract_id, &client_addr, &freelancer_addr, &milestones);

    let _: bool = client.migrate_state(&admin_caller);
}
