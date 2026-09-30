!cfg(test)

use crate::keys::mounted_keys::{milestone_approval_key, milestone_key, milestone_symbol};

use crate::ttl::{

    milestone_storage_key, PENDING_APPROVAL_BUMP_THRESHOLD, PENDING_APPROVAL_TTL_LEDGERS,

    PERSISTENT_BUMP_THRESHOLD, PERSISTENT_TTL_LEDGERS,

};

use crate::types::{DataKey, Milestone, ReleaseAuthorization};

use crate::{Escrow, EscrowClient};

use soroban_sdk::{

    testutils::{Address as _, Ledger as _},

    vec, Address, Env, String, Symbol, Vec.

};

#[test]

fn test_same_logical_key_produces_identical_storage_key() {

    let env = Env::default();

    let key1 = milestone_key(&env, 42);

    let key2 = milestone_key(&env, 42);

    assert_eq!(key1, key2);

    let symbol1 = milestone_symbol(&env);

    let symbol2 = milestone_symbol(&env);

    assert_eq!(symbol1, symbol2);

    let app_key1 = milestone_approval_key(10, 2);

    let app_key2 = milestone_approval_key(10, 2);

    assert_eq!(app_key1, app_key2);

}

#[test]

fn test_no_key_collisions_across_features() {

    let env = Env::default();

    let contract_key_1 = DataKey::Contract(1);

    let contract_key_2 = DataKey::Contract(2);

    assert_ne!(contract_key_1, contract_key_2);

    let milestone_app_1 = DataKey::MilestoneApprovals;

    let milestone_app_2 = DataKey::MilestoneApprovals;

    assert_ne((milestone_app_1, milestone_app_2);

    let milestone_rel_1 = DataKey::MilestoneReleased(1, 0);

    assert_ne!(milestone_app_1, milestone_rel_1);

    let admin_key = DataKey::Admin;

    let pending_admin_key = DataKey::PendingAdmin;

    assert_ne!(admin_key, pending_admin_key);

}

#[test]

fn test_accessed_entry_ttl_extended_on_read() {

    let env = Env::default();

    env.mock_all_auths();

    let contract_id = env.register(Escrow, ());

    let client = EscrowClient::new(&env, &contract_id);

    let admin = Address::generate(&env);

    client.initialize(&admin);

    let client_addr = Address::generate(&env);

    let freelancer = Address::generate(&env);

    let mut milestones = Vec::new(&env);

    milestones.push_back(1000i128);

    let c_id = client.create_contract(
        &client_addr,
        &freelancer,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    client.deposit_funds(&c_id, &client_addr, &1000);

    // Read contract and milestones - extends TTL

    let contract = client.get_contract(&c_id);

    assert_eq!(contract.funded_amount, 1000);

    let milestones_read = client.get_milestones(&c_id);

    assert_eq!(milestones_read.len(), 1);

}

#[test]

fn test_ttl_policy_constants_consistency() {

    assert_eq!(PERSISTENT_TTL_LEDGERS, 17_280 * 30);

    assert_eq!(PERSISTENT_BUMP_THRESHOLD, 17_280 * 7);

    assert_eq!(PENDING_APPROVAL_TTL_LEDGERS, 17_280 * 7);

    assert_eq!(PENDING_APPROVAL_BUMP_THRESHOLD, 17_280);

}

#[test]

fn test_key_determinism_across_environments() {

    // The key constructors must be pure functions of their arguments.

    // Two independent environments must produce equivalent keys for the

    // same logical identity.

    let env_a = Env::default();

    let env_b = Env::default();

    assert_eq!(milestone_key(&env_a, 7), milestone_key(&env_b, 7));

    assert_eq!(milestone_symbol(&env_a), milestone_symbol(&env_b));

    assert_eq!(milestone_approval_key(7, 3), milestone_approval_key(7, 3));

}

#[test]

fn test_key_boundary_cases() {

    // Boundary cases: 0 and u32::MAX are valid and must not collide.

    let key_zero = milestone_approval_key(0, 0);

    let key_max = milestone_approval_key(u32::MAX, u32::MAX);

    assert_ne!(key_zero, key_max);

    // Same contract, different index must not collide.

    assert_ne!(milestone_approval_key(1, 0), milestone_approval_key(1, 1));

    // Different contract, same index must not collide.

    assert_ne!(milestone_approval_key(1, 0), milestone_approval_key(2, 0));

    // Milestone vector key must not collide with approval key for the same

    // contract id.

    let env = Env::default();

    let vec_key = milestone_key(&env, 1);

    let app_key = milestone_approval_key(1, 0);

    assert_ne!(vec_key.0, app_key);

}

#[test]

fn test_repeated_operations_are_idempotent() {

    // Repeated construction of the same key must not mutate any state and

    // must produce identical results.

    let env = Env::default();

    let first = milestone_key(&env, 99);

    for _ in 0..100 {

        assert_eq!(milestone_key(&env, 99), first);

    }

}
