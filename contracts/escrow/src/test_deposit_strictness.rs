#`!cfg(test)]

use crate::{
    types::{ContractStatus, DepositMode},
    EscrowClient, EscrowError,
};
use soroban_sdk::{testutils::[Address as _], Address, Env, Vec};

fn setup_env() -> (Env, EscrowClient<'static>, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, crate::Escrow);
    let client = EscrowClient::new(&env, &contract_id);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);

    (env, client, client_addr, freelancer_addr)
}

// -----------------------------------------------------------------------------
-// ExactTotal -- valid / invalid / duplicate / boundary
-// -----------------------------------------------------------------------------

#[test]
fn test_exact_total_accepts_exact_amount() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 2000]); // Total 3000

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None, // arbiter
        &milestones,
        &None, // terms_hash
        &None, // grace_period
        &DepositMode::ExactTotal,
    );

    let res = client.deposit_funds(&contract_id, &3000);
    assert!(res);

    let data = client.get_contract(&contract_id);
    assert_eq!(data.status, ContractStatus::Funded);
    assert_eq!(data.total_deposited, 3000);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_exact_total_rejects_partial_amount() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 2000]); // Total 3000

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &DepositMode::ExactTotal,
    );

    // Deposit 1000, should fail because ExactDepositRequired = 11
    client.deposit_funds(&contract_id, &1000);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_exact_total_rejects_overpayment() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 2000]); // Total 3000

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &DepositMode::ExactTotal,
    );

    // Overpayment must be rejected with the same exact-deposit error.
    client.deposit_funds(&contract_id, &3001);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_exact_total_rejects_zero_amount() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 2000]);

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &DepositMode::ExactTotal,
    );

    // Zero is an invalid deposit amount for any mode.
    client.deposit_funds(&contract_id, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_exact_total_rejects_duplicate_deposit() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 2000]);

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &DepositMode::ExactTotal,
    );

    // First deposit fully funds the contract.
    assert!(client.deposit_funds(&contract_id, &3000));

    // A second deposit on a funded contract must be rejected.
    client.deposit_funds(&contract_id, &3000);
}

// -----------------------------------------------------------------------------
// Incremental -- valid / invalid / duplicate / boundary
// -----------------------------------------------------------------------------

#[test]
fn test_incremental_accepts_multiple_deposits() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 2000]); // Total 3000

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &DepositMode::Incremental,
    );

    // First deposit 1000
    let res = client.deposit_funds(&contract_id, &1000);
    assert!(res);

    let data_partial = client.get_contract(&contract_id);
    assert_eq!(data_partial.status, ContractStatus::PartiallyFunded);
    assert_eq!(data_partial.total_deposited, 1000);

    // Second deposit 2000
    let res2 = client.deposit_funds(&contract_id, &2000);
    assert!(res2);

    let data_funded = client.get_contract(&contract_id);
    assert_eq!(data_funded.status, ContractStatus::Funded);
    assert_eq!(data_funded.total_deposited, 3000);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn test_incremental_rejects_overflow() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 2000]); // Total 3000

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &DepositMode::Incremental,
    );

    // Try to deposit 4000, should fail with DepositWouldExceedTotal = 12
    client.deposit_funds(&contract_id, &4000);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)"]
fn test_incremental_rejects_overflow_after_partial() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 2000]); // Total 3000

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &DepositMode::Incremental,
    );

    // Partial deposit of 2000 leaves 1000 remaining.
    assert!(client.deposit_funds(&contract_id, &2000));

    // Attempting to deposit 2000 more would exceed the total.
    client.deposit_funds(&contract_id, &2000);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)"]
fn test_incremental_rejects_zero_amount() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 2000]);

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &DepositMode::Incremental,
    );

    // Zero is not a valid incremental deposit.
    client.deposit_funds(&contract_id, &0);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn test_incremental_rejects_duplicate_after_funded() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 2000]); // Total 3000

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &DepositMode::Incremental,
    );

    // Fully fund the contract.
    assert!(client.deposit_funds(&contract_id, &3000));

    // Any further deposit would exceed total and must be rejected.
    client.deposit_funds(&contract_id, &1);
}

#[test]
fn test_incremental_accepts_exact_remaining_boundary() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 2000]); // Total 3000

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &DepositMode::Incremental,
    );

    // Partial deposit of 1000.
    assert!(client.deposit_funds(&contract_id, &1000));

    // Exact remaining amount of 2000 is the boundary and must succeed.
    assert!(client.deposit_funds(&contract_id, &2000));

    let data = client.get_contract(&contract_id);
    assert_eq!(data.status, ContractStatus::Funded);
    assert_eq!(data.total_deposited, 3000);
}

// -----------------------------------------------------------------------------
-// Cross-mode invariants:
-//   * depositing on an unknown contract must fail deterministically
-/   * the contract must not be mutated by a rejected deposit
-// -----------------------------------------------------------------------------

#[test]
#[should_panic]
fn test_deposit_rejects_unknown_contract() {
    let (env, client, _, _) = setup_env();

    // A non-existent contract id must not allow deposits.
    let unknown_id = Address::generate(&env);
    client.deposit_funds(&unknown_id, &1000);
}

#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_rejected_deposit_does_not_mutate_state() {
    let (env, client, client_addr, freelancer_addr) = setup_env();

    let milestones = Vec::from_array(&env, [1000, 200]); // Total 1200

    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &None,
        &None,
        &DepositMode::ExactTotal,
    );

    // Record the pre-deposit state.
    let before = client.get_contract(&contract_id);

    // This deposit is rejected because it is not the exact total.
    let result = client.try_deposit_funds(&contract_id, &1000);
    assert!(result.is_error());

    // State must be unchanged after the rejected deposit.
    let after = client.get_contract(&contract_id);
    assert_eq!(before.status, after.status);
    assert_eq!(before.total_deposited, after.total_deposited);
}
