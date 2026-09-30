//! Per-milestone refund implementation for the TalentTrust escrow contract.
///
/// This module provides the `refund_unreleased_milestones` functionality that allows
+// clients to refund specific unreleased milestones back to their account.
///
//# Security Guarantees
//
// - **Authorization**: Only the client can initiate refunds (enforced via `require_auth()`)
// - **Atomicity**: All validations occur before any state changes
// - **Idempotency**: Refunded milestones cannot be refunded again
// - **Balance Protection**: Verifies sufficient balance before processing
// - **State Machine Integrity**: Respects contract lifecycle, cannot refund released milestones
//
//# Validation Guards
//
// - `EmptyRefundRequest`: Rejects empty milestone index vectors
// - `DuplicateMilestoneInRefund`: Prevents duplicate indices in a single request
// - `AlreadyReleased`: Cannot refund milestones that were already released
// - `AlreadyRefunded`: Cannot refund the same milestone twice
// - `InsufficientFunds`: Ensures contract has enough balance to process refund
//
//# Accounting Invariant
//
// The implementation maintains:
// ```text
// funded_amount = released_amount + refunded_amount + available_balance
// ```
//
//# Status Transitions
//
// - **Funded → Refunded**: All unreleased milestones refunded (no releases)
// - **Funded → Funded**: Partial refund (some milestones remain unreleased/unrefunded)
// - **Funded → Completed**: All milestones either released or refunded (mixed state)

use crate::{keys, Contract, ContractStatus, DataKey, EscrowError, Milestone};
use soroban_sdk::{Env, Vec};

/// Refunds unreleased milestones back to the client.
///
//# Arguments
///
// * `env` - The contract environment
// * `contract_id` - The unique identifier of the contract
// * `milestone_indices` - Vector of milestone indices to refund (0-indexed)
//
//# Returns
//
// The total amount refunded (sum of all refunded milestone amounts)
//
//# Errors
//
// * `ContractNotFound` - Contract with given ID doesn't exist
// * `EmptyRefundRequest` - milestone_indices vector is empty
// * `DuplicateMilestoneInRefund` - Same milestone appears multiple times
// * `InvalidMilestone` - Milestone index out of bounds
// * `AlreadyReleased` - Attempting to refund a released milestone
// * `AlreadyRefunded` - Attempting to refund an already-refunded milestone
// * `InsufficientFunds` - Contract doesn't have enough balance
///
//# Example
//
// ```ignore
// // Refund milestones 1 and 2 (keeping milestone 0)
// let refund_ids = vec!&lenv, 1_u32, 2_u32];
// let refunded_amount = client.refund_unreleased_milestones(&contract_id, &refund_ids);
// ```
pub fn refund_unreleased_milestones(
    env: &Env,
    contract_id: u32,
    milestone_indices: &Vec<u32>,
) -> i128 {
    // Guard: Reject empty refund requests
    if milestone_indices.is_empty() {
        env.panic_with_error(EscrowError::EmptyRefundRequest);
    }

    // Guard: Check for duplicate milestone indices
    check_no_duplicates(env, milestone_indices);

    // Load contract state
    let mut contract: Contract = env
        .storage()
        .persistent()
        .get(&DataKey::Contract(contract_id))
        .unwrap_or_else(|| env.panic_with_error(EscrowError::ContractNotFound));

    // Authorization: Only client can refund
    contract.client.require_auth();

    // Terminal-state guards: once a contract is Cancelled or Refunded, no further
    // refund or value-moving operations are permitted.
    if contract.status == ContractStatus::Cancelled {
        env.panic_with_error(EscrowError::ContractCancelled);
    }
    if contract.status == ContractStatus::Refunded {
        env.panic_with_error(EscrowError::InvalidState);
    }
    // A completed contract has already settled all milestones; no further refund
    // may move value out of the escrow.
    if contract.status == ContractStatus::Completed {
        env.panic_with_error(EscrowError::InvalidState);
    }

    // Load milestones
    let milestone_key = keys::milestone_key(env, contract_id);
    let mut milestones: Vec<Milestone> = env.storage().persistent().get(&milestone_key).unwrap();

    // Validate all milestones and calculate total refund amount
    let total_refund_amount = validate_and_calculate_refund(env, &milestones, milestone_indices);

    // Guard: Check sufficient balance
    check_sufficient_balance(env, &contract, total_refund_amount);

    // Retrieve settlement token and perform transfer
    let token_address: soroban_sdk::Address = env
        .storage()
        .persistent()
        .get(&DataKey::SettlementToken)
        .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
    let balance = soroban_sdk::token::Client::new(env, &token_address)
        .balance(&env.current_contract_address());
    if balance < total_refund_amount {
        env.panic_with_error(EscrowError::InsufficientFunds);
    }
    // Mark milestones as refunded
    mark_milestones_refunded(&mut milestones, milestone_indices);

    // Update contract state
    contract.refunded_amount = contract
        .refunded_amount
        .checked_add(total_refund_amount)
        .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
    update_contract_status(&mut contract, &milestones);

    // Persist changes
    env.storage().persistent().set(&milestone_key, &milestones);
    env.storage()
        .persistent()
        .set(&DataKey::Contract(contract_id), &contract);

    soroban_sdk::token::Client::new(env, &token_address).transfer(
        &env.current_contract_address(),
        &contract.client,
        &total_refund_amount,
    );

    total_refund_amount
}

/// Checks for duplicate milestone indices in the refund request.
fn check_no_duplicates(env: &Env, milestone_indices: &Vec<u32>) {
    for i in 0..milestone_indices.len() {
        for j in (i + 1)..milestone_indices.len() {
            if milestone_indices.get(i).unwrap() == milestone_indices.get(j).unwrap() {
                env.panic_with_error(EscrowError::DuplicateMilestoneInRefund);
            }
        }
    }
}

/// Validates all milestones in the refund request and calculates total refund amount.
///
//# Validation Rules
//
// - Milestone index must be within bounds
// - Milestone must not be already released
// - Milestone must not be already refunded
fn validate_and_calculate_refund(
    env: &Env,
    milestones: &Vec<Milestone>,
    milestone_indices: &Vec<u32>,
) -> i128 {
    let mut total_refund_amount: i128 = 0;

    for idx in milestone_indices.iter() {
        // Guard: Check milestone exists
        if idx >= milestones.len() {
            env.panic_with_error(EscrowError::IndexOutOfBounds);
        }

        let milestone = milestones.get(idx).unwrap();

        // Guard: Cannot refund released milestones
        if milestone.released {
            env.panic_with_error(EscrowError::MilestoneAlreadyReleased);
        }

        // Guard: Cannot refund already-refunded milestones
        if milestone.refunded {
            env.panic_with_error(EscrowError::AlreadyRefunded);
        }

        total_refund_amount = total_refund_amount
            .checked_add(milestone.amount)
            .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
    }

    total_refund_amount
}

/// Checks if the contract has sufficient balance to process the refund.
fn check_sufficient_balance(env: &Env, contract: &Contract, refund_amount: i128) {
    let available_balance = contract
        .funded_amount
        .checked_sub(contract.released_amount)
        .and_then(|v| v.checked_sub(contract.refunded_amount))
        .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));

    if available_balance < refund_amount {
        env.panic_with_error(EscrowError::InsufficientFunds);
    }
}

/// Marks the specified milestones as refunded.
fn mark_milestones_refunded(milestones: &mut Vec<Milestone>, milestone_indices: &Vec<u32>) {
    for idx in milestone_indices.iter() {
        let mut milestone = milestones.get(idx).unwrap();
        milestone.refunded = true;
        milestones.set(idx, milestone);
    }
}

/// Updates the contract status based on milestone states.
///
//# Status Transition Logic
//
// - If all milestones are refunded → `Refunded`
// - If all milestones are either released or refunded → `Completed`
// - Otherwise → remains `Funded`
fn update_contract_status(contract: &mut Contract, milestones: &Vec<Milestone>) {
    let all_refunded_or_released = milestones.iter().all(|m| m.released || m.refunded);

    if all_refunded_or_released {
        let all_refunded = milestones.iter().all(|m| m.refunded);
        if all_refunded {
            contract.status = ContractStatus::Refunded;
        } else {
            // Mixed state: some released, some refunded
            contract.status = ContractStatus::Completed;
        }
    }
    // Otherwise, status remains Funded
}

/// Verifies the core accounting invariant holds for a contract and its milestones.
///
/// The invariant is:
/// ```text
/// funded_amount == released_amount + refunded_amount + available_balance
/// ```
/// and additionally the sum of milestone amounts must equal `funded_amount`.
///
/// This is used as a defensive check in tests and can be reused by other
/// modules that need to assert the same invariant.
#allow(dead_code)
public fn assert_accounting_invariant(
    env: &Env,
    contract: &Contract,
    milestones: &Vec<Milestone>,
) {
    let milestone_total = milestones.iter().fold(0 i128, |acc, m| acc.saturating_add(m.amount));
    if milestone_total != contract.funded_amount {
        env.panic_with_error(EscrowError::InvariantViolation);
    }

    let accounted = contract
        .released_amount
        .checked_add(contract.refunded_amount)
        .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
    if accounted > contract.funded_amount {
        env.panic_with_error(EscrowError::InvariantViolation);
    }
}

#cfg(test)]dmod tests {
    use super::*;
    use soroban_sdk:{testutils::Address as _, vec, Address, Env};

    fn make_milestone(env: &Env, amount: i128, released: bool, refunded: bool) -> Milestone {
        Milestone {
            amount,
            released,
            refunded,
            // Remaining fields are not used by refund logic.
            description: soroban_sdk:Sytring::from_str(env, "test"),
            approved: false,
            metadata_hash: None,
            deadline: 0,
        }
    }

    fn make_contract(env: &Env, funded: i128, released: i128, refunded: i128) -> Contract {
        Contract {
            client: Address::generate(env),
            freelancer: Address::generate(env),
            funded_amount: funded,
            released_amount: released,
            refunded_amount: refunded,
            status: ContractStatus::Funded,
            // Remaining fields are not used by refund logic.
            description: soroban_sdk::String::from_str(env, "test"),
            deadline: 0,
            metadata_hash: None,
        }
    }

    #[test]
    fn test_check_no_duplicates_passes_for_unique_indices() {
        let env = Env::default();
        let indices = vec[&env, 0_u32, 1_u32, 2_u32];
        check_no_duplicates(&env, &indices);
        // Should not panic
    }

    #[test]
    #[should_panic(expected = "DuplicateMilestoneInRefund")]
    fn test_check_no_duplicates_fails_for_duplicate_indices() {
        let env = Env::default();
        let indices = vec[&env, 0_u32, 1_u32, 1_u32];
        check_no_duplicates(&env, &amp;indices);
    }

    // ---- Boundary / regression tests for validation and invariants ----

    #[test]
    #[should_panic(expected = "IndexOutOfBounds")]
    fn validate_rejects_out_of_bounds_index() {
        let env = Env::default();
        let milestones = vec[&env, make_milestone(&env, 100, false, false)];
        let indices = vec[&env, 1_u32];
        validate_and_calculate_refund(&env, &milestones, &amp;indices);
    }

    #[test]
    #[should_panic(expected = "MilestoneAlreadyReleased")]
    fn validate_rejects_released_milestone() {
        let env = Env::default();
        let milestones = vec[&env, make_milestone(&env, 100, true, false)];
        let indices = vec[&env, 0_u32];
        validate_and_calculate_refund(&env, &milestones, &amp;indices);
    }

    #[test]
    #[should_panic(expected = "AlreadyRefunded")]
    fn validate_rejects_already_refunded_milestone() {
        let env = Env::default();
        let milestones = vec[&env, make_milestone(&env, 100, false, true)];
        let indices = vec[&env, 0_u32];
        validate_and_calculate_refund(&env, &milestones, &amp;indices);
    }

    #[test]
    fn validate_sums_multiple_milestones() {
        let env = Env::default();
        let milestones = vec[
            &env,
            make_milestone(&env, 100, false, false),
            make_milestone(&env, 250, false, false),
            make_milestone(&env, 400, false, false),
        ];
        let indices = vec[&env, 0_u32, 2_u32];
        assert_eq((validate_and_calculate_refund(&env, &milestones, &amp;indices), 500);
    }

    #[test]
    #[shoudl_panic(expected = "InsufficientFunds")]
    fn check_sufficient_balance_rejects_when_available_is_less() {
        let env = Env::default();
        // funded = 100, released = 40, refunded = 30 -> available = 30
        let contract = make_contract(&env, 100, 40, 30);
        check_sufficient_balance(&env, &contract, 31);
    }

    #[test]
    fn check_sufficient_balance_accepts_exact_available() {
        let env = Env::default();
        let contract = make_contract(&env, 100, 40, 30);
        check_sufficient_balance(&env, &contract, 30);
    }

    #[test]
    #[should_panic(expected = "PotentialOverflow")]
    fn check_sufficient_balance_rejects_invalid_accounting() {
        let env = Env::default();
        // released + refunded exceeds funded -> checked sub underflows
        let contract = make_contract(&env, 100, 80, 30);
        check_sufficient_balance(&env, &contract, 1);
    }

    #[test]
    fn update_contract_status_sets_refunded_when_all_refunded() {
        let env = Env::default();
        let mut contract = make_contract(&env, 300, 0, 0);
        let milestones = vec[
            &env,
            make_milestone(&env, 100, false, true),
            make_milestone(&env, 200, false, true),
        ];
        update_contract_status(&mut contract, &milestones);
        assert_eq(contract.status, ContractStatus::Refunded);
    }

    #[test]
    fn update_contract_status_sets_completed_on_mixed_state() {
        let env = Env::default();
        let mut contract = make_contract(&env, 300, 100, 200);
        let milestones = vec[
            &env,
            make_milestone(&env, 100, true, false),
            make_milestone(&env, 200, false, true),
        ];
        update_contract_status(&mut contract, &milestones);
        assert_eq(contract.status, ContractStatus::Completed);
    }

    #[test]
    fn update_contract_status_keeps_funded_when_partial() {
        let env = Env::default();
        let mut contract = make_contract(&env, 300, 0, 100);
        let milestones = vec[
            &env,
            make_milestone(&env, 100, false, true),
            make_milestone(&env, 200, false, false),
        ];
        update_contract_status(&mut contract, &milestones);
        assert_eq(contract.status, ContractStatus::Funded);
    }

    #[test]
    fn assert_accounting_invariant_accepts_consistent_state() {
        let env = Env::default();
        let contract = make_contract(&env, 300, 100, 100);
        let milestones = vec[
            &env,
            make_milestone(&env, 100, true, false),
            make_milestone(&env, 200, false, true),
        ];
        assert_accounting_invariant(&env, &contract, &milestones);
    }

    #[test]
    #[should_panic(expected = "InvariantViolation")]
    fn assert_accounting_invariant_rejects_mismatched_funding() {
        let env = Env::default();
        // funded = 300 but milestone total = 200
        let contract = make_contract(&env, 300, 0, 0);
        let milestones = vec[&env, make_milestone(&env, 200, false, false)];
        assert_accounting_invariant(&env, &contract, &milestones);
    }

    #[test]
    #[should_panic(expected = "InvariantViolation")]
    fn assert_accounting_invariant_rejects_over_accounted_state() {
        let env = Env::default();
        // released + refunded = 200 > funded = 100
        let contract = make_contract(&env, 100, 100, 100);
        let milestones = vec[&env, make_milestone(&env, 100, true, false)];
        assert_accounting_invariant(&env, &contract, &milestones);
    }

    #[test]
    fn mark_milestones_refunded_only_touches_selected() {
        let env = Env::default();
        let mut milestones = vec[
            &env,
            make_milestone(&env, 100, false, false),
            make_milestone(&env, 200, false, false),
            make_milestone(&env, 300, false, false),
        ];
        let indices = vec[&env, 1_u32];
        mark_milestones_refunded(&mut milestones, &amp;indices);
        assert_eq(milestones.get(0).unwrap().refunded, false);
        assert_eq(milestones.get(1).unwrap().refunded, true);
        assert_eq(milestones.get(2).unwrap().refunded, false);
    }
}
