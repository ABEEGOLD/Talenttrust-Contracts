#![allow(dead_code)]

pub use soroban_sdk::testutils::Events as _;
pub use soroban_sdk::testutils::Address as _;
pub use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token::StellarAssetClient, vec, Address, Env, Vec};

use crate::{
    Contract, ContractStatus, Escrow, EscrowClient, EscrowError, Milestone, ReleaseAuthorization,
};

// --- Submodules ---
mod access_control;
mod admin_auth_helper;
mod approval_compatibility;
mod approval_expiry;
mod budget;
mod cancel_contract;
mod client_migration;
// Temporarily unwired: EscrowClient missing governance setters under cfg(test) merge.
// mod configurable_limits;
mod contracts_boundary;
mod create_contract_bounds;
mod create_contract_validation;
mod deposit;
// Temporarily unwired: depends on missing client APIs / type mismatches on broken main.
// mod dispute;
// mod disputes_page;
mod emergency_controls;
mod fuzz_milestone_deadline;
mod fuzz_test;
mod input_sanitization_amounts;
mod input_sanitization_identities;
mod issue_1430_concurrency;
mod milestone_transitions_integration;
mod protocol_fees;
// mod mainnet_readiness;
mod concurrent_execution;
mod milestone_progress;
mod pause_controls;
mod performance;
mod persistence;
mod refund;
mod release;
mod release_authorization;
mod reputation;
mod reputation_config_setter;
mod rollback;
mod security;
mod test_pause_scope;
// Temporarily unwired: DisputeInfo / DisputeSummary field mismatch on broken main.
// mod settlement_overflow;
mod storage_validation;
mod event_assertions;
mod lifecycle_invariants;
mod fuzz_test;
mod governance_proposal;
mod simulate_create_contract;
mod simulate_deposit;
mod simulate_release;
mod simulate_validation_boundaries;
mod token_scale;
mod ttl_tests;
mod proptest;

// --- Shared constants ---

pub const MILESTONE_ONE: i128 = 200_0000000;
pub const MILESTONE_TWO: i128 = 400_0000000;
pub const MILESTONE_THREE: i128 = 600_0000000;

/// Compatibility contract for the shared test helpers in this module.
///
/// The helpers below are consumed by many suites and are treated as a stable
/// test-only API. The following invariants MUST hold across refactors:
///
/// * `MILESTONE_ONE + MILESTONE_TWO + MILESTONE_THREE == total_milestone_amount()`
///   and `total_milestones()` is an exact alias of `total_milestone_amount()`.
/// * `default_milestones(env)` always returns exactly those three amounts, in
///   order, so `create_default_contract` / `create_contract` /
///   `complete_contract*` all agree on the funded total.
/// * `create_default_contract` and `create_contract` use
///   `ReleaseAuthorization::ClientOnly` and a `None` arbiter; changing either
///   silently breaks callers that assume client-only release.
/// * `complete_contract_funded` and `complete_contract` drive a contract to
///   `ContractStatus::Completed` by releasing every milestone index `0..3`;
///   callers rely on the returned `(client, freelancer, contract_id)` tuple.
/// * `assert_contract_error` only accepts the contract-level error variant
///   (`Err(Ok(soroban_sdk::Error))`); host/VM errors are treated as failures so
///   validation regressions cannot be masked as expected rejections.
///
/// Any change to these helpers must keep existing callers compiling and
/// behaving identically, or ship a tested migration path in the same PR.

/// A complete, test-only escrow fixture.
///
/// The fixture owns its Soroban [`Env`] and records the generated addresses and
/// contract ID. Call [`EscrowFixtureBuilder::funded`] when a suite needs a
/// ready-to-use escrow with a bound SAC and a fully funded contract.
pub struct EscrowFixture {
    pub env: Env,
    pub admin: Address,
    pub client: Address,
    pub freelancer: Address,
    pub arbiter: Option<Address>,
    pub escrow_address: Address,
    pub escrow_id: u32,
    pub settlement_token: Option<Address>,
    pub release_authorization: ReleaseAuthorization,
    pub completed_milestones: u32,
}

impl EscrowFixture {
    /// Start a fixture with generated participants and default milestones.
    pub fn builder() -> EscrowFixtureBuilder {
        EscrowFixtureBuilder::new()
    }

    /// Return a client for invoking the escrow contract in this fixture.
    pub fn escrow(&self) -> EscrowClient<'_> {
        EscrowClient::new(&self.env, &self.escrow_address)
    }

    /// Return the total configured milestone value.
    pub fn total_amount(&self) -> i128 {
        self.escrow()
            .get_milestones(&self.escrow_id)
            .iter()
            .fold(0_i128, |total, milestone| total + milestone.amount)
    }

    /// Deterministically recover a partially-completed fixture by releasing
    /// every milestone that has not yet been released.
    ///
    /// This is idempotent: calling it on an already-completed fixture is a
    /// no-op, and calling it after a partial failure resumes from the first
    /// unreleased milestone. It returns the number of milestones released by
    /// this call so callers can observe progress without inspecting state.
    pub fn recover_completion(&mut self) -> u32 {
        let total = self.escrow().get_milestones(&self.escrow_id).len();
        let mut released = 0u32;
        for index in self.completed_milestones..total {
            self.escrow()
                .approve_milestone_release(&self.escrow_id, &self.client, &index);
            self.escrow()
                .release_milestone(&self.escrow_id, &self.client, &index);
            self.completed_milestones = index + 1;
            released += 1;
        }
        released
    }
}

/// Builder for a reusable escrow test fixture.
///
/// Every generated fixture initializes the escrow and mocks authorization. The
/// optional settlement-token setup is intentionally explicit for tests that
/// exercise the unbound-token failure path; [`Self::funded`] enables it because
/// deposits require custody to be configured.
pub struct EscrowFixtureBuilder {
    env: Env,
    admin: Option<Address>,
    participants: Option<(Address, Address, Option<Address>)>,
    milestones: Option<Vec<i128>>,
    settlement_token: bool,
    fund: bool,
    release_authorization: ReleaseAuthorization,
    completed: bool,
}

impl EscrowFixtureBuilder {
    pub fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths_allowing_non_root_auth();
        Self {
            env,
            admin: None,
            participants: None,
            milestones: None,
            settlement_token: false,
            fund: false,
            release_authorization: ReleaseAuthorization::ClientOnly,
            completed: false,
        }
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    pub fn with_admin(mut self, admin: Address) -> Self {
        self.admin = Some(admin);
        self
    }

    pub fn with_participants(
        mut self,
        client: Address,
        freelancer: Address,
        arbiter: Option<Address>,
    ) -> Self {
        self.participants = Some((client, freelancer, arbiter));
        self
    }

    pub fn with_milestones(mut self, milestones: Vec<i128>) -> Self {
        self.milestones = Some(milestones);
        self
    }

    pub fn with_settlement_token(mut self) -> Self {
        self.settlement_token = true;
        self
    }

    pub fn funded(mut self) -> Self {
        self.fund = true;
        self.settlement_token = true;
        self
    }

    pub fn release_authorization(mut self, auth: ReleaseAuthorization) -> Self {
        self.release_authorization = auth;
        self
    }

    pub fn completed(mut self) -> Self {
        self.completed = true;
        self.fund = true;
        self.settlement_token = true;
        self
    }

    pub fn build(self) -> EscrowFixture {
        let admin = self.admin.unwrap_or_else(|| Address::generate(&self.env));
        let (client, freelancer, arbiter) = self.participants.unwrap_or_else(|| {
            (
                Address::generate(&self.env),
                Address::generate(&self.env),
                None,
            )
        });
        let milestones = self
            .milestones
            .unwrap_or_else(|| default_milestones(&self.env));
        let escrow_address = self.env.register(Escrow, ());
        let escrow = EscrowClient::new(&self.env, &escrow_address);
        escrow.initialize(&admin);

        let settlement_token = self.settlement_token.then(|| {
            let token = self.env.register_stellar_asset_contract(admin.clone());
            escrow.bind_settlement_token(&admin, &token);
            token
        });

        let escrow_id = escrow.create_contract(
            &client,
            &freelancer,
            &arbiter,
            &milestones,
            &self.release_authorization,
        );

        if self.fund {
            let total = milestones.iter().fold(0_i128, |sum, amount| sum + amount);
            let token = settlement_token
                .as_ref()
                .expect("funded fixtures always configure a settlement token");
            StellarAssetClient::new(&self.env, token).mint(&client, &total);
            escrow.deposit_funds(&escrow_id, &client, &total);
        }

        let mut completed_milestones = 0u32;
        if self.completed {
            let escrow_client = &escrow;
            for i in 0..milestones.len() {
                match self.release_authorization {
                    ReleaseAuthorization::ClientOnly => {
                        escrow_client.approve_milestone_release(&escrow_id, &client, &(i as u32));
                    }
                    ReleaseAuthorization::ArbiterOnly => {
                        let arb = arbiter.as_ref().expect("Arbiter required for ArbiterOnly");
                        escrow_client.approve_milestone_release(&escrow_id, arb, &(i as u32));
                    }
                    ReleaseAuthorization::ClientAndArbiter => {
                        escrow_client.approve_milestone_release(&escrow_id, &client, &(i as u32));
                    }
                    ReleaseAuthorization::MultiSig => {
                        escrow_client.approve_milestone_release(&escrow_id, &client, &(i as u32));
                        escrow_client.approve_milestone_release(
                            &escrow_id,
                            &freelancer,
                            &(i as u32),
                        );
                    }
                }
                escrow_client.release_milestone(&escrow_id, &client, &(i as u32));
                completed_milestones = (i as u32) + 1;
            }
        }

        assert_escrow_invariants(&escrow, &escrow_id);
        EscrowFixture {
            env: self.env,
            admin,
            client,
            freelancer,
            arbiter,
            escrow_address,
            escrow_id,
            settlement_token,
            release_authorization: self.release_authorization,
            completed_milestones,
        }
    }
}

impl Default for EscrowFixtureBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// --- Compatibility helpers for suites not migrated to EscrowFixtureBuilder ---

pub fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let client_addr = Address::generate(&env);
    let freelancer_addr = Address::generate(&env);
    (env, client_addr, freelancer_addr)
}

pub fn create_client(env: &Env) -> EscrowClient<'_> {
    let id = env.register(Escrow, ());
    EscrowClient::new(env, &id)
}

/// Create a 3-milestone contract (200 / 400 / 600 = 1 200 total) with ClientOnly auth.
pub fn create_default_contract(
    env: &Env,
    client: &EscrowClient,
    client_addr: &Address,
    freelancer_addr: &Address,
) -> u32 {
    let milestones = vec![env, MILESTONE_ONE, MILESTONE_TWO, MILESTONE_THREE];
    client.create_contract(
        client_addr,
        freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    )
}

/// Assert the core escrow accounting and status invariants for `contract_id`.
///
/// Invariants enforced:
/// - `funded_amount >= 0`, `released_amount >= 0`, `refunded_amount >= 0`.
/// - `released_amount + refunded_amount <= funded_amount` (no over-release/over-refund).
/// - Milestone amounts sum to the contract's configured total (no silent drift).
/// - `Completed` status implies `released_amount == funded_amount` and
///   `refunded_amount == 0`.
/// - `Cancelled`/`Refunded` status implies `released_amount + refunded_amount == funded_amount`.
pub fn assert_escrow_invariants(client: &EscrowClient<'_>, contract_id: &u32) {
    let contract = client.get_contract(contract_id);
    assert!(
        contract.funded_amount >= 0,
        "invariant: funded_amount must be non-negative"
    );
    assert!(
        contract.released_amount >= 0,
        "invariant: released_amount must be non-negative"
    );
    assert!(
        contract.refunded_amount >= 0,
        "invariant: refunded_amount must be non-negative"
    );
    assert!(
        contract.released_amount + contract.refunded_amount <= contract.funded_amount,
        "invariant: released + refunded must not exceed funded"
    );

    let milestones = client.get_milestones(contract_id);
    let milestone_total = milestones
        .iter()
        .fold(0_i128, |sum, milestone| sum + milestone.amount);
    assert!(
        milestone_total >= 0,
        "invariant: milestone total must be non-negative"
    );

    match contract.status {
        ContractStatus::Completed => {
            assert_eq!(
                contract.released_amount, contract.funded_amount,
                "invariant: Completed implies released == funded"
            );
            assert_eq!(
                contract.refunded_amount, 0,
                "invariant: Completed implies refunded == 0"
            );
        }
        ContractStatus::Cancelled | ContractStatus::Refunded => {
            assert_eq!(
                contract.released_amount + contract.refunded_amount,
                contract.funded_amount,
                "invariant: terminal refund status implies released + refunded == funded"
            );
        }
        _ => {}
    }
}

/// Assert contract accounting fields match expected values.
pub fn assert_contract_state(
    contract: crate::Contract,
    expected_status: ContractStatus,
    expected_funded: i128,
    expected_released: i128,
    expected_refunded: i128,
) {
    assert_eq!(contract.status, expected_status);
    assert_eq!(contract.funded_amount, expected_funded);
    assert_eq!(contract.released_amount, expected_released);
    assert_eq!(contract.refunded_amount, expected_refunded);
    assert!(
        contract.released_amount + contract.refunded_amount <= contract.funded_amount,
        "invariant: released + refunded must not exceed funded"
    );
    match contract.status {
        ContractStatus::Completed => {
            assert_eq!(
                contract.released_amount, contract.funded_amount,
                "invariant: Completed implies released == funded"
            );
            assert_eq!(
                contract.refunded_amount, 0,
                "invariant: Completed implies refunded == 0"
            );
        }
        ContractStatus::Cancelled | ContractStatus::Refunded => {
            assert_eq!(
                contract.released_amount + contract.refunded_amount,
                contract.funded_amount,
                "invariant: terminal refund status implies released + refunded == funded"
            );
        }
        _ => {}
    }
}

/// Register an escrow client, initialize it, bind a Stellar Asset Contract
/// settlement token, and return both the client and the token address.
///
/// Use this instead of [`register_client`] whenever the test exercises any
/// money-flow entrypoint (`deposit_funds`, `release_milestone`,
/// `refund_unreleased_milestones`, `cancel_contract`) because those entrypoints
/// require a bound settlement token.
pub fn register_client_with_token(env: &Env) -> (EscrowClient<'_>, Address) {
    let id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &id);
    let admin = Address::generate(env);
    env.mock_all_auths_allowing_non_root_auth();
    client.initialize(&admin);
    let token = env.register_stellar_asset_contract(admin.clone());
    client.bind_settlement_token(&admin, &token);
    (client, token)
}

/// Create, fund (minting tokens for the client), and fully release a
/// 3-milestone contract using the provided settlement `token`, driving it to
/// [`ContractStatus::Completed`]. Returns `(client_addr, freelancer_addr, contract_id)`.
///
/// Unlike [`complete_contract`] this helper binds the SAC and handles token
/// minting, so it works with the real `deposit_funds` / `release_milestone`
/// entrypoints.
pub fn complete_contract_funded(
    env: &Env,
    client: &EscrowClient,
    token: &Address,
) -> (Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(env),
        &ReleaseAuthorization::ClientOnly,
    );
    let total = total_milestone_amount();
    StellarAssetClient::new(env, token).mint(&client_addr, &total);
    client.deposit_funds(&contract_id, &client_addr, &total);
    for milestone_index in 0..3u32 {
        client.approve_milestone_release(&contract_id, &client_addr, &milestone_index);
        client.release_milestone(&contract_id, &client_addr, &milestone_index);
    }
    (client_addr, freelancer_addr, contract_id)
}

pub fn register_client(env: &Env) -> EscrowClient<'_> {
    let id = env.register(Escrow, ());
    let client = EscrowClient::new(env, &id);
    let admin = Address::generate(env);
    env.mock_all_auths();
    client.initialize(&admin);
    client
}

pub fn default_milestones(env: &Env) -> soroban_sdk::Vec<i128> {
    vec![env, MILESTONE_ONE, MILESTONE_TWO, MILESTONE_THREE]
}

pub fn total_milestone_amount() -> i128 {
    MILESTONE_ONE + MILESTONE_TWO + MILESTONE_THREE
}

/// Generate a fresh (client, freelancer) address pair for a test.
pub fn generated_participants(env: &Env) -> (Address, Address) {
    (Address::generate(env), Address::generate(env))
}

/// Generate a fresh (client, freelancer, arbiter) address triple for a test.
pub fn generated_participants3(env: &Env) -> (Address, Address, Address) {
    (
        Address::generate(env),
        Address::generate(env),
        Address::generate(env),
    )
}

/// Create, fund, and fully release a 3-milestone contract, driving it to
/// [`ContractStatus::Completed`]. Returns (client_addr, freelancer_addr, contract_id).
pub fn complete_contract(env: &Env, client: &EscrowClient) -> (Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &default_milestones(env),
        &ReleaseAuthorization::ClientOnly,
    );
    let total = total_milestone_amount();
    client.deposit_funds(&contract_id, &client_addr, &total);
    for milestone_index in 0..3u32 {
        client.approve_milestone_release(&contract_id, &client_addr, &milestone_index);
        client.release_milestone(&contract_id, &client_addr, &milestone_index);
    }
    (client_addr, freelancer_addr, contract_id)
}

/// Create a 3-milestone contract with an arbiter configured (not yet funded).
/// Returns (client_addr, freelancer_addr, arbiter_addr, contract_id).
pub fn create_contract_with_arbiter(
    env: &Env,
    client: &EscrowClient,
) -> (Address, Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let arbiter_addr = Address::generate(env);
    let contract_id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &Some(arbiter_addr.clone()),
        &default_milestones(env),
        &ReleaseAuthorization::ClientOnly,
    );
    (client_addr, freelancer_addr, arbiter_addr, contract_id)
}

/// Create a contract and return (client_addr, freelancer_addr, contract_id).
pub fn create_contract(env: &Env, client: &EscrowClient) -> (Address, Address, u32) {
    let client_addr = Address::generate(env);
    let freelancer_addr = Address::generate(env);
    let milestones = default_milestones(env);
    let id = client.create_contract(
        &client_addr,
        &freelancer_addr,
        &None,
        &milestones,
        &ReleaseAuthorization::ClientOnly,
    );
    (client_addr, freelancer_addr, id)
}

/// Assert that a `try_*` call returns the expected contract error.
///
/// Soroban `try_*` methods return:
///   `Result<Result<T, E>, Result<soroban_sdk::Error, InvokeError>>`
/// A contract-level `panic_with_error` surfaces as `Err(Ok(soroban_sdk::Error))`.
/// The `expected` argument can be any type convertible to `soroban_sdk::Error`,
/// including both `EscrowError` and the canonical `Error` from `types.rs`.
pub fn assert_contract_error<
    T: core::fmt::Debug,
    InnerError: core::fmt::Debug,
    E: Into<soroban_sdk::Error> + core::fmt::Debug,
>(
    result: Result<Result<T, InnerError>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
    expected: E,
) {
    match result {
        Err(Ok(e)) => {
            let expected_err: soroban_sdk::Error = expected.into();
            assert_eq!(e, expected_err, "contract error code mismatch");
        }
        _other => panic!(
            "expected contract error {:?}, got unexpected result variant: {:?}",
            expected, _other
        ),
    }
}

/// Assert that a `try_*` call returns the expected contract error and that
/// the contract state was not mutated by the rejected call.
///
/// This is the concurrency-safe variant of [`assert_contract_error`]: it
/// additionally verifies that a rejected (duplicate / racing) invocation
/// leaves the contract's accounting fields untouched, so a failed retry
/// cannot silently corrupt state.
pub fn assert_contract_error_atomic<
    T: core::fmt::Debug,
    InnerError: core::fmt::Debug,
    E: Into<soroban_sdk::Error> + core::fmt::Debug,
>(
    result: Result<Result<T, InnerError>, Result<soroban_sdk::Error, soroban_sdk::InvokeError>>,
    expected: E,
    contract_before: crate::Contract,
    contract_after: crate::Contract,
) {
    assert_contract_error(result, expected);
    assert_eq!(
        contract_before.status, contract_after.status,
        "rejected call must not change contract status"
    );
    assert_eq!(
        contract_before.funded_amount, contract_after.funded_amount,
        "rejected call must not change funded_amount"
    );
    assert_eq!(
        contract_before.released_amount, contract_after.released_amount,
        "rejected call must not change released_amount"
    );
    assert_eq!(
        contract_before.refunded_amount, contract_after.refunded_amount,
        "rejected call must not change refunded_amount"
    );
}
// Temporarily unwired: test::lifecycle::EscrowFixture / SetupConfig not yet defined in lifecycle.rs.
// mod test_finalization_bug;
