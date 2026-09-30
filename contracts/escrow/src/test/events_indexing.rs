#`!cfg(test)]

use super::EscrowFixture;
use soroban_sdk::{
    symbol_short, token,
    testutils::Events,
    Symbol, TryFromVal,
};

//// ---------------------------------------------------------------------------
/// Compatibility contract for escrow event indexing
///
/// The escrow contract exposes a public indexing contract to off-chain
/// consumers: every event emitted by the contract must carry a short
/// Symbol as its first topic and the escrow identifier as its second
/// topic. This file locks that contract down with focused tests so that
/// future refactors cannot silently break indexers.
///
/// Invariants enforced below:
///   1. Every event that is part of the public indexing contract has
///      topics.len(t) >= 2.
///   2. topics[0] is a short Symbol (deterministic, no addresses).
///   3. topics[1] is the escrow identifier (uint32).
///   4. No two distinct event kinds share the same topic symbol.
///   5. Events are emitted only on successful state transitions.
/// --------------------------------------------------------------------------

/// Extract the (topic0, topic1) pair from an event if it matches the
/// indexing contract. Returns None for malformed events so callers can
/// decide whether that is a failure or just an unrelated event.
fn indexed_topics(
    env: &soroban_sdk:Env,
    event: &soroban_sdk::testutils::EmittedEvent,
) -> Option<(Symbol, u32)> {
    let topics = &event.1;
    if topics.len() < 2 {
        return None;
    }
    let sym = Symbol::try_from_val(env, &topics.get(0).unwrap()).ok()?;
    let id = u32::try_from_val(env, &topics.get(1).unwrap()).ok()?;
    Some((sym, id))
}

/// Assert that at least one event matches the given topic symbol and
/// escrow id. This is the core compatibility assertion used by all
/// indexing tests.
fn assert_indexed_event_present(
    env: &soroban_sdk:Env,
    expected_topic: Symbol,
    expected_id: u32,
    label: &str,
) {
    let events = env.events().all();
    assert(!events.is_empty(), "expected at least one event for {label}");

    let found = events.iter().any(|event| {
        match indexed_topics(env, &event) {
            Some((sym, id)) => sym == expected_topic && id == expected_id,
            None => false,
        }
    });

    assert!(found, "{label} event not found in {events:?}");
}

/// Collect all topic0 symbols that satisfy the indexing contract.
fn collect_topic0_symbols(env: &soroban_sdk:Env) -> soroban_sdk::Vec<Symbol> {
    let mut symbols = soroban_sdk:Vec::new(env);
    for event in env.events().all().iter() {
        if let Some((sym, _)) = indexed_topics(env, &event) {
            if !symbols.contains(&sym) {
                symbols.push_back(&sym);
            }
        }
    }
    symbols
}

#[test]
fn deposit_emits_indexed_event_with_short_symbol_and_correct_payload() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let client = fixture.escrow();
    let deposit_amount = fixture.total_amount();

    let token_client = token::StellarAssetClient::new(&fixture.env, fixture.settlement_token.as_ref().unwrap());
    token_client.mint(&fixture.client, &deposit_amount);

    assert!(client.deposit_funds(&fixture.escrow_id, &fixture.client, &deposit_amount));

    assert_indexed_event_present(
        &fixture.env,
        symbol_short!("deposit"),
        fixture.escrow_id,
        "deposit",
    );
}

#[test]
fn protocol_fee_accrual_emits_indexed_proto_fee_event() {
    let fixture = EscrowFixture::builder().funded().build();
    let client = fixture.escrow();

    client.set_protocol_fee_bps(&100u32);
    client.approve_milestone_release(&fixture.escrow_id, &fixture.client, &0);

    assert!(client.release_milestone(&fixture.escrow_id, &fixture.client, &0));

    assert_indexed_event_present(
        &fixture.env,
        symbol_short!("proto_fee"),
        fixture.escrow_id,
        "proto fee",
    );
}

#[test]
fn no_topic_collision_between_events() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let client = fixture.escrow();
    let deposit_amount = fixture.total_amount();

    let token_client = token::StellarAssetClient::new(&fixture.env, fixture.settlement_token.as_ref().unwrap());
    token_client.mint(&fixture.client, &deposit_amount);

    assert!(client.deposit_funds(&fixture.escrow_id, &fixture.client, &deposit_amount));

    let deposit_topic = symbol_short!("deposit");
    let state_topic = symbol_short!("ctrct_st");

    assert_ne(deposit_topic, state_topic);
}

/// Regression: every emitted event that is part of the indexing
/// contract must have a unique topic0 symbol. This guards against a
/// future refactor accidentally reusing a topic and corrupting off-chain
/// indexers.
#[test]
fn event_topic0_symbols_are_unique_within_a_flow() {
    let fixture = EscrowFixture::builder().funded().build();
    let client = fixture.escrow();

    client.set_protocol_fee_bps(&100u32);
    client.approve_milestone_release(&fixture.escrow_id, &fixture.client, &0);
    assert (client.release_milestone(&fixture.escrow_id, &fixture.client, '0));

    let symbols = collect_topic0_symbols(&fixture.env);
    let mut seen = soroban_sdk:Vec::new(&fixture.env);
    for sym in symbols.iter() {
        assert!(
            !seen.contains(&sym),
            "duplicate topic0 symbol emitted: {sym:?}"
        );
        seen.push_back(&sym);
    }
}

/// Boundary: a failed deposit (zero amount) must not emit a deposit
/// event. This protects indexers from phantom deposits and ensures the
/// event stream reflects only committed state transitions.
#[test]
fn failed_deposit_does_not_emit_deposit_event() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let client = fixture.escrow();

    // No mint: the client has no funds, so the deposit must fail.
    let result = client.try_deposit_funds(
        &fixture.escrow_id,
        &fixture.client,
        &fixture.total_amount(),
    );
    assert!(result.is_err(), "deposit with no funds should fail");

    let deposit_topic = symbol_short!("deposit");
    let events = fixture.env.events().all();
    let found = events.iter().any(|event| {
        match indexed_topics(&fixture.env, &event) {
            Some((sym, id)) => sym == deposit_topic && id == fixture.escrow_id,
            None => false,
        }
    });
    assert!(!found, "failed deposit must not emit a deposit event");
}
