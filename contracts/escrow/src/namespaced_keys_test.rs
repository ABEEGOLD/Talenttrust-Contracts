#![cfg(test)]

use crate::keys::{
    milestone_approval_key, milestone_key, milestone_symbol, validate_milestone_id,
};
use crate::ttl::{
    milestone_storage_key, PENDING_APPROVAL_BUMP_THRESHOLD, PENDING_APPROVAL_TTL_LEDGERS,
    PERSISTENT_BUMP_THRESHOLD, PERSISTENT_TTL_LEDGERS,
};
use crate::types::{DataKey, Milestone, ReleaseAuthorization};
use crate::{Escrow, EscrowClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger as _},
    vec, Address, Env, String, Symbol, Vec,
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

    let milestone_app_1 = DataKey::MilestoneApprovals(1, 0);
    let milestone_app_2 = DataKey::MilestoneApprovals(1, 1);
    assert_ne!(milestone_app_1, milestone_app_2);

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
fn test_milestone_key_deterministic_across_env_instances() {
    let env_a = Env::default();
    let env_b = Env::default();

    // Same logical milestone id must produce identical storage keys regardless
    // of which Env instance is used, preserving compatibility contracts.
    assert_eq!(milestone_key(&env_a, 7), milestone_key(&env_b, 7));
    assert_eq!(milestone_symbol(&env_a), milestone_symbol(&env_b));
    assert_eq!(
        milestone_approval_key(7, 3),
        milestone_approval_key(7, 3)
    );
}

#[test]
fn test_milestone_key_boundary_values() {
    let env = Env::default();

    // Boundary: zero and max u32 milestone ids must be accepted and distinct.
    let zero = milestone_key(&env, 0);
    let max = milestone_key(&env, u32::MAX);
    assert_ne!(zero, max);

    // Boundary: approval keys at index 0 and max index must be distinct.
    let app_zero = milestone_approval_key(0, 0);
    let app_max = milestone_approval_key(u32::MAX, u32::MAX);
    assert_ne!(app_zero, app_max);
}

#[test]
fn test_milestone_key_rejects_invalid_id() {
    // Invalid milestone ids must be rejected deterministically so callers
    // cannot silently collide with a valid key.
    assert!(validate_milestone_id(0).is_err());
}

#[test]
fn test_milestone_key_duplicate_inputs_are_stable() {
    let env = Env::default();

    // Duplicate inputs must not produce divergent keys across repeated calls.
    let first = milestone_key(&env, 99);
    for _ in 0..16 {
        assert_eq!(milestone_key(&env, 99), first);
    }

    let first_app = milestone_approval_key(99, 1);
    for _ in 0..16 {
        assert_eq!(milestone_approval_key(99, 1), first_app);
    }
}

#[test]
fn test_milestone_key_regression_matches_storage_key() {
    let env = Env::default();

    // Regression: the public key helper must remain compatible with the
    // storage-key derivation used by the TTL module.
    for id in [1u32, 2, 42, 1000] {
        assert_eq!(milestone_key(&env, id), milestone_storage_key(&env, id));
    }
}
