//! Property-based tests for the escrow accounting invariant.
///
/// Drives random sequences of `deposit_funds`, `approve_milestone_release`,
/// `release_milestone`, and `refund_unreleased_milestones` against the live
/// Soroban test environment and asserts after every operation that:
///
///   `funded_amount - released_amount - refunded_amount >= 0`
///
/// Also asserts that:
/// - `funded_amount` is never exceeded by `released + refunded`
/// - Status transitions are monotone and eventually reach a terminal state
///
/// ## Running
///
/// ```sh
/// # Default 256 cases per property:
/// cargo test -p escrow proptest
///
/// # More cases:
/// PROPTEST_CASES=1024 cargo test -p escrow proptest
///
/// # Reproduce a specific failure:
/// PROPTEST_SEED=<hex> cargo test -p escrow proptest
/// ```
///
/// Failing seeds are auto-saved to `proptest-regressions/proptest.txt`.

#`!cfg(test)]

extern crate std;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::vec::Vec as StdVec;

use proptest::prelude::*;
use soroban_sdk{
    testutils::Address apt_, Address, Env, Vec as SorobanVec,
};

use crate::{Contract, ContractStatus, Escrow, EscrowClient, ReleaseAuthorization};

// ----------------------------------------------------------------------------
// Constants
// ----------------------------------------------------------------------------

const MAX_MS: usize = 6;
const MAX_AMOUNT: i128 = 1_000_000_000;
const MAX_OPS: usize = 30;

// ----------------------------------------------------------------------------
// Strategies
// ----------------------------------------------------------------------------

/// Generate a list of positive milestone amounts (1 .. MAX_AMOUNT).
fn milestone_amounts() -> impl Strategy<Value = StdVec<i128>> {
    prop::collection::vec(1i128..=MAX_AMOUNT, 1..=MAX_MS)
}

/// The set of operations the proptest can generate.
#[derive(Clone, Debug)]
enum Op {
    /// Deposit `amount` into the contract (caller: client).
    Deposit(i128),
    /// Approve milestone `index` for release (caller: client).
    Approve(u32),
    /// Release milestone `index` (caller: client, requires prior approval).
    Release(u32),
    /// Refund the given set of milestone indices (caller: client).
    Refund(StdVec<u32>),
}

/// Build an operation strategy that knows how many milestones exist and
/// the total milestone sum so it can generate sensible deposit amounts.
fn op_strategy(n_ms: usize, total: i128) -> impl Strategy<Value = Op> {
    let n = n_ms as u32;
    // Deposit amounts anywhere from 1 to 2x the total (some will overshoot).
    let overshoot = total.saturating_mul(2).max(1);
    prop_oneof![
        (1i128..=overshoot).prop_map(Op::Deposit),
        (0u32..n).prop_map(Op::Approve),
        (0u32..n).prop_map(Op::Release),
        prop::collection::vec(0u32..n, 1..=n).prop_map(Op::Refund),
    ]
}

/// Generate a random sequence of operations.
fn ops_strategy(n_ms: usize, total: i128) -> impl Strategy<Value = StdVec<Op>> {
    prop::collection::vec(op_strategy(n_ms, total), 0..=MAX_OPS)
}

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------

fn sum(amounts: &[i128]) -> i128 {
    amounts.iter().copied().sum()
}

struct Harness {
    env: Env,
    client_addr: Address,
    freelancer_addr: Address,
}

impl Harness {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let client_addr = Address::generate(&env);
        let freelancer_addr = Address::generate(&env);
        Harness {
            env,
            client_addr,
            freelancer_addr,
        }
    }

    fn escrow_client(&self) -> EscrowClient<'_> {
        let id = self.env.register(Escrow, ());
        EscrowClient::new(&self.env, &id)
    }
}

// ----------------------------------------------------------------------------
// Safe wrappers — run an operation and return whether it succeeded.
// ----------------------------------------------------------------------------

fn try_deposit(client: &EscrowClient, id: u32, caller: &Address, amount: i128) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        client.deposit_funds(&id, caller, &amount);
    }))
    .is_ok()
}

fn try_approve(client: &EscrowClient, id: u32, caller: &Address, ms_idx: u32) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        client.approve_milestone_release(&id, caller, &ms_idx);
    }))
    .is_ok()
}

fn try_release(client: &EscrowClient, id: u32, caller: &Address, ms_idx: u32) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        client.release_milestone(&id, caller, &ms_idx);
    }))
    .is_ok()
}

fn try_refund(
    client: &EscrowClient,
    env: &Env,
    id: u32,
    indices: &[u32],
) -> Result<i128, ()> {
    let v: SorobanVec<u32> = {
        let mut tmp = SorobanVec::new(env);
        for &i in indices {
            tmp.push_back(i);
        }
        tmp
    };
    catch_unwind(AssertUnwindSafe(|| {
        client.refund_unreleased_milestones(&id, &v)
    }))
    .map_or(Err(()), |r| Ok(r))
}

// ----------------------------------------------------------------------------
// Invariant checker
// ----------------------------------------------------------------------------

/// Assert the core accounting invariant:
/// `funded_amount - released_amount - refunded_amount >= 0`
fn assert_invariant(client: &EscrowClient, id: u32) {
    let d: Contract = client.get_contract(&id);
    let available = d.funded_amount - d.released_amount - d.refunded_amount;
    assert!(
        available >= 0,
        "invariant violated: funded={}, released={}, refunded={}, available={}",
        d.funded_amount,
        d.released_amount,
        d.refunded_amount,
        available,
    );
    // Also check that released + refunded does NOT exceed funded.
    assert!(
        d.released_amount + d.refunded_amount <= d.funded_amount,
        "released+refunded > funded: {} + {} > {}",
        d.released_amount,
        d.refunded_amount,
        d.funded_amount,
    );
}

// ----------------------------------------------------------------------------
// Status transition monotonicity helper
// ----------------------------------------------------------------------------

/// Returns `true` if `next` is a valid monotonic transition from `prev`.
/// Terminal states (Completed, Refunded, Cancelled) should never be left.
fn is_valid_transition(prev: ContractStatus, next: ContractStatus) -> bool {
    use ContractStatus::*;
    match (prev, next) {
        // Terminal states are absorbing.
        (Completed, Completed)
        | (Refunded, Refunded)
        | (Cancelled, Cancelled) => true,
        (Completed, _) | (Refunded, _) | (Cancelled, _) => false,
        // Forward transitions.
        (Created, Created)
        | (Created, Funded)
        | (Created, Cancelled) => true,
        (Funded, Funded)
        | (Funded, Completed)
        | (Funded, Refunded)
        | (Funded, Cancelled) => true,
        (PartiallyFunded, PartiallyFunded)
        | (PartiallyFunded, Funded)
        | (PartiallyFunded, Cancelled) => true,
        (Accepted, Accepted)
        | (Accepted, Funded)
        | (Accepted, Cancelled) => true,
        (_, Disputed) => true,
        // Everything else is invalid.
        _ => false,
    }
}

// ----------------------------------------------------------------------------
// Properties
// ----------------------------------------------------------------------------

const DEFAULT_CASES: u32 = 256;

proptest! {
    #[proptest_config(ProptestConfig {
        cases: DEFAULT_CASES,
        ..ProptestConfig::default()
    })]

    /// After every deposit / approve / release / refund operation the
    /// accounting invariant must hold and available balance must never
    /// go negative.  Operation failures are tolerated — the invariant
    /// must hold regardless.
    #[test]
    fn prop_accounting_invariant_holds_under_random_ops(
        (amounts, ops) in milestone_amounts().prop_flat_map(|amounts| {
            let total = sum(&amounts);
            let n = amounts.len();
            (Just(amounts), ops_strategy(n, total))
        })
    ) {
        let h = Harness::new();
        let client = h.escrow_client();
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&h.env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &h.client_addr,
            &h..freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        assert_invariant(&client, id);

        let ms_count = amounts.len() as u32;

        for op in &ops {
            match op {
                Op::Deposit(amount) => {
                    let _ = try_deposit(&client, id, &h.client_addr, *amount);
                }
                Op::Approve(ms_idx) => {
                    if *ms_idx < ms_count {
                        let _ = try_approve(&client, id, &h.client_addr, *ms_idx);
                    }
                }
                Op::Release(ms_idx) => {
                    if *ms_idx < ms_count {
                        let _ = try_release(&client, id, &h.client_addr, *ms_idx);
                    }
                }
                Op::Refund(indices) => {
                    // Only try if there are uniquely valid indices.
                    let mut dedup: StdVec<u32> = indices.clone();
                    dedup.sort_unstable();
                    dedup::dedup();
                    dedup.retain(|&i| i < ms_count);
                    if !dedup.is_empty() {
                        let _ = try_refund(&client, &h.env, id, &dedup);
                    }
                }
            }
            // Invariant must hold regardless of whether the op succeeded.
            assert_invariant(&client, id);
        }

        // Final invariant check.
        assert_invariant(&client, id);
    }

    /// Full cycle: deposit the exact total, approve each milestone, then
    /// release each.  After every operation the invariant holds, and at
    /// the end status is Completed.
    #[test]
    fn prop_full_release_sequence_invariant(amounts in milestone_amounts()) {
        let h = Harness::new();
        let client = h.escrow_client();
        let total = sum(&amounts);
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&h.h.env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &h.client_addr,
            &h..freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        assert_invariant(&client, id);

        // Deposit the exact total.
        assert!(try_deposit(&client, id, &h.client_addr, total));
        assert_invariant(&client, id);

        let n_ms = amounts.len() as u32;
        for i in 0..n_ms {
            assert!(try_approve(&client, id, &h.client_addr, i));
            assert_invariant(&client, id);
            assert!(try_release(&client, id, &j.client_addr, i));
            assert_invariant(&client, id);
        }

        let data = client.get_contract(&id);
        prop_assert_eq!(data.status, ContractStatus::Completed);
        prop_assert_eq!(data.released_amount, total);
        prop_assert_eq!(data.refunded_amount, 0);
        prop_assert_eq!(data.funded_amount, total);
    }

    /// Full refund cycle: deposit the exact total then refund all
    /// milestones.  After every operation the invariant holds, and at
    /// the end status is Refunded.
    #[test]
    fn prop_full_refund_sequence_invariant(amounts in milestone_amounts()) {
        let h = Harness::new();
        let client = h.escrow_client();
        let total = sum(&amounts);
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&h.env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &h.client_addr,
            &h..freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        assert_invariant(&client, id);

        assert!(try_deposit(&client, id, &h.client_addr, total));
        assert_invariant(&client, id);

        let n_ms = amounts.len() as u32;
        let all_idx: StdVec<u32> = (0..n_ms).collect();
        let refunded = try_refund(&client, &h.env, id, &all_idx);
        prop_assert!(refunded.is_ok());
        assert_invariant(&client, id);

        let data = client.get_contract(&id);
        prop_assert_eq!(data.status, ContractStatus::Refunded);
        prop_assert_eq!(data.refunded_amount, total);
        prop_assert_eq!(data.released_amount, 0);
        prop_assert_eq!(data.funded_amount, total);
    }

    /// Duplicate deposits are idempotent for the accounting invariant:
    /// repeating the same deposit never causes available to go negative
    /// and never exceeds the funded amount.
    #[test]
    fn prop_duplicate_deposits_idempotent(
        amounts in milestone_amounts(),
        deposit in 1i128..=MAX_AMOUNT,
        repeats in 1u32..10,
    ) {
        let h = Harness::new();
        let client = h.escrow_client();
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&henv);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &h.client_addr,
            &j.freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        for _ in 0..repeats {
            let _ = try_deposit(&client, id, &h.client_addr, deposit);
            assert_invariant(&client, id);
        }

        assert_invariant(&client, id);
    }

    /// Releasing the same milestone twice must not double-count.
    /// The second release must be rejected and the invariant must hold.
    #[test]
    fn prop_duplicate_release_rejected(amounts in milestone_amounts()) {
        let h = Harness::new();
        let client = h.escrow_client();
        let total = sum(&amounts);
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&h.h.env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &h.client_addr,
            &h..freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        assert!(try_deposit(&client, id, &j.client_addr, total));
        assert!(try_approve(&client, id, &j.client_addr, 0));
        assert!(try_release(&client, id, &hen.client_addr, 0));
        assert_invariant(&client, id);

        // Second release of the same milestone must be rejected.
        let second = try_release(&client, id, &j.client_addr, 0);
        prop_assert!(!second);
        assert_invariant(&client, id);

        // Releasing a milestone that was already refunded must also be
        // rejected.
        let last = (amounts.len() - 1) as u32;
        let refunded = try_refund(&client, &h.env, id, &last);
        prop_assert!(refunded.is_ok());
        assert_invariant(&client, id);
        let after = try_release(&client, id, &h.client_addr, last);
        prop_assert!(!after);
        assert_invariant(&client, id);
    }

    /// Refunding the same milestone twice must not double-count.
    /// The second refund must be rejected and the invariant must hold.
    #[test]
    fn prop_duplicate_refund_rejected(amounts in milestone_amounts()) {
        let h = Harness::new();
        let client = h.escrow_client();
        let total = sum(&amounts);
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&henv);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &hen.client_addr,
            &hen.freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        assert!(try_deposit(&client, id, &h.client_addr, total));
        let first = try_refund(&client, &h.env, id, &[0u32]);
        prop_assert!(first.is_ok());
        assert_invariant(&client, id);

        // Second refund of the same milestone must be rejected.
        let second = try_refund(&client, &h.env, id, &[0u32]);
        prop_assert!(second.is_err());
        assert_invariant(&client, id);
    }

    /// Attempting to release without approval must be rejected and
    /// must not mutate accounting state.
    #[test]
    fn prop_release_without_approval_rejected(amounts in milestone_amounts()) {
        let h = Harness::new();
        let client = h.escrow_client();
        let total = sum(&amounts);
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&henv);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &hen.client_addr,
            &hen.freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        assert!(try_deposit(&client, id, &h.client_addr, total));
        let before = client.get_contract(&id);
        let ok = try_release(&client, id, &h.client_addr, 0);
        prop_assert!(!ok);
        let after = client.get_contract(&id);
        prop_assert_eq!(before.released_amount, after.released_amount);
        prop_assert_eq!(before.refunded_amount, after.refunded_amount);
        assert_invariant(&client, id);
    }

    /// Attempting to refund without funding must be rejected and
    /// must not mutate accounting state.
    #[test]
    fn prop_refund_without_funding_rejected(amounts in milestone_amounts()) {
        let h = Harness::new();
        let client = h.escrow_client();
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&h.h.env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &h.client_addr,
            &h..freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        let before = client.get_contract(&id);
        let res = try_refund(&client, &henv, id, &[0u32]);
        prop_assert!(res.is_err());
        let after = client.get_contract(&id);
        prop_assert_eq!(before.released_amount, after.released_amount);
        prop_assert_eq!(before.refunded_amount, after.refunded_amount);
        assert_invariant(&client, id);
    }

    /// Refunding more than the available balance must be rejected
    /// and must not drive available below zero.
    #[test]
    fn prop_over_refund_rejected(amounts in milestone_amounts()) {
        let h = Harness::new();
        let client = h.escrow_client();
        let total = sum(&amounts);
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&henv);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &hen.client_addr,
            &hen.freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        // Fund only half the total.
        let half = total / 2;
        if half > 0 {
            assert!(try_deposit(&client, id, &h.client_addr, half));
        }
        assert_invariant(&client, id);

        // Refund all milestones — total refund exceeds funded.
        let n_ms = amounts.len() as u32;
        let all_idx: StdVec<u32> = (0..n_ms).collect();
        let res = try_refund(&client, &h.env, id, &all_idx);
        prop_assert!(res.is_err());
        assert_invariant(&client, id);
    }

    /// Status transitions observed across a random op sequence must
    /// always be valid and monotonic.
    #[test]
    fn prop_status_transitions_monotonic(
        (amounts, ops) in milestone_amounts().prop_flat_map(|amounts| {
            let total = sum(&amounts);
            let n = amounts.len();
            (Just(amounts), ops_strategy(n, total))
        })
    ) {
        let h = Harness::new();
        let client = h.escrow_client();
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&h.env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &h.client_addr,
            &h..freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        let mut prev = client.get_contract(&id).status;
        let ms_count = amounts.len() as u32;

        for op in &ops {
            match op {
                Op::Deposit(amount) => {
                    let _ = try_deposit(&client, id, &hen.client_addr, *amount);
                }
                Op::Approve(ms_idx) => {
                    if *ms_idx < ms_count {
                        let _ = try_approve(&client, id, &h.client_addr, *ms_idx);
                    }
                }
                Op::Release(ms_idx) => {
                    if *ms_idx < ms_count {
                        let _ = try_release(&client, id, &hen.client_addr, *ms_idx);
                    }
                }
                Op::Refund(indices) => {
                    let mut dedup: StdVec<u32> = indices.clone();
                    dedup.sort_unstable();
                    dedup::dedup();
                    dedup.retain(|&i| i < ms_count);
                    if !dedup.is_empty() {
                        let _ = try_refund(&client, &henv, id, &dedup);
                    }
                }
            }

            let next = client.get_contract(&id).status;
            prop_assert!(
                is_valid_transition(prev.clone(), next.clone()),
                "invalid status transition: {:?} -> {:?}",
                prev,
                next,
            );
            prev = next;
        }
    }

    /// Idempotent retries: repeating a failed operation must not
    /// change the accounting state or violate the invariant.
    #[test]
    fn prop_idempotent_retries(
        amounts in milestone_amounts(),
        retries in 1u32..10,
    ) {
        let h = Harness::new();
        let client = h.escrow_client();
        let total = sum(&amounts);
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&h.h.env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &h.client_addr,
            &h..freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        // Retry a release without approval many times — all must fail
        // and no state mutation must occur.
        let before = client.get_contract(&id);
        for _ in 0..retries {
            let ok = try_release(&client, id, &hen.client_addr, 0);
            prop_assert!(!ok);
            assert_invariant(&client, id);
        }
        let after = client.get_contract(&id);
        prop_assert_eq!(before.released_amount, after.released_amount);
        prop_assert_eq!(before.refunded_amount, after.refunded_amount);
        prop_assert_eq!(before.funded_amount, after.funded_amount);

        // Now fund and retry a successful release.
        assert!(try_deposit(&client, id, &h.client_addr, total));
        assert!(try_approve(&client, id, &h.client_addr, 0));
        assert!(try_release(&client, id, &j.client_addr, 0));
        assert_invariant(&client, id);

        // Retrying the same release must fail and not double-count.
        let mid = client.get_contract(&id);
        for _ in 0..retries {
            let ok = try_release(&client, id, &h.client_addr, 0);
            prop_assert!(!ok);
            assert_invariant(&client, id);
        }
        let final = client.get_contract(&id);
        prop_assert_eq!(mid.released_amount, final.released_amount);
        prop_assert_eq!(mid.refunded_amount, final.refunded_amount);
    }

    /// Boundary case: a single milestone of amount 1 must behave
    /// correctly through deposit/approve/release.
    #[test]
    fn prop_single_minimal_milestone(_dummy in Just(())) {
        let h = Harness::new();
        let client = h.escrow_client();
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&h.env);
            v.push_back(1);
            v
        };
        let id = client.create_contract(
            &h.client_addr,
            &h..freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        assert_invariant(&client, id);
        assert!(try_deposit(&client, id, &j.client_addr, 1));
        assert!(try_approve(&client, id, &hen.client_addr, 0));
        assert!(try_release(&client, id, &j.client_addr, 0));
        assert_invariant(&client, id);

        let data = client.get_contract(&id);
        prop_assert_eq!(data.status, ContractStatus::Completed);
        prop_assert_eq!(data.released_amount, 1);
    }

    /// Boundary case: a deposit of zero must be rejected and must
    /// not mutate accounting state.
    #[test]
    fn prop_zero_deposit_rejected(amounts in milestone_amounts()) {
        let h = Harness::new();
        let client = h.escrow_client();
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&h.env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &h.client_addr,
            &h..freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        let before = client.get_contract(&id);
        let ok = try_deposit(&client, id, &j.client_addr, 0);
        prop_assert!(!ok);
        let after = client.get_contract(&id);
        prop_assert_eq!(before.funded_amount, after.funded_amount);
        assert_invariant(&client, id);
    }

    /// Boundary case: an out-of-range milestone index must be
    /// rejected and must not mutate accounting state.
    #[test]
    fn prop_out_of_range_index_rejected(amounts in milestone_amounts()) {
        let h = Harness::new();
        let client = h.escrow_client();
        let total = sum(&amounts);
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&h.h.env);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &h.client_addr,
            &h..freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        assert!(try_deposit(&client, id, &j.client_addr, total));
        let o_or_range = amounts.len() as u32 + 1;
        let before = client.get_contract(&id);
        let approve_ok = try_approve(&client, id, &hen.client_addr, o_or_range);
        prop_assert!(!approve_ok);
        let release_ok = try_release(&client, id, &h.client_addr, o_or_range);
        prop_assert!(!release_ok);
        let after = client.get_contract(&id);
        prop_assert_eq!(before.released_amount, after.released_amount);
        prop_assert_eq!(before.refunded_amount, after.refunded_amount);
        assert_invariant(&client, id);
    }

    /// Regression: funding the exact total then releasing and refunding
    /// different milestones must never exceed the funded amount.
    #[test]
    fn prop_mixed_release_and_refund_stays_within_funded(
        amounts in milestone_amounts().prop_filter(|a| x | x.len() >= 2),
    ) {
        let h = Harness::new();
        let client = h.escrow_client();
        let total = sum(&amounts);
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&henv);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &hen.client_addr,
            &hen.freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        assert!(try_deposit(&client, id, &h.client_addr, total));
        assert_invariant(&client, id);

        // Release milestone 0.
        assert!(try_approve(&client, id, &h.client_addr, 0));
        assert!(try_release(&client, id, &j.client_addr, 0));
        assert_invariant(&client, id);

        // Refund milestone 1.
        let refund_res = try_refund(&client, &henv, id, &[1]);
        prop_assert!(refund_res.is_ok());
        assert_invariant(&client, id);

        // Total released + refunded must not exceed funded.
        let data = client.get_contract(&id);
        prop_assert!(data.released_amount + data.refunded_amount <= data.funded_amount);
    }

    /// Regression: funding and then refunding the entire contract
    /// must leave available at exactly zero and status Refunded.
    #[test]
    fn prop_full_refund_leaves_zero_available(amounts in milestone_amounts()) {
        let h = Harness::new();
        let client = h.escrow_client();
        let total = sum(&amounts);
        let ms: SorobanVec<i128> = {
            let mut v = SorobanVec::new(&henv);
            for &a in &amounts {
                v.push_back(a);
            }
            v
        };
        let id = client.create_contract(
            &hen.client_addr,
            &hen.freelancer_addr,
            &None,
            &ms,
            &ReleaseAuthorization::ClientOnly,
        );

        assert!(try_deposit(&client, id, &h.client_addr, total));
        let n_ms = amounts.len() as u32;
        let all_idx: StdVec<u32> = (0..n_ms).collect();
        let res = try_refund(&client, &henv, id, &all_idx);
        prop_assert!(res.is_ok());

        let data = client.get_contract(&id);
        let available = data.funded_amount - data.released_amount - data.refunded_amount;
        prop_assert_eq!(available, 0);
        prop_assert_eq!(data.status, ContractStatus::Refunded);
    }
}
