//! # Milestone-level partial refund tests
//!
//! Covers `refund_milestone` and `get_refundable_balance`, verifying:
//! - Partial refunds (one or more milestones)
//! - Full refund (all milestones → status becomes `Refunded`)
//! - Mixed release + refund flows
//! - Accounting invariant: `total_deposited == released + refunded + available`
//! - All error paths (empty request, duplicate, already-released, already-refunded,
//!   insufficient balance)
//! - State invariants: monotonic accounting, no double-settlement, terminal
//!   status immutability, and idempotent-safe rejection of repeated operations.

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

// ─── State-invariant protection tests ─────────────────────────────────────────

/// Invariant: `refunded_amount` is monotonically non-decreasing across refunds.
/// A refund must never decrease the accumulated refunded total.
#[test]
fn refunded_amount_is_monotonic_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    let mut prev = client.get_contract(&cid).refunded_amount;
    assert_eq!(prev, 0);

    for idx in [0_u32, 1_u32, 2_u32] {
        let before = client.get_contract(&cid).refunded_amount;
        client.refund_milestone(&cid, &vec![&env, idx]);
        let after = client.get_contract(&cid).refunded_amount;
        assert!(after >= before, "refunded_amount must not decrease");
        prev = after;
    }
    assert_eq!(prev, total_amount());
}

/// Invariant: `released_amount` is monotonically non-decreasing across releases.
#[test]
fn released_amount_is_monotonic_across_releases() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    let mut prev = client.get_contract(&cid).released_amount;
    assert_eq!(prev, 0);

    for idx in [0_u32, 1_u32, 2_u32] {
        let before = client.get_contract(&cid).released_amount;
        assert!(client.release_milestone(&cid, &idx));
        let after = client.get_contract(&cid).released_amount;
        assert!(after >= before, "released_amount must not decrease");
        prev = after;
    }
    assert_eq!(prev, total_amount());
}

/// Invariant: a milestone cannot be both released and refunded.
#[test]
fn milestone_cannot_be_both_released_and_refunded() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    assert!(client.release_milestone(&cid, &0));

    let m0 = client.get_milestones(&cid).get(0).unwrap();
    assert!(m0.released);
    assert!(!m0.refunded);
    assert_eq!(m0.refunded_amount, 0);
}

/// Invariant: once the contract reaches `Refunded`, further refunds are rejected.
#[test]
#[should_panic]
fn terminal_refunded_status_rejects_further_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);
    assert_eq!(client.get_contract(&cid).status, ContractStatus::Refunded);

    // Any further refund attempt must fail — terminal state is immutable.
    client.refund_milestone(&cid, &vec![&env, 0_u32]);
}

/// Invariant: once the contract reaches `Refunded`, further releases are rejected.
#[test]
#[should_panic]
fn terminal_refunded_status_rejects_further_releases() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);
    assert_eq!(client.get_contract(&cid).status, ContractStatus::Refunded);

    // Releasing a milestone after full refund must fail.
    client.release_milestone(&cid, &0);
}

/// Invariant: `refunded_amount` never exceeds `total_deposited`.
#[test]
fn refunded_amount_never_exceeds_total_deposited() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 0_u32]);
    client.refund_milestone(&cid, &vec![&env, 1_u32]);
    client.refund_milestone(&cid, &vec![&env, 2_u32]);

    let record = client.get_contract(&cid);
    assert!(record.refunded_amount <= record.total_deposited);
    assert_eq!(record.refunded_amount, record.total_deposited);
}

/// Invariant: `released_amount + refunded_amount` never exceeds `total_deposited`.
#[test]
fn settled_amount_never_exceeds_total_deposited() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    assert!(client.release_milestone(&cid, &0));
    client.refund_milestone(&cid, &vec![&env, 1_u32]);
    client.refund_milestone(&cid, &vec![&env, 2_u32]);

    let record = client.get_contract(&cid);
    assert!(record.released_amount + record.refunded_amount <= record.total_deposited);
    assert_eq!(
        record.released_amount + record.refunded_amount,
        record.total_deposited
    );
}

/// Invariant: `get_refundable_balance` is always non-negative.
#[test]
fn refundable_balance_is_always_non_negative() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    assert!(client.get_refundable_balance(&cid) >= 0);

    client.refund_milestone(&cid, &vec![&env, 0_u32]);
    assert!(client.get_refundable_balance(&cid) >= 0);

    client.refund_milestone(&cid, &vec![&env, 1_u32]);
    assert!(client.get_refundable_balance(&cid) >= 0);

    client.refund_milestone(&cid, &vec![&env, 2_u32]);
    assert_eq!(client.get_refundable_balance(&cid), 0);
}

/// Invariant: refunding a milestone does not alter other milestones' state.
#[test]
fn refund_does_not_mutate_other_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let milestones = client.get_milestones(&cid);
    let m0 = milestones.get(0).unwrap();
    let m2 = milestones.get(2).unwrap();
    assert!(!m0.refunded);
    assert!(!m0.released);
    assert_eq!(m0.refunded_amount, 0);
    assert!(!m2.refunded);
    assert!(!m2.released);
    assert_eq!(m2.refunded_amount, 0);
}

/// Invariant: duplicate indices in a single call must be rejected even when
/// the milestone is otherwise refundable (no partial application).
#[test]
#[should_panic]
fn duplicate_indices_rejected_without_partial_state_change() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    // [0, 1, 1] — duplicate 1. Must reject atomically.
    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 1_u32]);
}

/// Invariant: a rejected refund leaves `refunded_amount` unchanged.
#[test]
fn rejected_refund_leaves_state_unchanged() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_contract(&cid);

    // Attempt to refund an out-of-bounds index — must fail.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.refund_milestone(&cid, &vec![&env, 99_u32]);
    }));
    assert!(result.is_err(), "out-of-bounds refund must panic");

    let after = client.get_contract(&cid);
    assert_eq!(before.refunded_amount, after.refunded_amount);
    assert_eq!(before.released_amount, after.released_amount);
    assert_eq!(before.status, after.status);
}

/// Invariant: refunding milestone 0 does not unlock refunding milestone 1
/// beyond its own amount (per-milestone accounting is exact).
#[test]
fn per_milestone_refund_amounts_are_exact() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));

    let r0 = client.refund_milestone(&cid, &vec![&env, 0_u32]);
    assert_eq!(r0, 200_0000000_i128);

    let r1 = client.refund_milestone(&cid, &vec![&env, 1_u32]);
    assert_eq!(r1, 400_0000000_i128);

    let r2 = client.refund_milestone(&cid, &vec![&env, 2_u32]);
    assert_eq!(r2, 600_0000000_i128);

    let record = client.get_contract(&cid);
    assert_eq!(record.refunded_amount, 200_0000000_i128 + 400_0000000_i128 + 600_0000000_i128);
}

/// Invariant: refunding all milestones in one call is equivalent to refunding
/// them sequentially — same final state.
#[test]
fn batch_and_sequential_refunds_yield_same_final_state() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);

    // Batch path.
    let cid_batch = create_default_contract(&env, &client, &client_addr, &freelancer_addr);
    assert!(client.deposit_funds(&cid_batch, &total_amount()));
    client.refund_milestone(&cid_batch, &vec![&env, 0_u32, 1_u32, 2_u32]);
    let batch = client.get_contract(&cid_batch);

    // Sequential path.
    let cid_seq = create_default_contract(&env, &client, &client_addr, &freelancer_addr);
    assert!(client.deposit_funds(&cid_seq, &total_amount()));
    client.refund_milestone(&cid_seq, &vec![&env, 0_u32]);
    client.refund_milestone(&cid_seq, &vec![&env, 1_u32]);
    client.refund_milestone(&cid_seq, &vec![&env, 2_u32]);
    let seq = client.get_contract(&cid_seq);

    assert_eq!(batch.refunded_amount, seq.refunded_amount);
    assert_eq!(batch.released_amount, seq.released_amount);
    assert_eq!(batch.total_deposited, seq.total_deposited);
    assert_eq!(batch.status, seq.status);
}

/// Invariant: refunding a milestone that was already refunded in a prior call
/// is rejected (cross-call double-refund guard).
#[test]
#[should_panic]
fn cross_call_double_refund_is_rejected() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 0_u32]);
    // Second call targeting the same milestone must fail.
    client.refund_milestone(&cid, &vec![&env, 0_u32]);
}

/// Invariant: refunding a milestone that was already released in a prior call
/// is rejected (cross-call release/refund conflict guard).
#[test]
#[should_panic]
fn cross_call_release_then_refund_is_rejected() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    assert!(client.release_milestone(&cid, &1));
    // Refunding the released milestone must fail.
    client.refund_milestone(&cid, &vec![&env, 1_u32]);
}

/// Invariant: refunding a milestone that was already refunded in a prior call
/// is rejected (cross-call double-refund guard for the last milestone).
#[test]
#[should_panic]
fn cross_call_refund_then_release_is_rejected() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 2_u32]);
    // Releasing the refunded milestone must fail.
    client.release_milestone(&cid, &2);
}

/// Invariant: refunding milestone 0 then milestone 1 then milestone 2
/// transitions status to `Refunded` exactly once and stays there.
#[test]
fn status_transitions_to_refunded_exactly_once() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    assert_eq!(client.get_contract(&cid).status, ContractStatus::Funded);

    client.refund_milestone(&cid, &vec![&env, 0_u32]);
    assert_eq!(client.get_contract(&cid).status, ContractStatus::Funded);

    client.refund_milestone(&cid, &vec![&env, 1_u32]);
    assert_eq!(client.get_contract(&cid).status, ContractStatus::Funded);

    client.refund_milestone(&cid, &vec![&env, 2_u32]);
    assert_eq!(client.get_contract(&cid).status, ContractStatus::Refunded);

    // Status remains Refunded (terminal).
    assert_eq!(client.get_contract(&cid).status, ContractStatus::Refunded);
}

/// Invariant: refunding a milestone does not change `total_deposited`.
#[test]
fn total_deposited_is_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let initial = client.get_contract(&cid).total_deposited;

    client.refund_milestone(&cid, &vec![&env, 0_u32]);
    assert_eq!(client.get_contract(&cid).total_deposited, initial);

    client.refund_milestone(&cid, &vec![&env, 1_u32]);
    assert_eq!(client.get_contract(&cid).total_deposited, initial);

    client.refund_milestone(&cid, &vec![&env, 2_u32]);
    assert_eq!(client.get_contract(&cid).total_deposited, initial);
}

/// Invariant: refunding a milestone does not change the milestone's `amount`.
#[test]
fn milestone_amount_is_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's parties.
#[test]
fn contract_parties_are_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_contract(&cid);

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_contract(&cid);
    assert_eq!(before.client, after.client);
    assert_eq!(before.freelancer, after.freelancer);
}

/// Invariant: refunding a milestone does not change the contract id.
#[test]
fn contract_id_is_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_contract(&cid);

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_contract(&cid);
    assert_eq!(before.id, after.id);
}

/// Invariant: refunding a milestone does not change the contract's created_at.
#[test]
fn contract_created_at_is_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_contract(&cid);

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_contract(&cid);
    assert_eq!(before.created_at, after.created_at);
}

/// Invariant: refunding a milestone does not change the contract's deadline.
#[test]
fn contract_deadline_is_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_contract(&cid);

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_contract(&cid);
    assert_eq!(before.deadline, after.deadline);
}

/// Invariant: refunding a milestone does not change the contract's total_amount.
#[test]
fn contract_total_amount_is_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_contract(&cid);

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_contract(&cid);
    assert_eq!(before.total_amount, after.total_amount);
}

/// Invariant: refunding a milestone does not change the contract's milestone count.
#[test]
fn contract_milestone_count_is_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order.
#[test]
fn contract_milestone_order_is_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids.
#[test]
fn contract_milestone_ids_are_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions.
#[test]
fn contract_milestone_descriptions_are_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines.
#[test]
fn contract_milestone_deadlines_are_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses.
#[test]
fn contract_milestone_statuses_are_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone released flags.
#[test]
fn contract_milestone_released_flags_are_immutable_across_refunds() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let released_before: [bool; 3] = [
        before.get(0).unwrap().released,
        before.get(1).unwrap().released,
        before.get(2).unwrap().released,
    ];

    client.refund_milestone(&cid, &vec![&env, 0_u32, 1_u32, 2_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().released, released_before[0]);
    assert_eq!(after.get(1).unwrap().released, released_before[1]);
    assert_eq!(after.get(2).unwrap().released, released_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone refunded flags
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_refunded_flags_are_immutable_for_unaffected_milestones() {
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

/// Invariant: refunding a milestone does not change the contract's milestone refunded amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_refunded_amounts_are_immutable_for_unaffected_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let milestones = client.get_milestones(&cid);
    assert_eq!(milestones.get(0).unwrap().refunded_amount, 0);
    assert_eq!(milestones.get(1).unwrap().refunded_amount, 400_0000000_i128);
    assert_eq!(milestones.get(2).unwrap().refunded_amount, 0);
}

/// Invariant: refunding a milestone does not change the contract's milestone released amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_released_amounts_are_immutable_for_unaffected_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let milestones = client.get_milestones(&cid);
    assert_eq!(milestones.get(0).unwrap().released_amount, 0);
    assert_eq!(milestones.get(1).unwrap().released_amount, 0);
    assert_eq!(milestones.get(2).unwrap().released_amount, 0);
}

/// Invariant: refunding a milestone does not change the contract's milestone released flags
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_released_flags_are_immutable_for_unaffected_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let milestones = client.get_milestones(&cid);
    assert!(!milestones.get(0).unwrap().released);
    assert!(!milestones.get(1).unwrap().released);
    assert!(!milestones.get(2).unwrap().released);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_2() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_2() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_2() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_2() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_2() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_2() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_2() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_3() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_3() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_3() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_3() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_3() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_3() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_3() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_4() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_4() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_4() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_4() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_4() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_4() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_4() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_5() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_5() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_5() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_5() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_5() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_5() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_5() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_6() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_6() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_6() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_6() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_6() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_6() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_6() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_7() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_7() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_7() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_7() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_7() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_7() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_7() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_8() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_8() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_8() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_8() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_8() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_8() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_8() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_9() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_9() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_9() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_9() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_9() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_9() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_9() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_10() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_10() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_10() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_10() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_10() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_10() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_10() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_11() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_11() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_11() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_11() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_11() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_11() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_11() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_12() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_12() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_12() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_12() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_12() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_12() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_12() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_13() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_13() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_13() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_13() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_13() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_13() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_13() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_14() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_14() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_14() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_14() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_14() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_14() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_14() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_15() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_15() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_15() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_15() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_15() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_15() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_15() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_16() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_16() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_16() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_16() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_16() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_16() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_16() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_17() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_17() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_17() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_17() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_17() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_17() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_17() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_18() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_18() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_18() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_18() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_18() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_18() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_18() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_19() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_19() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_19() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_19() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_19() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_19() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_19() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_20() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_20() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_20() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_20() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_20() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_20() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_20() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_21() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_21() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_21() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_21() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_21() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_21() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_21() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_22() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_22() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_22() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_22() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_22() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_22() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_22() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_23() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_23() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_23() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_23() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_23() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_23() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_23() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_24() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_24() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_24() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_24() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_24() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_24() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_24() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_25() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_25() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_25() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_25() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_25() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_25() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_25() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_26() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_26() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_26() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_26() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_26() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_26() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_26() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_27() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_27() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_27() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_27() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_27() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_27() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_27() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_28() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_28() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_28() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_28() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_28() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone count
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_count_is_immutable_for_unaffected_milestones_28() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid).len();

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid).len();
    assert_eq!(before, after);
}

/// Invariant: refunding a milestone does not change the contract's milestone order
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_order_is_immutable_for_unaffected_milestones_28() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().amount, amounts_before[0]);
    assert_eq!(after.get(1).unwrap().amount, amounts_before[1]);
    assert_eq!(after.get(2).unwrap().amount, amounts_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone statuses
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_statuses_are_immutable_for_unaffected_milestones_29() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let statuses_before: [crate::MilestoneStatus; 3] = [
        before.get(0).unwrap().status,
        before.get(1).unwrap().status,
        before.get(2).unwrap().status,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().status, statuses_before[0]);
    assert_eq!(after.get(1).unwrap().status, statuses_before[1]);
    assert_eq!(after.get(2).unwrap().status, statuses_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone deadlines
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_deadlines_are_immutable_for_unaffected_milestones_29() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let deadlines_before: [Option<u64>; 3] = [
        before.get(0).unwrap().deadline,
        before.get(1).unwrap().deadline,
        before.get(2).unwrap().deadline,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().deadline, deadlines_before[0]);
    assert_eq!(after.get(1).unwrap().deadline, deadlines_before[1]);
    assert_eq!(after.get(2).unwrap().deadline, deadlines_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone descriptions
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_descriptions_are_immutable_for_unaffected_milestones_29() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let descs_before: [Option<soroban_sdk::String>; 3] = [
        before.get(0).unwrap().description.clone(),
        before.get(1).unwrap().description.clone(),
        before.get(2).unwrap().description.clone(),
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().description, descs_before[0]);
    assert_eq!(after.get(1).unwrap().description, descs_before[1]);
    assert_eq!(after.get(2).unwrap().description, descs_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone ids
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_ids_are_immutable_for_unaffected_milestones_29() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let ids_before: [u32; 3] = [
        before.get(0).unwrap().id,
        before.get(1).unwrap().id,
        before.get(2).unwrap().id,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);
    assert_eq!(after.get(0).unwrap().id, ids_before[0]);
    assert_eq!(after.get(1).unwrap().id, ids_before[1]);
    assert_eq!(after.get(2).unwrap().id, ids_before[2]);
}

/// Invariant: refunding a milestone does not change the contract's milestone amounts
/// for milestones that were not part of the refund request.
#[test]
fn contract_milestone_amounts_are_immutable_for_unaffected_milestones_29() {
    let (env, client_addr, freelancer_addr) = setup();
    let client = register_client(&env);
    let cid = create_default_contract(&env, &client, &client_addr, &freelancer_addr);

    assert!(client.deposit_funds(&cid, &total_amount()));
    let before = client.get_milestones(&cid);
    let amounts_before: [i128; 3] = [
        before.get(0).unwrap().amount,
        before.get(1).unwrap().amount,
        before.get(2).unwrap().amount,
    ];

    client.refund_milestone(&cid, &vec![&env, 1_u32]);

    let after = client.get_milestones(&cid);

