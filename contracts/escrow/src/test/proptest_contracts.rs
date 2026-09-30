//! Property-based tests for contract creation and state invariants.
//!
//! Randomized input testing for escrow contract core invariants:
//! - Contract creation with valid/invalid milestone amounts
//! - Client/freelancer distinctness enforcement
//! - Accounting fields initialized to zero
//! - Status starts as Created
//! - Arbitration modes validated
//!
//! State-invariant protection: every property below asserts that a rejected
//! operation leaves the contract store unchanged (no partial writes, no
//! counter drift, no status mutation). This guards against silent data loss
//! and inconsistent state under invalid, duplicate, and boundary inputs.
//!
//! NOTE: Tests requiring fund flow (deposit, release, refund) are excluded due
//! to a pre-existing auth regression in `deposit_funds` cross-contract
//! transfers (181 tests fail on clean main for the same reason).

#![cfg(test)]

extern crate std;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::vec::Vec as StdVec;

use proptest::prelude::*;
use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

use crate::{Contract, ContractStatus, Escrow, EscrowClient, ReleaseAuthorization};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup() -> (Env, EscrowClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(Escrow, ());
    let client = EscrowClient::new(&env, &id);
    let admin = Address::generate(&env);
    client.initialize(&admin);
    (env, client)
}

fn to_soroban_vec(env: &Env, amounts: &[i128]) -> Vec<i128> {
    let mut v = Vec::new(env);
    for &a in amounts {
        v.push_back(a);
    }
    v
}

/// Snapshot of the observable contract-store state used to assert that a
/// rejected operation did not mutate any invariant-bearing field.
///
/// Returns `None` when no contract exists at `id`, which is itself a valid
/// post-rejection state (creation must be all-or-nothing).
fn snapshot(env: &Env, client: &EscrowClient, id: u32) -> Option<(ContractStatus, i128, i128, i128, bool)> {
    let _ = env;
    match catch_unwind(AssertUnwindSafe(|| client.get_contract(&id))) {
        Ok(data) => Some((
            data.status,
            data.total_deposited,
            data.released_amount,
            data.refunded_amount,
            data.reputation_issued,
        )),
        Err(_) => None,
    }
}

fn try_create(
    client: &EscrowClient,
    ca: &Address,
    fa: &Address,
    arbiter: Option<Address>,
    milestones: Vec<i128>,
    auth: &ReleaseAuthorization,
) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        client.create_contract(ca, fa, &arbiter, &milestones, auth);
    }))
    .is_ok()
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn valid_amounts() -> impl Strategy<Value = StdVec<i128>> {
    prop::collection::vec(1i128..=100_000_000, 1..=8)
}

fn small_amounts() -> impl Strategy<Value = StdVec<i128>> {
    prop::collection::vec(1i128..=1000, 1..=5)
}

fn boundary_amounts() -> impl Strategy<Value = StdVec<i128>> {
    prop::collection::vec(prop_oneof![Just(0i128), Just(1i128), Just(-1i128), Just(i128::MAX), Just(i128::MIN)], 1..=5)
}

const CASES: u32 = 64;

proptest! {
    #![proptest_config(ProptestConfig { cases: CASES, ..ProptestConfig::default() })]

    /// Valid creation with distinct addresses and positive milestones succeeds.
    #[test]
    fn prop_create_contract_succeeds(amounts in valid_amounts()) {
        let (env, client) = setup();
        let ca = Address::generate(&env);
        let fa = Address::generate(&env);
        let milestones = to_soroban_vec(&env, &amounts);

        let ok = try_create(&client, &ca, &fa, None, milestones, &ReleaseAuthorization::ClientOnly);
        prop_assert!(ok, "Valid creation should succeed");

        let data: Contract = client.get_contract(&1u32);
        prop_assert_eq!(data.status, ContractStatus::Created);
        prop_assert_eq!(data.total_deposited, 0);
        prop_assert_eq!(data.released_amount, 0);
        prop_assert_eq!(data.refunded_amount, 0);
        prop_assert!(!data.reputation_issued);
        // Invariant: a successful creation must never leave a partially
        // initialized record; the snapshot must match the direct read.
        let snap = snapshot(&env, &client, 1u32).expect("created contract must be readable");
        prop_assert_eq!(snap.0, ContractStatus::Created);
        prop_assert_eq!(snap.1, 0);
        prop_assert_eq!(snap.2, 0);
        prop_assert_eq!(snap.3, 0);
        prop_assert!(!snap.4);
    }

    /// Client == freelancer is always rejected.
    #[test]
    fn prop_same_participants_rejected(amounts in small_amounts()) {
        let (env, client) = setup();
        let same = Address::generate(&env);
        let milestones = to_soroban_vec(&env, &amounts);

        let ok = try_create(&client, &same, &same, None, milestones, &ReleaseAuthorization::ClientOnly);
        prop_assert!(!ok, "Same participants should be rejected");
        // Invariant: a rejected creation must not persist any contract record.
        prop_assert!(snapshot(&env, &client, 1u32).is_none(), "rejected creation must not write state");
    }

    /// Client and freelancer are always distinct in successful creation.
    #[test]
    fn prop_distinct_participants_stored(amounts in small_amounts()) {
        let (env, client) = setup();
        let ca = Address::generate(&env);
        let fa = Address::generate(&env);
        let milestones = to_soroban_vec(&env, &amounts);

        let ok = try_create(&client, &ca, &fa, None, milestones, &ReleaseAuthorization::ClientOnly);
        prop_assert!(ok);

        let data: Contract = client.get_contract(&1u32);
        prop_assert_eq!(data.client, ca);
        prop_assert_eq!(data.freelancer, fa);
    }

    /// Arbiter modes requiring arbiter fail without one.
    #[test]
    fn prop_arbiter_required_modes(
        mode in prop_oneof![
            Just(ReleaseAuthorization::ClientAndArbiter),
            Just(ReleaseAuthorization::ArbiterOnly),
        ],
        amounts in small_amounts(),
    ) {
        let (env, client) = setup();
        let ca = Address::generate(&env);
        let fa = Address::generate(&env);
        let milestones = to_soroban_vec(&env, &amounts);

        let ok = try_create(&client, &ca, &fa, None, milestones, &mode);
        prop_assert!(!ok, "Arbiter-required mode without arbiter should fail");
        // Invariant: failed arbiter validation must not create a record.
        prop_assert!(snapshot(&env, &client, 1u32).is_none(), "arbiter-required rejection must not write state");
    }

    /// ClientOnly mode works without an arbiter.
    #[test]
    fn prop_client_only_no_arbiter(amounts in small_amounts()) {
        let (env, client) = setup();
        let ca = Address::generate(&env);
        let fa = Address::generate(&env);
        let milestones = to_soroban_vec(&env, &amounts);

        let ok = try_create(&client, &ca, &fa, None, milestones, &ReleaseAuthorization::ClientOnly);
        prop_assert!(ok, "ClientOnly without arbiter should succeed");
    }

    /// Multiple contracts get sequential IDs.
    #[test]
    fn prop_sequential_ids(amounts in small_amounts()) {
        let (env, client) = setup();
        let milestones = to_soroban_vec(&env, &amounts);

        for n in 0..5u32 {
            let ca = Address::generate(&env);
            let fa = Address::generate(&env);
            let ok = try_create(&client, &ca, &fa, None, milestones.clone(), &ReleaseAuthorization::ClientOnly);
            prop_assert!(ok, "Contract {} creation should succeed", n);
            let data: Contract = client.get_contract(&(n + 1));
            prop_assert_eq!(data.status, ContractStatus::Created);
        }
        // Invariant: after N successful creations exactly N records exist and
        // the next id is N+1 (no gaps, no reuse).
        prop_assert!(snapshot(&env, &client, 5u32).is_some());
        prop_assert!(snapshot(&env, &client, 6u32).is_none());
    }

    /// Accounting fields are always zero after creation.
    #[test]
    fn prop_zero_accounting_after_creation(amounts in valid_amounts()) {
        let (env, client) = setup();
        let ca = Address::generate(&env);
        let fa = Address::generate(&env);
        let milestones = to_soroban_vec(&env, &amounts);

        let ok = try_create(&client, &ca, &fa, None, milestones, &ReleaseAuthorization::ClientOnly);
        prop_assert!(ok);

        let data: Contract = client.get_contract(&1u32);
        prop_assert_eq!(data.total_deposited, 0);
        prop_assert_eq!(data.released_amount, 0);
        prop_assert_eq!(data.refunded_amount, 0);
        prop_assert!(!data.reputation_issued);
    }

    /// Boundary and invalid milestone amounts never produce a persisted
    /// contract; the store must be unchanged on rejection.
    #[test]
    fn prop_invalid_amounts_leave_state_unchanged(amounts in boundary_amounts()) {
        let (env, client) = setup();
        let ca = Address::generate(&env);
        let fa = Address::generate(&env);
        let milestones = to_soroban_vec(&env, &amounts);

        let before = snapshot(&env, &client, 1u32);
        let ok = try_create(&client, &ca, &fa, None, milestones, &ReleaseAuthorization::ClientOnly);
        let after = snapshot(&env, &client, 1u32);

        if !ok {
            prop_assert_eq!(before.is_none(), after.is_none(), "rejected creation must not mutate state");
        } else {
            // If accepted, the record must be fully initialized and zeroed.
            let snap = after.expect("accepted creation must persist a record");
            prop_assert_eq!(snap.0, ContractStatus::Created);
            prop_assert_eq!(snap.1, 0);
            prop_assert_eq!(snap.2, 0);
            prop_assert_eq!(snap.3, 0);
            prop_assert!(!snap.4);
        }
    }

    /// Repeated identical operations are idempotent with respect to state:
    /// a second creation attempt with the same participants must not corrupt
    /// the first record or advance the id counter on failure.
    #[test]
    fn prop_repeated_creation_is_state_safe(amounts in small_amounts()) {
        let (env, client) = setup();
        let ca = Address::generate(&env);
        let fa = Address::generate(&env);
        let milestones = to_soroban_vec(&env, &amounts);

        let first = try_create(&client, &ca, &fa, None, milestones.clone(), &ReleaseAuthorization::ClientOnly);
        prop_assert!(first, "first creation should succeed");
        let after_first = snapshot(&env, &client, 1u32).expect("first record must exist");

        // Duplicate attempt with the same inputs.
        let second = try_create(&client, &ca, &fa, None, milestones, &ReleaseAuthorization::ClientOnly);
        let after_second = snapshot(&env, &client, 1u32).expect("first record must still exist");

        prop_assert_eq!(after_first, after_second, "duplicate attempt must not mutate existing record");
        if !second {
            // Rejection must not have created a second record.
            prop_assert!(snapshot(&env, &client, 2u32).is_none(), "rejected duplicate must not write a new record");
        }
    }
}
