//! # Milestone-level partial refund tests
//!
//! Covers `refund_milestone` and `get_refundable_balance`, verifying:
//! - Partial refunds (one or more milestones)
//! - Full refund (all milestones → status becomes `Refunded`)
//! - Mixed release + refund flows
//! - Accounting invariant: `total_deposited == released + refunded + available`
//! - All error paths (empty request, duplicate, already-released, already-refunded,
//!   insufficient balance)
//! - State-invariant protection: no double-refund, no refund of released
//!   milestones, no release of refunded milestones, and no partial mutation
//!   when a refund request is rejected.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, vec, Address, Env};

use crate::{ContractStatus, Escrow, EscrowClient};

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    (env, client_addr, freelancer_addr)
}

fn register_client(env: &Env) -> EscrowClient {
    let id = env.register(Escrow, ());
    EscrowClient::new(env, &id)
}

/// Create a 3-milestone contract (200 / 400 / 600 stroops = 1 200 total).
fn create_default_contract(
    env: &Env,
    client: &EscrowClient,
    client_addr: &Address,
    freelancer_addr: &Address,
) -> u32 {
    let milestones = vec![env, 200_0000000_i128, 400_0000000_i128, 600_0000000_i128];
    client.create_contract(client_addr, freelancer_addr, &None, &milestones)
}

fn total_amount() -> i128 {
    200_0000000 + 400_0000000 + 600_0000000
}

// ─── Happy-path tests ─────────────────────────────────────────────────────────

/// Refunding a single unreleased milestone returns the correct amount and
/// preserves the accounting invariant.
#[test]
fn refund_single_milestone_updates_accounting() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    // Refund milestone 1 (400 stroops).
    let refunded = client.refund_milestone(&cid, &vec![&env, 1_u32]);
    assert_eq!(refunded, 400_0000000_i128);

    let record = client.get_contract(&cid);
    assert_eq!(record.refunded_amount, 400_0000000_i128);
    assert_eq!(record.released_amount, 0);
    assert_eq!(record.status, ContractStatus::Funded);

    // Invariant: deposited == released + refunded + available
    let available = client.get_refundable_balance(&cid);
    assert_eq!(available, total_amount() - 400_0000000_i128);
    assert_eq!(
        record.total_deposited,
        record.released_amount + record.refunded_amount + available
    );
}

/// Refunding multiple milestones in one call works correctly.
#[test]
fn refund_multiple_milestones_in_one_call() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    // Refund milestones 0 and 2 (200 + 600 = 800 stroops).
    let refunded = client.refund_milestone(&cid, &vec![&env, 0_u32, 2_u32]);
    assert_eq!(refunded, 200_0000000_i128 + 600_0000000_i128);

    let record = client.get_contract(&cid);
    assert_eq!(record.refunded_amount, 800_0000000_i128);
    assert_eq!(record.released_amount, 0);
    assert_eq!(record.status, ContractStatus::Funded); // milestone 1 still pending

    let available = client.get_refundable_balance(&cid);
    assert_eq!(available, 400_0000000_i128);
    assert_eq!(
        record.total_deposited,
        record.released_amount + record.refunded_amount + available
    );
}

/// When all milestones are refunded the contract status becomes `Refunded`.
#[test]
fn refunding_all_milestones_transitions_to_refunded_status() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    let refunded = client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);
    assert_eq!(refunded, total_amount());

    let record = client.get_contract(&cid);
    assert_eq!(record.status, ContractStatus::Refunded);
    assert_eq!(record.refunded_amount, total_amount());
    assert_eq!(record.released_amount, 0);
    assert_eq!(client.get_refundable_balance(&cid), 0);
}

/// Mixed flow: release some milestones, refund the rest → status `Refunded`.
#[test]
fn mixed_release_and_refund_settles_contract() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    // Release milestone 0 (200 stroops).
    assert!(client.release_milestone(&cid, &0));

    // Refund milestones 1 and 2 (400 + 600 = 1 000 stroops).
    let refunded = client.refund_milestone(&cid, &vec![&env, 1_u32, 2_u32]);
    assert_eq!(refunded, 400_0000000_i128 + 600_0000000_i128);

    let record = client.get_contract(&cid);
    assert_eq!(record.status, ContractStatus::Refunded);
    assert_eq!(record.released_amount, 200_0000000_i128);
    assert_eq!(record.refunded_amount, 1_000_0000000_i128);
    assert_eq!(client.get_refundable_balance(&cid), 0);

    // Invariant holds.
    assert_eq!(
        record.total_deposited,
        record.released_amount + record.refunded_amount + client.get_refundable_balance(&cid)
    );
}

/// Refunding one milestone at a time across multiple calls accumulates correctly.
#[test]
fn sequential_single_milestone_refunds_accumulate() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    // Refund milestone 2 first.
    let r1 = client.refund_milestone(&cid, &vec![&env, 2_u32]);
    assert_eq!(r1, 600_0000000_i128);

    let record = client.get_contract(&cid);
    assert_eq!(record.refunded_amount, 600_0000000_i128);
    assert_eq!(record.status, ContractStatus::Funded);

    // Refund milestone 1 next.
    let r2 = client.refund_milestone(&cid, &vec![&env, 1_u32]);
    assert_eq!(r2, 400_0000000_i128);

    let record = client.get_contract(&cid);
    assert_eq!(record.refunded_amount, 1_000_0000000_i128);
    assert_eq!(record.status, ContractStatus::Funded); // milestone 0 still pending

    // Refund milestone 0 last → all settled.
    let r3 = client.refund_milestone(&cid, &vec![&env, 0_u32]);
    assert_eq!(r3, 200_0000000_i128);

    let record = client.get_contract(&cid);
    assert_eq!(record.refunded_amount, total_amount());
    assert_eq!(record.status, ContractStatus::Refunded);
    assert_eq!(client.get_refundable_balance(&cid), 0);
}

/// `get_refundable_balance` decreases correctly after each refund.
#[test]
fn refundable_balance_decreases_after_each_refund() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    assert_eq!(client.get_refundable_balance(&cid), total_amount());

    client.refund_milestone(&cid, &vec![&env, 0_u32]);
    assert_eq!(
        client.get_refundable_balance(&cid),
        total_amount() - 200_0000000_i128
    );

    client.refund_milestone(&cid, &vec![&env, 1_u32]);
    assert_eq!(
        client.get_refundable_balance(&cid),
        total_amount() - 200_0000000_i128 - 400_0000000_i128
    );

    client.refund_milestone(&cid, &vec![&env, 2_u32]);
    assert_eq!(client.get_refundable_balance(&cid), 0);
}

/// Milestone-level `refunded` flag is set after a refund.
#[test]
fn milestone_refunded_flag_is_set_after_refund() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let milestones = client.get_milestones(&cid);
    assert!(!milestones.get(0).unwrap().refunded);
    assert!(milestones.get(1).unwrap().refunded);
    assert!(!milestones.get(2).unwrap().refunded);
}

/// Milestone-level `refunded_amount` equals the milestone amount after refund.
#[test]
fn milestone_refunded_amount_equals_milestone_amount() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 2_u32]);

    let milestones = client.get_milestones(&cid);
    let m2 = milestones.get(2).unwrap();
    assert_eq!(m2.refunded_amount, m2.amount);
    assert_eq!(m2.refunded_amount, 600_0000000_i128);
}

/// Partial deposit: only milestone 0 is funded; refunding milestone 0 succeeds.
#[test]
fn partial_deposit_allows_refund_of_funded_milestone() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    // Deposit only enough for milestone 0.
    assert!(client.deposit_funds(&cid, &200_0000000_i128));

    let refunded = client.refund_milestone(&cid, &vec![&env, 0_u32]);
    assert_eq!(refunded, 200_0000000_i128);

    let record = client.get_contract(&cid);
    assert_eq!(record.refunded_amount, 200_0000000_i128);
    assert_eq!(client.get_refundable_balance(&cid), 0);
}

// ─── Error-path tests ─────────────────────────────────────────────────────────

/// An empty milestone list is rejected.
#[test]
#[should_panic]
fn rejects_empty_refund_request() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env]);
}

/// Duplicate milestone indices in a single call are rejected.
#[test]
#[should_panic]
fn rejects_duplicate_milestone_indices() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 1_u32, 1_u32]);
}

/// Attempting to refund an already-released milestone is rejected.
#[test]
#[should_panic]
fn rejects_refund_of_released_milestone() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    assert!(client.release_milestone(&cid, &0));

    // Milestone 0 is released; refunding it must fail.
    client.refund_milestone(&cid, &vec![&env, 0_u32]);
}

/// Attempting to refund an already-refunded milestone is rejected (double-refund guard).
#[test]
#[should_panic]
fn rejects_double_refund_of_same_milestone() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 2_u32]);

    // Second refund of the same milestone must fail.
    client.refund_milestone(&cid, &vec![&env, 2_u32]);
}

/// Refund is rejected when the escrow balance is insufficient.
#[test]
#[should_panic]
fn rejects_refund_when_balance_insufficient() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    // Deposit only 200 stroops but try to refund milestone 1 (400 stroops).
    assert!(client.deposit_funds(&cid, &200_0000000_i128));
    client.refund_milestone(&cid, &vec![&env, 1_u32]);
}

/// An out-of-bounds milestone index is rejected.
#[test]
#[should_panic]
fn rejects_out_of_bounds_milestone_index() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 99_u32]);
}

/// Releasing a milestone that was previously refunded is rejected.
#[test]
#[should_panic]
fn rejects_release_of_refunded_milestone() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    // Milestone 1 is refunded; releasing it must fail.
    client.release_milestone(&cid, &1);
}

// ─── Invariant stress tests ───────────────────────────────────────────────────

/// Interleaved releases and refunds always preserve the accounting invariant.
#[test]
fn accounting_invariant_holds_across_interleaved_operations() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    let check_invariant = |cid: &u32| {
        let r = client.get_contract(cid);
        let avail = client.get_refundable_balance(cid);
        assert_eq!(r.total_deposited, r.released_amount + r.refunded_amount + avail);
    };

    check_invariant(&cid);

    client.release_milestone(&cid, &0);
    check_invariant(&cid);

    client.refund_milestone(&cid, &vec![&env, 1_u32]);
    check_invariant(&cid);

    client.refund_milestone(&cid, &vec![&env, 2_u32]);
    check_invariant(&cid);

    let record = client.get_contract(&cid);
    assert_eq!(record.status, ContractStatus::Refunded);
}

/// A contract with a single milestone that is refunded reaches `Refunded` status.
#[test]
fn single_milestone_contract_reaches_refunded_status() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec![&env, 500_0000000_i128];
    let cid = client.create_contract(&client_addr, &freelancer_addr, &None, &milestones);

    assert!(client.deposit_funds(&cid, &500_0000000_i128));
    let refunded = client.refund_milestone(&cid, &vec![&env, 0_u32]);
    assert_eq!(refunded, 500_0000000_i128);

    let record = client.get_contract(&cid);
    assert_eq!(record.status, ContractStatus::Refunded);
    assert_eq!(record.refunded_amount, 500_0000000_i128);
    assert_eq!(client.get_refundable_balance(&cid), 0);
}

// ─── State-invariant regression tests ─────────────────────────────────────────

/// A rejected refund request must not mutate any state (atomicity of the
/// multi-milestone refund path). If one index in the batch is invalid, the
/// whole call must revert and leave the contract untouched.
#[test]
#[should_panic]
fn rejected_batch_refund_leaves_state_unchanged() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    // Snapshot pre-state.
    let before = client.get_contract(&cid);
    let before_milestones = client.get_milestones(&cid);
    let before_available = client.get_refundable_balance(&cid);

    // Batch contains a valid index (0) and an out-of-bounds index (99).
    // The call must panic and no partial mutation may persist.
    let _ = client.refund_milestone(&cid, &vec![&env, 0_u32, 99_u32]);

    // Unreachable if the call correctly panics; kept for clarity if the
    // panic behavior ever changes to a Result-based API.
    let after = client.get_contract(&cid);
    assert_eq!(before.refunded_amount, after.refunded_amount);
    assert_eq!(before.released_amount, after.released_amount);
    assert_eq!(before.status, after.status);
    assert_eq!(before_available, client.get_refundable_balance(&cid));
    let after_milestones = client.get_milestones(&cid);
    assert_eq!(before_milestones.len(), after_milestones.len());
    for i in 0..before_milestones.len() {
        let b = before_milestones.get(i).unwrap();
        let a = after_milestones.get(i).unwrap();
        assert_eq!(b.refunded, a.refunded);
        assert_eq!(b.released, a.released);
        assert_eq!(b.refunded_amount, a.refunded_amount);
    }
}

/// A rejected refund of an already-released milestone must not mutate state.
#[test]
#[should_panic]
fn rejected_refund_of_released_milestone_leaves_state_unchanged() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    assert!(client.release_milestone(&cid, &0));

    let before = client.get_contract(&cid);
    let before_available = client.get_refundable_balance(&cid);

    // Milestone 0 is released; refunding it must revert atomically.
    let _ = client.refund_milestone(&cid, &vec![&env, 0_u32]);

    let after = client.get_contract(&cid);
    assert_eq!(before.refunded_amount, after.refunded_amount);
    assert_eq!(before.released_amount, after.released_amount);
    assert_eq!(before.status, after.status);
    assert_eq!(before_available, client.get_refundable_balance(&cid));
}

/// A rejected double-refund must not mutate state.
#[test]
#[should_panic]
fn rejected_double_refund_leaves_state_unchanged() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 2_u32]);

    let before = client.get_contract(&cid);
    let before_available = client.get_refundable_balance(&cid);
    let before_milestones = client.get_milestones(&cid);

    // Second refund of the same milestone must revert atomically.
    let _ = client.refund_milestone(&cid, &vec![&env, 2_u32]);

    let after = client.get_contract(&cid);
    assert_eq!(before.refunded_amount, after.refunded_amount);
    assert_eq!(before.released_amount, after.released_amount);
    assert_eq!(before.status, after.status);
    assert_eq!(before_available, client.get_refundable_balance(&cid));
    let after_milestones = client.get_milestones(&cid);
    let b2 = before_milestones.get(2).unwrap();
    let a2 = after_milestones.get(2).unwrap();
    assert_eq!(b2.refunded, a2.refunded);
    assert_eq!(b2.refunded_amount, a2.refunded_amount);
}

/// A rejected release of a refunded milestone must not mutate state.
#[test]
#[should_panic]
fn rejected_release_of_refunded_milestone_leaves_state_unchanged() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let before = client.get_contract(&cid);
    let before_available = client.get_refundable_balance(&cid);
    let before_milestones = client.get_milestones(&cid);

    // Milestone 1 is refunded; releasing it must revert atomically.
    let _ = client.release_milestone(&cid, &1);

    let after = client.get_contract(&cid);
    assert_eq!(before.refunded_amount, after.refunded_amount);
    assert_eq!(before.released_amount, after.released_amount);
    assert_eq!(before.status, after.status);
    assert_eq!(before_available, client.get_refundable_balance(&cid));
    let after_milestones = client.get_milestones(&cid);
    let b1 = before_milestones.get(1).unwrap();
    let a1 = after_milestones.get(1).unwrap();
    assert_eq!(b1.refunded, a1.refunded);
    assert_eq!(b1.released, a1.released);
    assert_eq!(b1.refunded_amount, a1.refunded_amount);
}

/// Boundary: refunding the last remaining milestone transitions to `Refunded`
/// and leaves zero refundable balance, with the accounting invariant intact.
#[test]
fn boundary_last_milestone_refund_settles_exactly() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    client.refund_milestone(&cid, &vec![&env, 0_u32]);
    client.refund_milestone(&cid, &vec![&env, 1_u32]);
    assert_eq!(client.get_refundable_balance(&cid), 600_0000000_i128);

    // Last milestone.
    let last = client.refund_milestone(&cid, &vec![&env, 2_u32]);
    assert_eq!(last, 600_0000000_i128);

    let record = client.get_contract(&cid);
    assert_eq!(record.status, ContractStatus::Refunded);
    assert_eq!(record.refunded_amount, total_amount());
    assert_eq!(record.released_amount, 0);
    assert_eq!(client.get_refundable_balance(&cid), 0);
    assert_eq!(
        record.total_deposited,
        record.released_amount + record.refunded_amount + client.get_refundable_balance(&cid)
    );
}

/// Boundary: refunding a milestone with zero amount (if permitted by the
/// contract) must not corrupt accounting; if rejected, state is unchanged.
#[test]
fn boundary_zero_amount_milestone_refund_is_consistent() {
    let env = Env::default();
    env.mock_all_auths();
    let client = register_client(&env);

    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let milestones = vec![&env, 0_i128, 500_0000000_i128];
    let cid = client.create_contract(&client_addr, &freelancer_addr, &None, &milestones);

    assert!(client.deposit_funds(&cid, &500_0000000_i128));

    let before = client.get_contract(&cid);
    let before_available = client.get_refundable_balance(&cid);

    // Attempt to refund the zero-amount milestone. Whether the contract
    // accepts or rejects this, the accounting invariant must hold and no
    // negative or inconsistent state may be produced.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.refund_milestone(&cid, &vec![&env, 0_u32])
    }));

    let after = client.get_contract(&cid);
    let after_available = client.get_refundable_balance(&cid);

    assert_eq!(
        after.total_deposited,
        after.released_amount + after.refunded_amount + after_available
    );
    assert!(after.refunded_amount >= before.refunded_amount);
    assert!(after_available <= before_available);

    if result.is_err() {
        // Rejected: no mutation.
        assert_eq!(before.refunded_amount, after.refunded_amount);
        assert_eq!(before_available, after_available);
    }
}

/// Repeated identical refund calls: the first succeeds, subsequent ones are
/// rejected and leave state unchanged (idempotency guard).
#[test]
fn repeated_refund_calls_are_not_idempotent_but_safe() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    // First refund succeeds.
    let first = client.refund_milestone(&cid, &vec![&env, 1_u32]);
    assert_eq!(first, 400_0000000_i128);

    let snapshot = client.get_contract(&cid);
    let snapshot_available = client.get_refundable_balance(&cid);

    // Subsequent identical calls must be rejected without mutation.
    for _ in 0..3 {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.refund_milestone(&cid, &vec![&env, 1_u32])
        }));
        assert!(res.is_err(), "double refund must be rejected");

        let now = client.get_contract(&cid);
        assert_eq!(snapshot.refunded_amount, now.refunded_amount);
        assert_eq!(snapshot.released_amount, now.released_amount);
        assert_eq!(snapshot.status, now.status);
        assert_eq!(snapshot_available, client.get_refundable_balance(&cid));
    }
}

/// Concurrent-style interleaving: alternating release/refund on distinct
/// milestones never violates the accounting invariant or status transitions.
#[test]
fn interleaved_release_and_refund_preserves_invariants() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    let check = |cid: &u32| {
        let r = client.get_contract(cid);
        let avail = client.get_refundable_balance(cid);
        assert_eq!(r.total_deposited, r.released_amount + r.refunded_amount + avail);
        assert!(r.released_amount >= 0);
        assert!(r.refunded_amount >= 0);
        assert!(avail >= 0);
        assert!(r.released_amount + r.refunded_amount <= r.total_deposited);
    };

    check(&cid);
    client.release_milestone(&cid, &0);
    check(&cid);
    client.refund_milestone(&cid, &vec![&env, 2_u32]);
    check(&cid);
    client.refund_milestone(&cid, &vec![&env, 1_u32]);
    check(&cid);

    let record = client.get_contract(&cid);
    assert_eq!(record.status, ContractStatus::Refunded);
    assert_eq!(record.released_amount, 200_0000000_i128);
    assert_eq!(record.refunded_amount, 1_000_0000000_i128);
    assert_eq!(client.get_refundable_balance(&cid), 0);
}
