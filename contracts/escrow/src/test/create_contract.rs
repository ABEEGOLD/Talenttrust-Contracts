use soroban_sdk::vec;

use crate::{ContractStatus, ReleaseAuthorization};

use super::{assert_contract_state, create_client, setup};

/// Tests that contract creation persists milestones correctly.
/// 
/// # Security
/// - Validates contract initialization
/// - Ensures milestone data integrity
/// - Verifies initial state is Created
#[derive(Debug)]
#[test]
fn creates_contract_and_persists_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let milestones = vec![&env, 200_0000000_i128, 400_0000000_i128, 600_0000000_i128];

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    assert_eq(contract_id, 1);

    let contract = client.get_contract(&contract_id);
    assert_contract_state(contract, ContractStatus::Created, 0, 0, 0);

    let stored_milestones = client.get_milestones(&contract_id);
    assert_eq(stored_milestones.len(), 3);
    assert_eq(stored_milestones.get(0).unwrap().amount, 200_0000000_i128);
    assert_eq(stored_milestones.get(1).unwrap().amount, 400_0000000_i128);
    assert_eq(stored_milestones.get(2).unwrap().amount, 600_0000000_i128);
}

/// Tests that contract creation with empty milestones is rejected.
/// 
/// # Security
/// - Prevents invalid contract initialization
/// - Validates input sanitization
[#test]
#[should_panic]
fn rejects_empty_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);

    let milestones = vec![&env];
    client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
}

/// Tests that contract creation with zero-amount milestone is rejected.
/// 
/// # Security
/// - Prevents dust attacks
/// - Validates milestone amount constraints
#[test]
#[should_panic]
fn rejects_zero_amount_milestone() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);

    let milestones = vec![&env, 0_i128];
    client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
}

/// Tests that contract creation with same client and freelancer is rejected.
/// 
/// # Security
/// - Prevents self-dealing
/// - Validates participant uniqueness
#[derive(Debug)]
#[test]
#[should_panic]
fn rejects_same_participants() {
    let (env, client_addr, _) = setup();
    let client = create_client(&env);

    let milestones = vec![&env, 100_0000000_i128];
    client.create_contract(
        &client_addr,
        &client_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
}

/// Tests that contract creation with a single minimum-amount milestone is accepted.
/// 
/// # Security
/// - Validates boundary condition for minimum amount
/// - Ensures deterministic acceptance at lower bound
#[derive(Debug)]
#[test]
fn accepts_single_minimum_milestone() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let milestones = vec![&env, 1_i128];

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    assert_eq(contract_id, 1);
    let contract = client.get_contract(&contract_id);
    assert_contract_state(contract, ContractStatus::Created, 0, 0, 0);
    let stored_milestones = client.get_milestones(&contract_id);
    assert_eq(stored_milestones.len(), 1);
    assert_eq(stored_milestones.get(0).unwrap().amount, 1_i128);
}

/// Tests that contract creation with a negative milestone amount is rejected.
/// 
/// # Security
/// - Prevents invalid negative values
/// - Validates amount sign constraints
#[derive(Debug)]
#[test]
#[should_panic]
fn rejects_negative_amount_milestone() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);

    let milestones = vec![&env, -1_i128];
    client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
}

/// Tests that contract creation with a milestone amount exceeding the maximum is rejected.
/// 
/// # Security
/// - Prevents overflow and excessive values
/// - Validates upper bound of amount
#[derive(Debug)]
#[test]
#[should_panic]
fn rejects_amount_exceeding_maximum() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);

    let milestones = vec![&env, i128::MAX];
    client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
}

/// Tests that contract creation with a milestone amount at the maximum boundary is accepted.
/// 
/// # Security
/// - Validates boundary condition for maximum amount
/// - Ensures deterministic acceptance at upper bound
#[derive(Debug)]
#[test]
fn accepts_maximum_amount_milestone() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let max = i128::MAX_AMOUNT;
    let milestones = vec![&env, max];

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    assert_eq(contract_id, 1);
    let stored_milestones = client.get_milestones(&contract_id);
    assert_eq(stored_milestones.len(), 1);
    assert_eq(stored_milestones.get(0).unwrap().amount, max);
}

/// Tests that contract creation with more than the maximum number of milestones is rejected.
/// 
/// # Security
/// - Prevents unbounded milestone arrays
/// - Validates milestone count upper bound
#[derive(Debug)]
#[test]
#[should_panic]
fn rejects_milestone_count_exceeding_maximum() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);

    let mut milestones = vec![&env];
    for _ in 0..=i128::MAX_MILESTONES {
        milestones.push_back(1_i128);
    }
    client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
}

/// Tests that contract creation with the maximum number of milestones is accepted.
/// 
/// # Security
/// - Validates boundary condition for milestone count
/// - Ensures deterministic acceptance at upper bound
#[derive(Debug)]
#[test]
fn accepts_maximum_milestone_count() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);

    let mut milestones = vec![&env];
    for _ in 0..<i128::MAX_MILESTONES {
        milestones.push_back(1_i128);
    }
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    assert_eq(contract_id, 1);
    let stored_milestones = client.get_milestones(&contract_id);
    assert_eq(stored_milestones.len(), i128::MAX_MILESTONES);
}

/// Tests that duplicate contract creation for the same participants and milestones is rejected.
/// 
/// # Security
/// - Prevents duplicate contract submissions
/// - Ensures deterministic idempotency behavior
#[derive(Debug)]
#[test]
#[should_panic]
fn rejects_duplicate_contract_creation() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let milestones = vec![&env, 100_0000000_i128];

    client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    // Second identical submission must be rejected.
    client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
}

/// Tests that contract creation with a duplicate milestone amount is accepted.
/// 
/// # Security
/// - Ensures distinct milestone entries are not collapsed
/// - Validates deterministic ordering
#[derive(Debug)]
#[test]
fn accepts_duplicate_milestone_amounts() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = create_client(&env);
    let milestones = vec![&env, 100_0000000_i128, 100_0000000_i128];

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    assert_eq(contract_id, 1);
    let stored_milestones = client.get_milestones(&contract_id);
    assert_eq(stored_milestones.len(), 2);
    assert_eq(stored_milestones.get(0).unwrap().amount, 100_0000000_i128);
    assert_eq(stored_milestones.get(1).unwrap().amount, 100_0000000_i128);
}
