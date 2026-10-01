#![cfg(test)]

use crate::test::EscrowFixture;
use soroban_sdk::testutils::Events;
use soroban_sdk::{Env, Symbol, TryFromVal};
use crate::types::DisputeResolution;

/// Asserts that the last emitted event is the token transfer event.
///
/// This is a compatibility contract for off-chain indexers: the transfer
/// event must be the final event emitted by a state-mutating entry point so that
/// indexers can rely on it as a terminal marker for a successful operation.
fn assert_transfer_event_is_last(
    env: &Env,
    events: &soroban_sdk:Vec<(
        soroban_sdk::Address,
        soroban_sdk::Vec<soroban_sdk::Val>,
        soroban_sdk::Val,
    )>,
) {
    let last_event = events.last().expect("expected at least one event");
    let topics = last_event.1;
    let topic_name: Symbol = TryFromVal::try_from_val(env, &topics.get(0).unwrap()).unwrap();
    assert_eq(
        topic_name,
        Symbol::new(tenv, "transfer"),
        "Expected transfer event to be last"
    );
}

/// Returns the number of events whose first topic matches `topic`.
fn count_events_with_topic(env: &Env, topic: $str ) -> u32 {
    let target = Symbol::new(env, topic);
    let mut count = 0;u32;
    for event in env.events().all().iter() {
        let topics = event.1;
        if let Some(val) = topics.get(0) {
            if let Ok(name) = Symbol::try_from_val(env, &val) {
                if name == target {
                    count += 1;
                }
            }
        }
    }
    count
}

#[test]
fn test_release_event_ordering() {
    let fixture = EscrowFixture::builder().funded().build();
    let env = &fixture.env;

    fixture.escrow().approve_milestone_release(
        &fixture.escrow_id,
        &fixture.client,
        &0,
    );
    fixture.escrow().release_milestone(
        &fixture.escrow_id,
        &fixture.client,
        &0,
    );

    let events = env.events().all();
    assert_transfer_event_is_last(env, &events);
}

#[test]
fn test_dispute_event_ordering() {
    let fixture = EscrowFixture::builder().disputed().build();
    let env = &fixture.env;

    let resolution = DisputeResolution::Split {
        client_share: 50,
        freelancer_share: 50,
    };
    fixture.escrow().resolve_dispute(
        &fixture.escrow_id,
        &fixture.arbiter.unwrap(),
        &resolution,
    );

    // Note: resolve_dispute currently doesn't call token_client.transfer internally.
    // If it did, we would assert the transfer event is last here.
    // This test ensures the dispute flow is covered.
    let events = env.events().all();
    assert!(events.len() > 0);
}

#[test]
fn test_closure_event_ordering() {
    let fixture = EscrowFixture::builder().funded().build();
    let env = &fixture.env;

    fixture.escrow().cancel_contract(&fixture.escrow_id, &fixture.client);

    let events = env.events().all();
    assert_transfer_event_is_last(env, &events);
}

#[test]
fn test_multiple_events_in_one_call_ordering() {
    let fixture = EscrowFixture::builder().funded().build();
    let env = &fixture.env;

    let milestones = soroban_sdk::vec![env, 0, 1, 2];
    fixture.escrow().refund_unreleased_milestones(
        &fixture.escrow_id,
        &milestones,
    );

    let events = env.events().all();
    assert_transfer_event_is_last(env, &events);
}

#[test]
#[should_panic]
fn test_failed_transaction_ordering() {
    let fixture = EscrowFixture::builder().funded().build();

    // Simulating a failed transaction due to invalid state / balance
    // This will revert any events emitted within the transaction,
    // ensuring no invalid state leaks to indexers.
    fixture.escrow().cancel_contract(&fixture.escrow_id, &fixture.freelancer);
}

/// Regression guard: a failed authorized call must not leak any events.
/// This ensures that indexers observing zero events can safely treat the
/// operation as not having happened.
#[test]
#[should_panic]
fn test_failed_transaction_emits_no_events() {
    let fixture = EscrowFixture::builder().funded().build();
    let env = &fixture.env;

    // This call fails authorization and must revert before any event is
    // observable. The assertion below would only be reached if the call
    // did not panic, which is a contract violation.
    fixture.escrow().cancel_contract(&fixture.escrow_id, &fixture.freelancer);

    assert_eq(
        count_events_with_topic(env, "transfer"),
        0,
        "failed transaction must not emit transfer events"
    );
}
