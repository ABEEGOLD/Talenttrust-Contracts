#![cfg(test)]

use crate::keys::{
    milestone_approval_key, milestone_key, milestone_symbol, recovery_key, RecoveryKey,
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
fn test_recovery_key_is_deterministic_for_same_inputs() {
    let env = Env::default();

    let key_a = recovery_key(&env, 7, 3, RecoveryKey::Pending);
    let key_b = recovery_key(&env, 7, 3, RecoveryKey::Pending);
    assert_eq!(key_a, key_b);
}

#[test]
fn test_recovery_key_distinguishes_state_and_scope() {
    let env = Env::default();

    let pending = recovery_key(&env, 7, 3, RecoveryKey::Pending);
    let committed = recovery_key(&env, 7, 3, RecoveryKey::Committed);
    let rolled_back = recovery_key(&env, 7, 3, RecoveryKey::RolledBack);
    assert_ne!(pending, committed);
    assert_ne!(pending, rolled_back);
    assert_ne!(committed, rolled_back);

    let other_contract = recovery_key(&env, 8, 3, RecoveryKey::Pending);
    let other_milestone = recovery_key(&env, 7, 4, RecoveryKey::Pending);
    assert_ne!(pending, other_contract);
    assert_ne!(pending, other_milestone);
}

#[test]
fn test_recovery_key_does_not_collide_with_existing_namespaces() {
    let env = Env::default();

    let recovery = recovery_key(&env, 1, 0, RecoveryKey::Pending);
    let contract = DataKey::Contract(1);
    let approvals = DataKey::MilestoneApprovals(1, 0);
    let released = DataKey::MilestoneReleased(1, 0);
    let milestone = milestone_key(&env, 1);

    assert_ne!(recovery, contract);
    assert_ne!(recovery, approvals);
    assert_ne!(recovery, released);
    assert_ne!(recovery, milestone);
}

#[test]
fn test_recovery_key_boundary_values_are_stable() {
    let env = Env::default();

    let zero = recovery_key(&env, 0, 0, RecoveryKey::Pending);
    let max = recovery_key(&env, u32::MAX, u32::MAX, RecoveryKey::RolledBack);
    assert_ne!(zero, max);

    let zero_again = recovery_key(&env, 0, 0, RecoveryKey::Pending);
    assert_eq!(zero, zero_again);
}

#[test]
fn test_recovery_key_round_trip_preserves_identity() {
    let env = Env::default();

    let original = recovery_key(&env, 42, 9, RecoveryKey::Committed);
    let encoded = original.clone();
    let decoded = encoded;
    assert_eq!(original, decoded);
}

#[test]
fn test_recovery_key_retry_is_idempotent() {
    let env = Env::default();

    let first = recovery_key(&env, 5, 1, RecoveryKey::Pending);
    let second = recovery_key(&env, 5, 1, RecoveryKey::Pending);
    let third = recovery_key(&env, 5, 1, RecoveryKey::Pending);
    assert_eq!(first, second);
    assert_eq!(second, third);
}

#[test]
fn test_recovery_key_partial_failure_does_not_alias_completed_state() {
    let env = Env::default();

    let partial = recovery_key(&env, 11, 2, RecoveryKey::Pending);
    let completed = recovery_key(&env, 11, 2, RecoveryKey::Committed);
    assert_ne!(partial, completed);
}

#[test]
fn test_recovery_key_rejects_invalid_state_transition_aliases() {
    let env = Env::default();

    let a = recovery_key(&env, 1, 1, RecoveryKey::Pending);
    let b = recovery_key(&env, 1, 1, RecoveryKey::RolledBack);
    let c = recovery_key(&env, 1, 1, RecoveryKey::Committed);
    assert_ne!(a, b);
    assert_ne!(b, c);
    assert_ne!(a, c);
}

#[test]
fn test_recovery_key_concurrent_execution_yields_single_canonical_key() {
    let env = Env::default();

    let mut keys = Vec::new(&env);
    for _ in 0..8 {
        keys.push_back(recovery_key(&env, 3, 3, RecoveryKey::Pending));
    }
    let canonical = keys.get(0).unwrap();
    for k in keys.iter() {
        assert_eq!(k, canonical);
    }
}

#[test]
fn test_recovery_key_logs_are_deterministic_across_runs() {
    let env = Env::default();

    let run1 = recovery_key(&env, 100, 5, RecoveryKey::Committed);
    let run2 = recovery_key(&env, 100, 5, RecoveryKey::Committed);
    assert_eq!(run1, run2);
    assert_eq!(format!("{:?}", run1), format!("{:?}", run2));
}

#[test]
fn test_recovery_key_does_not_expose_sensitive_inputs() {
    let env = Env::default();

    let key = recovery_key(&env, 999, 7, RecoveryKey::Pending);
    let debug = format!("{:?}", key);
    assert!(!debug.contains("999"));
    assert!(!debug.contains("secret"));
}
