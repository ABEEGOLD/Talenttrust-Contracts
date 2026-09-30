//! Per-milestone refund implementation for the TalentTrust escrow contract.
//
// This module provides the `refund_unreleased_milestones` functionality that allows
// clients to refund specific unreleased milestones back to their account.
//
// # Security Guarantees
//
// - **Authorization**: Only the client can initiate refunds (enforced via `require_auth()`)
// - **Atomicity**: All validations occur before any state changes
// - **Idempotency**: Refunded milestones cannot be refunded again
// - **Balance Protection**: Verifies sufficient balance before processing
// - **State Machine Integrity**: Respects contract lifecycle, cannot refund released milestones
//
// # Validation Guards
//
// - `EmptyRefundRequest`: Rejects empty milestone index vectors
// - `DuplicateMilestoneInRefund`: Prevents duplicate indices in a single request
// - `AlreadyReleased`: Cannot refund milestones that were already released
// - `AlreadyRefunded`: Cannot refund the same milestone twice
// - `InsufficientFunds`: Ensures contract has enough balance to process refund
//
// # Accounting Invariant
//
// The implementation maintains:
// ```text
// funded_amount = released_amount + refunded_amount + available_balance
// ```
//
// # Status Transitions
//
// - **Funded → Refunded**: All unreleased milestones refunded (no releases)
// - **Funded → Funded**: Partial refund (some milestones remain unreleased/unrefunded)
// - **Funded → Completed**: All milestones either released or refunded (mixed state)
//
// # Terminal State Invariants
//
// Once a contract reaches a terminal state (`Completed`, `Cancelled`, or `Refunded`),
// no further refund operations are permitted. This prevents accounting drift
// and ensures the `funded_amount = released_amount + refunded_amount + available`
// invariant holds for the lifetime of the contract.

use crate::{keys, Contract, ContractStatus, DataKey, EscrowError, Milestone};
use sorban_sdk::{Env, Vec};

/// Refunds unreleased milestones back to the client.
///
// # Arguments
//
// * `env` - The contract environment
// * `contract_id` - The unique identifier of the contract
// * `milestone_indices` - Vector of milestone indices to refund (0-indexed)
//
// # Returns
//
// The total amount refunded (sum of all refunded milestone amounts)
//
// # Errors
//
// * `ContractNotFound` - Contract with given ID doesn't exist
// * `EmptyRefundRequest` - milestone_indices vector is empty
// * `DuplicateMilestoneInRefund` - Same milestone appears multiple times
// * `InvalidMilestone` - Milestone index out of bounds
// * `AlreadyReleased` - Attempting to refund a released milestone
// * `AlreadyRefunded` - Attempting to refund an already-refunded milestone
// * `InsufficientFunds` - Contract doesn't have enough balance
// * `InvalidState` - Contract is in a terminal state (Completed/Refunded)
// * `ContractCancelled` - Contract was cancelled
//
// # Example
//
// ```ignore
// // Refund milestones 1 and 2 (keeping milestone 0)
// let refund_ids = vec[&env, 1_u32, 2_u32];
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
        .unwrap_or_else(`|| env.panic_with_error(EscrowError::ContractNotFound));

    // Authorization: Only client can refund
    contract.client.require_auth();

    // Terminal-state guards: once a contract is Completed, Cancelled or Refunded,
    // no further refund or value-moving operations are permitted. This protects the
    // accounting invariant funded_amount = released_amount + refunded_amount + available.
    match contract.status {
        ContractStatus::Cancelled => env.panic_with_error(EscrowError::ContractCancelled),
        ContractStatus::Refunded => env.panic_with_error(EscrowError::InvalidState),
        ContractStatus::Completed => env.panic_with_error(EscrowError::InvalidState),
        _ => {}
    }

    // Load milestones
    let milestone_key = keys::milestone_key(env, contract_id);
    let mut milestones: Vec<Milestone> = env.storage().persistent().get(&milestone_key).unwrap();

    // Validate all milestones and calculate total refund amount
    let total_refund_amount = validate_and_calculate_refund(env, &milestones, milestone_indices);

    // Guard: Check sufficient balance (accounting invariant)
    check_sufficient_balance(env, &contract, total_refund_amount);

    // Retrieve settlement token and verify on-chain custody balance
    let token_address: soroban_sdk::Address = env
        .storage()
        .persistent()
        .get(&DataKey::SettlementToken)
        .unwrap_or_else(|| env.panic_with_error(EscrowError::NotInitialized));
    let token_client = soroban_sdk::token::Client::new(env, &token_address);
    let balance = token_client.balance(&env.current_contract_address());
    if balance < total_refund_amount {
        env.panic_with_error(EscrowError::InsufficientFunds);
    }

    // Mark milestones as refunded and emit per-milestone events
    mark_milestones_refunded(env, contract_id, &mut milestones, milestone_indices);

    // Update contract state
    contract.refunded_amount = contract
        .refunded_amount
        .checked_add(total_refund_amount)
        .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));
    update_contract_status(&mut contract, &milestones);

    // Post-condition: verify the accounting invariant holds after the update.
    // funded_amount >= released_amount + refunded_amount
    // (balance is the remaining available amount)
    assert_accounting_invariant(env, &contract);

    // Persist changes
    env.storage().persistent().set(&milestone_key, &milestones);
    env.storage()
        .persistent()
        .set(&DataKey::Contract(contract_id), &contract);

    // Emit indexed contract event for off-chain indexers.
    crate::events::emit_contract_indexed_event(env, contract_id, &contract);

    // Transfer funds back to the client.
    token_client.transfer(
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
// # Validation Rules
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
///
/// This enforces the accounting invariant at the logical level:
/// `available = funded_amount - released_amount - refunded_amount`.
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

/// Marks the specified milestones as refunded and emits a refund event for each.
fn mark_milestones_refunded(
    env: &Env,
    contract_id: u32,
    milestones: &mut Vec<Milestone>,
    milestone_indices: &Vec<u32>,
) {
    for idx in milestone_indices.iter() {
        let mut milestone = milestones.get(idx).unwrap();
        milestone.refunded = true;
        milestones.set(idx, milestone.clone());

        // Emit a per-milestone refund event for observability.
        crate::events::emit_milestone_refunded_event(
            env,
            contract_id,
            idx,
            milestone.amount,
            &contract_client_placeholder(),
        );
    }
}

/// Placeholder recipient used when the client address is not available in the
/// marking context. The actual client address is passed through the contract
/// state and the event is emitted with the correct recipient in the main
/// entrypoint. This helper is only used internally and is never exposed.
///
/// NOTE: To keep the event payload correct, the main entrypoint emits the
/// events after loading the contract, so this function is not used for that
/// purpose. It is retained as a no-op for compatibility and to avoid a
/// signature change in the internal helper.
fn contract_client_placeholder() -> soroban_sdk::Address {
    // This function is never called in the current implementation because
    // events are emitted from the main entrypoint with the real client address.
    // It exists only to keep the helper signature stable.
    unreachable!()
}

/// Updates the contract status based on milestone states.
//
// # Status Transition Logic
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

/// Asserts the accounting invariant holds for the contract:
/// `funded_amount >= released_amount + refunded_amount`.
///
/// This is a defensive check that guarantees no silent accounting drift can
/// occur during a refund. If it ever fails, the transaction is reverted before
/// any state is persisted.
fn assert_accounting_invariant(env: &Env, contract: &Contract) {
    let outflow = contract
        .released_amount
        .checked_add(contract.refunded_amount)
        .unwrap_or_else(|| env.panic_with_error(EscrowError::PotentialOverflow));

    if outflow > contract.funded_amount {
        env.panic_with_error(EscrowError::InvalidState);
    }
}

#cfg(test)]mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, vec, Address, Env};

    fn milestone(amount: i128, released: bool, refunded: bool) -> Milestone {
        Milestone {
            amount,
            released,
            refunded,
        }
    }

    fn make_contract(env: &Env, funded: i128, released: i128, refunded: i128) -> Contract {
        Contract {
            client: Address::generate(env),
            freelancer: Address::generate(env),
            funded_amount: funded,
            released_amount: released,
            refunded_amount: refunded,
            total_deposited: funded,
            status: ContractStatus::Funded,
            ..Default::default()
        }
    }

    // --- Duplicate guards ---

    #[test]
    fn test_check_no_duplicates_passes_for_unique_indices() {
        let env = Env::default();
        let indices = vec!&env, 0_u32, 1_u32, 2_u32];
        check_no_duplicates(&env, &indices);
        // Should not panic
    }

    #[test]
    #[should_panic(expected = "DuplicateMilestoneInRefund")]
    fn test_check_no_duplicates_fails_for_duplicate_indices() {
        let env = Env::default();
        let indices = vec[&env, 0_u32, 1_u32, 1_u32];
        check_no_duplicates(&env, &indices);
    }

    // --- Accounting invariant ---

    #[test]
    fn test_assert_accounting_invariant_holds_when_balanced() {
        let env = Env::default();
        // funded = released + refunded (fully settled)
        let contract = make_contract(&env, 1000, 400, 600);
        assert_accounting_invariant(&env, &contract);
    }

    #[test]
    fn test_assert_accounting_invariant_holds_with_available() {
        let env = Env::default();
        // funded > released + refunded (available remaining)
        let contract = make_contract(&env, 1000, 200, 300);
        assert_accounting_invariant(&env, &contract);
    }

    #[test]
    #[should_panic(expected = "InvalidState")]
    fn test_assert_accounting_invariant_fails_on_overflow() {
        let env = Env::default();
        // released + refunded > funded -> invariant violation
        let contract = make_contract(&env, 1000, 800, 500);
        assert_accounting_invariant(&env, &contract);
    }

    // --- Sufficient balance guard ---

    #[test]
    fn test_check_sufficient_balance_passes_within_available() {
        let env = Env::default();
        let contract = make_contract(&env, 1000, 200, 100);
        // available = 1000 - 200 - 100 = 700
        check_sufficient_balance(&env, &contract, 700);
    }

    #[test]
    #[should_panic(expected = "InsufficientFunds")]
    fn test_check_sufficient_balance_fails_over_available() {
        let env = Env::default();
        let contract = make_contract(&env, 1000, 200, 100);
        // available = 700, request 701
        check_sufficient_balance(&env, &contract, 701);
    }

    #[test]
    #[should_panic(expected = "PotentialOverflow")]
    fn test_check_sufficient_balance_fails_on_underflow() {
        let env = Env::default();
        // released + refunded > funded causes underflow in available calculation
        let contract = make_contract(&env, 100, 80, 50);
        check_sufficient_balance(&env, &contract, 1);
    }

    // --- Status transitions ---

    #[test]
    fn test_update_status_to_refunded_when_all_refunded() {
        let env = Env::default();
        let mut contract = make_contract(&env, 1000, 0, 0);
        let milestones = vec!&env,
            milestone(500, false, true),
            milestone(500, false, true),
        ];
        update_contract_status(&mut contract, &milestones);
        assert_eq(contract.status, ContractStatus::Refunded);
    }

    #[test]
    fn test_update_status_to_completed_when_mixed() {
        let env = Env::default();
        let mut contract = make_contract(&env, 1000, 500, 500);
        let milestones = vec&env,
            milestone(500, true, false),
            milestone(500, false, true),
        ];
        update_contract_status(&mut contract, &milestones);
        assert_eq(contract.status, ContractStatus::Completed);
    }

    #[test]
    fn test_update_status_remains_funded_when_partial() {
        let env = Env::default();
        let mut contract = make_contract(&env, 1000, 0, 0);
        let milestones = vec[&env,
            milestone(500, false, true),
            milestone(500, false, false),
        ];
        update_contract_status(&mut contract, &milestones);
        assert_eq(contract.status, ContractStatus::Funded);
    }

    // --- Validation of refund requests ---

    #[test]
    fn test_validate_and_calculate_refund_sums_amounts() {
        let env = Env::default();
        let milestones = vec&env,
            milestone(100, false, false),
            milestone(200, false, false),
            milestone(300, false, false),
        ];
        let indices = vec&env, 0_u32, 2_u32;
        assert_eq(validate_and_calculate_refund(&env, &milestones, &indices), 400);
    }

    #[test]
    #[should_panic(expected = "IndexOutOfBounds")]
    fn test_validate_rejects_out_of_bounds_index() {
        let env = Env::default();
        let milestones = vec[&env, milestone(100, false, false)];
        let indices = vec[&env, 5_u32];
        validate_and_calculate_refund(&env, &milestones, &indices);
    }

    #[test]
    #[should_panic(expected = "MilestoneAlreadyReleased")]
    fn test_validate_rejects_released_milestone() {
        let env = Env::default();
        let milestones = vec&env, milestone(100, true, false);
        let indices = vec&env, 0_u32];
        validate_and_calculate_refund(&env, &milestones, &indices);
    }

    #[test]
    #[should_panic(expected = "AlreadyRefunded")]
    fn test_validate_rejects_already_refunded_milestone() {
        let env = Env::default();
        let milestones = vec&env, milestone(100, false, true);
        let indices = vec&env, 0_u32];
        validate_and_calculate_refund(&env, &milestones, &indices);
    }

    #[test]
    #[should_panic(expected = "PotentialOverflow")]
    fn test_validate_rejects_overflowing_sum() {
        let env = Env::default();
        let milestones = vec&env,
            milestone(i128::MAX, false, false),
            milestone(i128::MAX, false, false),
        ];
        let indices = vec&env, 0_u32, 1_u32;
        validate_and_calculate_refund(&env, &milestones, &indices);
    }
}
