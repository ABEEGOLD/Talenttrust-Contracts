#`!cfg(test)]

use crate::{ContractStatus, DataKey, Escrow, EscrowClient, StateV1, StateV2};
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

/// Seeds legacy StateV1 into persistent storage to simulate pre-migration ledger data.
fn seed_legacy_state(env: &Env, contract_id: &Address, legacy_state: StateV1) {
    env.as_contract(contract_id, || {
        env.storage()
            .persistent()
            .set(&DataKey::State, &legacy_state);
    });
}

/// Reads the currently persisted StateV2 directly from storage.
fn read_persisted_state(env: &Env, contract_id: &Address) -> Option<StateV2> {
    env.as_contract(contract_id, || {
        env.storage().persistent().get(&DataKey::State)
    })
}

#[test]
fn test_get_state_forward_compatible() {
    let env = Env::default();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec[&env, 1000_i128, 2000_i128];

    // Inject legacy StateV1 directly into the persistent storage representing pre-migration ledger data
    let legacy_state = StateV1 {
        client: client_addr.clone(),
        freelancer: freelancer_addr.clone(),
        milestones: milestones.clone(),
    };
    seed_legacy_state(&env, &contract_id, legacy_state);

    // Execute standard forward-compatible read entrypoint handling standard upgrades natively
    let active_state: StateV2 = client.get_state();

    assert_eq!(active_state.client, client_addr);
    assert_eq!(active_state.freelancer, freelancer_addr);
    assert_eq!(active_state.status, ContractStatus::Created);
}

#[test]
fn test_migrate_state_persistence() {
    let env = Env::default();
    env.mock_all_auths(); // Bypass strict Auth limits during environment test bounds explicitly
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let admin_caller = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec[&env, 5000_i128];

    let legacy_state = StateV1 {
        client: client_addr.clone(),
        freelancer: freelancer_addr.clone(),
        milestones: milestones.clone(),
    };
    seed_legacy_state(&env, &contract_id, legacy_state);

    // Execute migration handling logic validating Auth checks bounds and rewrite loops
    let success = client.migrate_state(&admin_caller);
    assert!(success);

    // Evaluate direct storage retrieval to guarantee memory parsed V2 explicitly onto datakey
    let saved_state = read_persisted_state(&env, &contract_id)
        .expect("migration must persist a StateV2");
    assert_eq!(saved_state.status, ContractStatus::Created);
    assert_eq!(saved_state.client, client_addr);
    assert_eq!(saved_state.freelancer, freelancer_addr);
    assert_eq!(saved_state.milestones, milestones);
}

/// Regression: a second migration attempt on already-migrated state must fail deterministically without corrupting the persisted state.
#[test]
fn test_migrate_state_is_not_re_entrant() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let admin_caller = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec[&env, 750_i128];

    let legacy_state = StateV1 {
        client: client_addr.clone(),
        freelancer: freelancer_addr.clone(),
        milestones: milestones.clone(),
    };
    seed_legacy_state(&env, &contract_id, legacy_state);

    assert!(client.migrate_state(&admin_caller));

    // Second attempt must not silently succeed or corrupt the already-migrated state.
    let second = client.migrate_state(&admin_caller);
    assert!(!second);

    let saved_state = read_persisted_state(&env, &contract_id)
        .expect("state must remain persisted after failed re-migration");
    assert_eq!(saved_state.status, ContractStatus::Created);
    assert_eq!(saved_state.client, client_addr);
    assert_eq!(saved_state.freelancer, freelancer_addr);
    assert_eq!(saved_state.milestones, milestones);
}

/// Boundary: migration with no persisted state must fail deterministically and leave storage empty.
#[test]
fn test_migrate_state_missing_state_fails_cleanly() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let admin_caller = Address::generate(&env);

    let result = client.migrate_state(&admin_caller);
    assert!(!result);

    // No partial write must be left behind on failure.
    assert!(read_persisted_state(&env, &contract_id).is_none());
}

/// Recovery: after a failed migration on missing state, a later seed of legacy state can be migrated successfully.
#[test]
fn test_migrate_recovers_after_failure() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &contract_id);

    let admin_caller = Address::generate(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec[&env, 125_i128];

    // First attempt fails because no state is seeded.
    assert!(!client.migrate_state(&admin_caller));
    assert!(read_persisted_state(&env, &contract_id).is_none());

    // Recovery: seed legacy state and retry.
    let legacy_state = StateV1 {
        client: client_addr.clone(),
        freelancer: freelancer_addr.clone(),
        milestones: milestones.clone(),
    };
    seed_legacy_state(&env, &contract_id, legacy_state);

    assert!(client.migrate_state(&admin_caller));

    let saved_state = read_persisted_state(&env, &contract_id)
        .expect("recovery migration must persist StateV2");
    assert_eq!(saved_state.status, ContractStatus::Created);
    assert_eq!(saved_state.client, client_addr);
    assert_eq!(saved_state.freelancer, freelancer_addr);
    assert_eq!(saved_state.milestones, milestones);
}
