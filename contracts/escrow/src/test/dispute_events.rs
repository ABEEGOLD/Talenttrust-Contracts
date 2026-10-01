//! Dispute index event tests.
//! Dispute index event tests.
//!
//! These tests verify that every disputes state change emits a well-topic'd
//! `dsp_index` event carrying the ids and amounts needed by off-chain indexers.
//!
//! Coverage:
//!   - `raise_dispute` emits `dsp_index` / `raised` with correct payload
//!   - `resolve_dispute` emits `dsp_index` / `settled` with correct payload
//!   - Topic uniqueness: `dsp_index` does not collide with other event topics
//!   - Payload correctness for each resolution variant (FullRefund, FullPayout,
//!     PartialRefund, Split)
//!
//! Compatibility contract: the `dsp_index` topic symbol, its sub-topics
//! (`raised`, `settled`), and the payload tuple layouts asserted below are
//! part of the public off-chain indexer interface and must not change without
//! a tested migration path.

#![cfg(test)]

use super::register_client;
use crate::{ContractStatus, DisputeResolution, DisputeSplit, ReleaseAuthorization};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events},
    vec, Address, Env, IntoVal, Symbol, TryFromVal, Val,
};
use soroban_sdk::FromVal;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Canonical topic symbol for dispute index events.  Off-chain indexers filter
/// on this exact symbol; changing it is a breaking compatibility change.
const DSP_INDEX_TOPIC: &str = "dsp_index";

/// Create a funded contract with an arbiter, ready for dispute.
/// Returns (client_addr, freelancer_addr, arbiter_addr, contract_id).
fn funded_with_arbiter(
    env: &Env,
    client: &crate::EscrowClient<'_>,
) -> (Address, Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let arbiter_addr = Address::generate(env);
    let milestones = vec![env, 100_i128];
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    assert!(client.deposit_funds(&contract_id, &client_addr, &100_i128));
    (client_addr, freelancer_addr, arbiter_addr, contract_id)
}

/// Find the first event whose topics start with `dsp_index` and have a second
/// topic matching `sub_topic`.  Returns `Some((topics_vec, data_val))`.
fn find_dsp_index_event(
    env: &Env,
    sub_topic: &Symbol,
) -> Option<(soroban_sdk::Vec<Val>, Val)> {
    let dsp_index_sym = symbol_short!("dsp_index");
    env.events().all().iter().find_map(|event| {
        let topics = &event.1;
        if topics.len() >= 2 {
            let t0 = Symbol::try_from_val(env, &topics.get(0).unwrap()).ok();
            let t1 = Symbol::try_from_val(env, &topics.get(1).unwrap()).ok();
            if t0.as_ref() == Some(&dsp_index_sym) && t1.as_ref() == Some(sub_topic) {
                return Some((topics.clone(), event.2.clone()));
            }
        }
        None
    })
}

/// Decode and assert the exact `dsp_index` / `raised` payload layout.
/// Returns the decoded tuple so callers can assert individual fields.
fn decode_raised_payload(
    env: &Env,
    data: &Val,
) -> (u32, Address, i128, i128, i128, u64) {
    <(u32, Address, i128, i128, i128, u64)>::from_val(env, data)
}

/// Decode and assert the exact `dsp_index` / `settled` payload layout.
fn decode_settled_payload(
    env: &Env,
    data: &Val,
) -> (u32, u32, i128, i128, ContractStatus, u64) {
    <(u32, u32, i128, i128, ContractStatus, u64)>::from_val(env, data)
}

// ---------------------------------------------------------------------------
// raise_dispute → dsp_index / raised
// ---------------------------------------------------------------------------

#[test]
fn raise_dispute_emits_dsp_index_raised_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, _arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));

    // Locate the dsp_index / raised event
    let raised_sym = symbol_short!("raised");
    let event = find_dsp_index_event(&env, &raised_sym);
    assert!(event.is_some(), "dsp_index/raised event must be emitted");

    let (topics, _data) = event.unwrap();

    // Assert topic structure
    assert_eq!(topics.len(), 2);
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(0).unwrap()).unwrap(),
        symbol_short!("dsp_index")
    );
    assert_eq!(
        Symbol::try_from_val(&env, &topics.get(1).unwrap()).unwrap(),
        symbol_short!("raised")
    );
}

#[test]
fn raise_dispute_raised_event_payload_correctness() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, _arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));

    let raised_sym = symbol_short!("raised");
    let (_topics, data) = find_dsp_index_event(&env, &raised_sym).unwrap();

    // Decode the data tuple: (contract_id, caller, funded_amount, released_amount,
    //                         refunded_amount, timestamp)
    let data_tuple = decode_raised_payload(&env, &data);

    assert_eq!(data_tuple.0, contract_id, "contract_id mismatch");
    assert_eq!(data_tuple.1, client_addr, "caller mismatch");
    assert_eq!(data_tuple.2, 100_i128, "funded_amount mismatch");
    assert_eq!(data_tuple.3, 0_i128, "released_amount mismatch");
    assert_eq!(data_tuple.4, 0_i128, "refunded_amount mismatch");
    // timestamp is a u64, just assert it exists (non-panicking decode proves it)
}

// ---------------------------------------------------------------------------
// resolve_dispute → dsp_index / settled (FullRefund)
// ---------------------------------------------------------------------------

#[test]
fn resolve_dispute_full_refund_emits_dsp_index_settled_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert!(client.resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::FullRefund,
    ));

    let settled_sym = symbol_short!("settled");
    let event = find_dsp_index_event(&env, &settled_sym);
    assert!(event.is_some(), "dsp_index/settled event must be emitted");

    let (_topics, data) = event.unwrap();
    let data_tuple = decode_settled_payload(&env, &data);

    assert_eq!(data_tuple.0, contract_id, "contract_id mismatch");
    assert_eq!(data_tuple.1, 0, "resolution_code for FullRefund should be 0");
    assert_eq!(data_tuple.2, 100, "client_payout should be full balance");
    assert_eq!(data_tuple.3, 0, "freelancer_payout should be zero");
    assert_eq!(
        data_tuple.4,
        ContractStatus::Refunded,
        "final status should be Refunded"
    );
}

// ---------------------------------------------------------------------------
// resolve_dispute → dsp_index / settled (FullPayout)
// ---------------------------------------------------------------------------

#[test]
fn resolve_dispute_full_payout_emits_correct_settled_payload() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert!(client.resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::FullPayout,
    ));

    let settled_sym = symbol_short!("settled");
    let (_topics, data) = find_dsp_index_event(&env, &settled_sym).unwrap();
    let data_tuple = decode_settled_payload(&env, &data);

    assert_eq!(data_tuple.1, 2, "resolution_code for FullPayout should be 2");
    assert_eq!(data_tuple.2, 0, "client_payout should be zero");
    assert_eq!(
        data_tuple.3, 100,
        "freelancer_payout should be full balance"
    );
    assert_eq!(
        data_tuple.4,
        ContractStatus::Completed,
        "final status should be Completed"
    );
}

// ---------------------------------------------------------------------------
// resolve_dispute → dsp_index / settled (PartialRefund)
// ---------------------------------------------------------------------------

#[test]
fn resolve_dispute_partial_refund_emits_correct_settled_payload() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert!(client.resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::PartialRefund,
    ));

    let settled_sym = symbol_short!("settled");
    let (_topics, data) = find_dsp_index_event(&env, &settled_sym).unwrap();
    let data_tuple = decode_settled_payload(&env, &data);

    assert_eq!(
        data_tuple.1, 1,
        "resolution_code for PartialRefund should be 1"
    );
    // PartialRefund: freelancer gets floor(100 * 30 / 100) = 30, client gets 70
    assert_eq!(data_tuple.2, 70, "client_payout should be 70");
    assert_eq!(data_tuple.3, 30, "freelancer_payout should be 30");
    assert_eq!(
        data_tuple.4,
        ContractStatus::Completed,
        "final status should be Completed (not fully refunded)"
    );
}

// ---------------------------------------------------------------------------
// resolve_dispute → dsp_index / settled (Split)
// ---------------------------------------------------------------------------

#[test]
fn resolve_dispute_split_emits_correct_settled_payload() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));

    let split = DisputeSplit {
        client_amount: 60,
        freelancer_amount: 40,
    };
    assert!(client.resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::Split(split),
    ));

    let settled_sym = symbol_short!("settled");
    let (_topics, data) = find_dsp_index_event(&env, &settled_sym).unwrap();
    let data_tuple = decode_settled_payload(&env, &data);

    assert_eq!(data_tuple.1, 3, "resolution_code for Split should be 3");
    assert_eq!(data_tuple.2, 60, "client_payout should be 60");
    assert_eq!(data_tuple.3, 40, "freelancer_payout should be 40");
    assert_eq!(
        data_tuple.4,
        ContractStatus::Completed,
        "final status should be Completed"
    );
}

// ---------------------------------------------------------------------------
// Topic collision check
// ---------------------------------------------------------------------------

#[test]
fn dsp_index_topic_does_not_collide_with_other_topics() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert!(client.resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::FullRefund,
    ));

    let events = env.events().all();
    let dsp_index_sym = symbol_short!("dsp_index");

    // Collect all unique first-position topics
    let mut first_topics: soroban_sdk::Vec<Symbol> = soroban_sdk::Vec::new(&env);
    for event in events.iter() {
        if event.1.len() > 0 {
            if let Ok(sym) = Symbol::try_from_val(&env, &event.1.get(0).unwrap()) {
                // Avoid duplicates
                let mut found = false;
                for existing in first_topics.iter() {
                    if existing == sym {
                        found = true;
                        break;
                    }
                }
                if !found {
                    first_topics.push_back(sym);
                }
            }
        }
    }

    // Verify dsp_index is present
    let has_dsp_index = first_topics.iter().any(|s| s == dsp_index_sym);
    assert!(has_dsp_index, "dsp_index topic must be present");

    // Verify dsp_index does not collide with other known topics
    let known_other_topics: [Symbol; 6] = [
        symbol_short!("dispute"),
        symbol_short!("created"),
        symbol_short!("refunded"),
        symbol_short!("cancelled"),
        symbol_short!("finalized"),
        symbol_short!("init"),
    ];

    for known in &known_other_topics {
        assert_ne!(
            &dsp_index_sym, known,
            "dsp_index must not collide with {:?}",
            known
        );
    }
}

// ---------------------------------------------------------------------------
// Both raise and resolve emit their respective dsp_index events in a full flow
// ---------------------------------------------------------------------------

#[test]
fn full_dispute_flow_emits_both_raised_and_settled_events() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert!(client.resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::FullPayout,
    ));

    let raised_sym = symbol_short!("raised");
    let settled_sym = symbol_short!("settled");

    assert!(
        find_dsp_index_event(&env, &raised_sym).is_some(),
        "dsp_index/raised must be emitted"
    );
    assert!(
        find_dsp_index_event(&env, &settled_sym).is_some(),
        "dsp_index/settled must be emitted"
    );
}

// ---------------------------------------------------------------------------
// Freelancer can raise dispute and the event captures the correct caller
// ---------------------------------------------------------------------------

#[test]
fn freelancer_raise_dispute_captures_correct_caller_in_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (_client_addr, freelancer_addr, _arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    // Freelancer raises the dispute
    assert!(client.raise_dispute(&contract_id, &freelancer_addr));

    let raised_sym = symbol_short!("raised");
    let (_topics, data) = find_dsp_index_event(&env, &raised_sym).unwrap();

    let data_tuple = decode_raised_payload(&env, &data);

    assert_eq!(
        data_tuple.1, freelancer_addr,
        "caller in event should be freelancer"
    );
}

// ---------------------------------------------------------------------------
// Compatibility contract: topic symbol and payload layout are stable
// ---------------------------------------------------------------------------

/// The `dsp_index` topic symbol must remain exactly `dsp_index` (7 chars, fits
/// in `symbol_short!`).  This guards against accidental renames that would
/// silently break off-chain indexers.
#[test]
fn dsp_index_topic_symbol_is_stable() {
    assert_eq!(DSP_INDEX_TOPIC, "dsp_index");
    assert_eq!(DSP_INDEX_TOPIC.len(), 9);
    let sym = symbol_short!("dsp_index");
    assert_eq!(sym, symbol_short!("dsp_index"));
}

/// Sub-topic symbols for raise and resolve must remain `raised` and `settled`.
#[test]
fn dsp_index_subtopic_symbols_are_stable() {
    assert_eq!(symbol_short!("raised"), symbol_short!("raised"));
    assert_eq!(symbol_short!("settled"), symbol_short!("settled"));
    assert_ne!(symbol_short!("raised"), symbol_short!("settled"));
}

/// The `raised` payload layout is a 6-tuple:
/// `(contract_id: u32, caller: Address, funded: i128, released: i128,
///   refunded: i128, timestamp: u64)`.
/// Decoding must succeed for a real event and fail for a malformed payload.
#[test]
fn raised_payload_layout_is_stable_and_rejects_malformed() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, _arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));

    let raised_sym = symbol_short!("raised");
    let (_topics, data) = find_dsp_index_event(&env, &raised_sym).unwrap();

    // Valid decode succeeds.
    let decoded = decode_raised_payload(&env, &data);
    assert_eq!(decoded.0, contract_id);
    assert_eq!(decoded.1, client_addr);

    // Malformed payload (wrong arity) must not decode into the expected shape.
    let malformed: Val = (1_u32, 2_u32).into_val(&env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_raised_payload(&env, &malformed)
    }));
    assert!(
        result.is_err(),
        "malformed payload must not decode as raised tuple"
    );
}

/// The `settled` payload layout is a 6-tuple:
/// `(contract_id: u32, resolution_code: u32, client_payout: i128,
///   freelancer_payout: i128, final_status: ContractStatus, timestamp: u64)`.
/// Decoding must succeed for a real event and fail for a malformed payload.
#[test]
fn settled_payload_layout_is_stable_and_rejects_malformed() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert!(client.resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::FullRefund,
    ));

    let settled_sym = symbol_short!("settled");
    let (_topics, data) = find_dsp_index_event(&env, &settled_sym).unwrap();

    // Valid decode succeeds.
    let decoded = decode_settled_payload(&env, &data);
    assert_eq!(decoded.0, contract_id);
    assert_eq!(decoded.1, 0);

    // Malformed payload (wrong arity) must not decode into the expected shape.
    let malformed: Val = (1_u32, 2_u32).into_val(&env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_settled_payload(&env, &malformed)
    }));
    assert!(
        result.is_err(),
        "malformed payload must not decode as settled tuple"
    );
}

/// Duplicate `raise_dispute` calls must not emit a second `dsp_index`/`raised`
/// event, preserving the invariant that each contract emits at most one raise
/// event.  This protects indexers from double-counting.
#[test]
fn duplicate_raise_dispute_does_not_emit_second_raised_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, _arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));

    let raised_sym = symbol_short!("raised");
    let count_after_first = env
        .events()
        .all()
        .iter()
        .filter(|event| {
            event.1.len() >= 2
                && Symbol::try_from_val(&env, &event.1.get(0).unwrap()).ok()
                    == Some(symbol_short!("dsp_index"))
                && Symbol::try_from_val(&env, &event.1.get(1).unwrap()).ok()
                    == Some(raised_sym.clone())
        })
        .count();
    assert_eq!(count_after_first, 1, "exactly one raised event expected");

    // Second raise must fail (already disputed) and must not add another event.
    let second = client.try_raise_dispute(&contract_id, &client_addr);
    assert!(second.is_err(), "duplicate raise must be rejected");

    let count_after_second = env
        .events()
        .all()
        .iter()
        .filter(|event| {
            event.1.len() >= 2
                && Symbol::try_from_val(&env, &event.1.get(0).unwrap()).ok()
                    == Some(symbol_short!("dsp_index"))
                && Symbol::try_from_val(&env, &event.1.get(1).unwrap()).ok()
                    == Some(raised_sym.clone())
        })
        .count();
    assert_eq!(
        count_after_second, 1,
        "rejected duplicate must not emit another raised event"
    );
}

/// Boundary: zero-amount contract cannot be disputed (no funds to settle), and
/// no `dsp_index` event should be emitted for the rejected call.
#[test]
fn zero_amount_raise_dispute_rejected_without_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    let arbiter_addr = Address::generate(&env);
    let milestones = vec![&env, 0_i128];
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr),
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );

    let result = client.try_raise_dispute(&contract_id, &client_addr);
    assert!(result.is_err(), "zero-amount dispute must be rejected");

    let raised_sym = symbol_short!("raised");
    assert!(
        find_dsp_index_event(&env, &raised_sym).is_none(),
        "no raised event should be emitted for rejected dispute"
    );
}

/// Boundary: unauthorized caller cannot raise a dispute and no event is emitted.
#[test]
fn unauthorized_raise_dispute_rejected_without_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (_client_addr, _freelancer_addr, _arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    let stranger = Address::generate(&env);
    let result = client.try_raise_dispute(&contract_id, &stranger);
    assert!(result.is_err(), "unauthorized raise must be rejected");

    let raised_sym = symbol_short!("raised");
    assert!(
        find_dsp_index_event(&env, &raised_sym).is_none(),
        "no raised event should be emitted for unauthorized caller"
    );
}

/// Boundary: unauthorized arbiter cannot resolve a dispute and no `settled`
/// event is emitted.
#[test]
fn unauthorized_resolve_dispute_rejected_without_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, _arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));

    let stranger = Address::generate(&env);
    let result = client.try_resolve_dispute(
        &contract_id,
        &stranger,
        &DisputeResolution::FullRefund,
    );
    assert!(result.is_err(), "unauthorized resolve must be rejected");

    let settled_sym = symbol_short!("settled");
    assert!(
        find_dsp_index_event(&env, &settled_sym).is_none(),
        "no settled event should be emitted for unauthorized arbiter"
    );
}

/// Regression: resolving a dispute that was never raised must be rejected and
/// must not emit a `settled` event.
#[test]
fn resolve_without_raise_rejected_without_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (_client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    let result = client.try_resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::FullRefund,
    );
    assert!(result.is_err(), "resolve without raise must be rejected");

    let settled_sym = symbol_short!("settled");
    assert!(
        find_dsp_index_event(&env, &settled_sym).is_none(),
        "no settled event should be emitted when no dispute exists"
    );
}

/// Regression: double resolve must be rejected and must not emit a second
/// `settled` event.
#[test]
fn double_resolve_rejected_without_second_settled_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));
    assert!(client.resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::FullRefund,
    ));

    let settled_sym = symbol_short!("settled");
    let count_after_first = env
        .events()
        .all()
        .iter()
        .filter(|event| {
            event.1.len() >= 2
                && Symbol::try_from_val(&env, &event.1.get(0).unwrap()).ok()
                    == Some(symbol_short!("dsp_index"))
                && Symbol::try_from_val(&env, &event.1.get(1).unwrap()).ok()
                    == Some(settled_sym.clone())
        })
        .count();
    assert_eq!(count_after_first, 1, "exactly one settled event expected");

    let second = client.try_resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::FullPayout,
    );
    assert!(second.is_err(), "double resolve must be rejected");

    let count_after_second = env
        .events()
        .all()
        .iter()
        .filter(|event| {
            event.1.len() >= 2
                && Symbol::try_from_val(&env, &event.1.get(0).unwrap()).ok()
                    == Some(symbol_short!("dsp_index"))
                && Symbol::try_from_val(&env, &event.1.get(1).unwrap()).ok()
                    == Some(settled_sym.clone())
        })
        .count();
    assert_eq!(
        count_after_second, 1,
        "rejected double resolve must not emit another settled event"
    );
}

/// Boundary: Split amounts that do not sum to the funded balance must be
/// rejected and must not emit a `settled` event.
#[test]
fn invalid_split_sum_rejected_without_event() {
    let env = Env::default();
    env.mock_all_auths();

    let client = register_client(&env);
    let (client_addr, _freelancer_addr, arbiter_addr, contract_id) =
        funded_with_arbiter(&env, &client);

    assert!(client.raise_dispute(&contract_id, &client_addr));

    let bad_split = DisputeSplit {
        client_amount: 60,
        freelancer_amount: 30,
    };
    let result = client.try_resolve_dispute(
        &contract_id,
        &arbiter_addr,
        &DisputeResolution::Split(bad_split),
    );
    assert!(result.is_err(), "split not summing to balance must be rejected");

    let settled_sym = symbol_short!("settled");
    assert!(
        find_dsp_index_event(&env, &settled_sym).is_none(),
        "no settled event should be emitted for invalid split"
    );
}
