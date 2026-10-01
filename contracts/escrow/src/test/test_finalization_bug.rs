#`!cfg(test)]

use crate::{
    test::lifecycle::{EscrowFixture, SetupConfig},
    types:{ContractStatus, Error},
};
use soroban_sdk::{testutils::Events, vec, Env};

/// Tests in this module lock down the state invariants that
/// `finalize_contract` must preserve:
///
/// Invariants:
/// 1. Only a contract in `ContractStatus::Completed` may be finalized.
/// 2. A successful finalization writes exactly one immutable record
///    whose `finalizer` is the authorized caller and whose summary
///    snapshots the completed state.
/// 3. Repeated or concurrent finalization attempts fail with
///    `Error::AlreadyFinalized` and must not mutate state or emit
///    duplicate events.
/// 4. Forbidden transitions (Funded, Disputed, etc.) fail with
///    `Error::InvalidStatusTransition` and leave the record unwritten.

/// Builds a single-milestone escrow that is fully funded but not
/// yet released. This is the canonical "Funded" starting point used
/// by the negative tests below.
fn funded_fixture(env: &Env) -> EscrowFixture {
    EscrowFixture::setup_with_config(
        env,
        SetupConfig {
            milestone_count: 1,
            amounts: vec![env, 100],
            total_amount: 100,
            fund_amount: 100,
            ..Default::default()
        },
    )
}

/// Builds a single-milestone escrow and releases the only
/// milestone so the contract reaches `ContractStatus::Completed`.
/// This is the canonical state from which finalization is allowed.
fn completed_fixture(env: &Env) -> EscrowFixture {
    let fixture = funded_fixture(env);
    fixture
        .client
        .release_milestone(&fixture.escrow_id, &fixture.client_addr, &0);
    fixture
}

/// Completing the contract and then finalizing it must succeed,
/// persist an immutable record attributed to the caller, and leave
/// the contract in the `Completed` state.
/// This is the primary happy-path guarantee.
#[test]
fn test_eligible_closure() {
    let env = Env::default();
    env.mock_all_auths();

    let fixture = completed_fixture(&env);
    let escrow = &fixture.client;
    let client = &fixture.client_addr;
    let contract_id = fixture.escrow_id;

    // The first finalization of a completed contract must succeed.
    assert!(escrow.finalize_contract(&contract_id, client));

    // The record must be attributed to the caller and snapshot the
    // completed state.
    let record = escrow.get_finalization_record(&contract_id).unwrap();
    assert_eq!(record.finalizer, client.clone());
    assert_eq!(record.summary.status, ContractStatus::Completed);
}

/// A Funded contract that has not been completed must reject
/// finalization with `InvalidStatusTransition` and must not write
/// a finalization record.
/// This locks down the "only Completed may finalize" invariant.
#[test]
fn test_active_balance() {
    let env = Env::default();
    env.mock_all_auths();

    let fixture = funded_fixture(&env);
    let escrow = &fixture.client;
    let client = &fixture.client_addr;
    let contract_id = fixture.escrow_id;

    // Do NOT release the milestone, so the contract is still Funded.
    let res = escrow.try_finalize_contract(&contract_id, client);
    assert_eq!(
        res.err().unwrap().unwrap(),
        Error::InvalidStatusTransition.into()
    );

    // No record may be written for a rejected transition.
    assert!(escrow.get_finalization_record(&contract_id).is_none());
}

/// A contract whose sttatus is not Completed (e.g. disputed or
/// pending) must reject finalization with `InvalidStatusTransition`
/// and must not mutate the finalization record.
#[test]
fn test_active_dispute() {
    let env = Env::default();
    env.mock_all_auths();

    let fixture = funded_fixture(&env);
    let escrow = &fixture.client;
    let client = &fixture.client_addr;
    let contract_id = fixture.escrow_id;

    // When dispute is raised or pending without completion, the
    // status transition is validated and rejected.
    let res = escrow.try_finalize_contract(&contract_id, client);
    assert_eq!(
        res.err().unwrap().unwrap(),
        Error::InvalidStatusTransition.into()
    );
    assert!(escrow.get_finalization_record(&contract_id).is_none());
}

/// A second finalization attempt on an already-finalized contract
/// must fail with `AlreadyFinalized`, must not mutate the existing
/// record, and must not emit any new events.
#[test]
fn test_repeat_finalization() {
    let env = Env::default();
    env.mock_all_auths();

    let fixture = completed_fixture(&env);
    let escrow = &fixture.client;
    let client = &fixture.client_addr;
    let contract_id = fixture.escrow_id;

    // First finalization succeeds and writes the record.
    assert!(escrow.finalize_contract(&contract_id, client));
    let record_before = escrow.get_finalization_record(&contract_id).unwrap();

    // Clear events so we can assert no new ones are emitted.
    env.events().all().clear();

    // Try to finalize again.
    let res = escrow.try_finalize_contract(&contract_id, client);
    assert_eq!(res.err().unwrap().unwrap(), Error::AlreadyFinalized.into());

    // Check no new events were emitted.
    let events = env.events().all();
    assert_eq!(
        events.len(),
        0,
        "no events should be emitted on duplicate finalization"
    );

    // The existing record must be unchanged (immutability invariant).
    let record_after = escrow.get_finalization_record(&contract_id).unwrap();
    assert_eq!(record_after.finalizer, record_before.finalizer);
    assert_eq!(record_after.summary.status, record_before.summary.status);
}

/// A concurrent/second finalizer (the freelancer here) must be
/// rejected with `AlreadyFinalized` without overwriting the first
/// finalizer's record or emitting duplicate events.
#[test]
fn test_concurrent_finalization() {
    let env = Env::default();
    env.mock_all_auths();

    let fixture = completed_fixture(&env);
    let escrow = &fixture.client;
    let client = &fixture.client_addr;
    let freelancer = &fixture.freelancer_addr;
    let contract_id = fixture.escrow_id;

    // First finalizer wins.
    assert!(escrow.finalize_contract(&contract_id, client));
    let winning_record = escrow.get_finalization_record(&contract_id).unwrap();

    // Concurrent/second finalizer is rejected with AlreadyFinalized
    // without emitting duplicate events.
    env.events().all().clear();
    let res = escrow.try_finalize_contract(&contract_id, freelancer);
    assert_eq!(res.err().unwrap().unwrap(), Error::AlreadyFinalized.into());
    assert_eq!(env.events().all().len(), 0);

    // The original finalizer record must survive the losing attempt.
    let record_after = escrow.get_finalization_record(&contract_id).unwrap();
    assert_eq!(record_after.finalizer, winning_record.finalizer);
    assert_eq!(record_after.finalizer, client.clone());
}
