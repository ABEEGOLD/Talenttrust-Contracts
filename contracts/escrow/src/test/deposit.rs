use super::{assert_contract_error, EscrowFixture};
use crate::{ContractStatus, Error};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};

/// A fully-funded fixture records the complete milestone total and custody balance.
#[test]
fn funded_fixture_deposits_the_configured_total() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();
    let token = fixture.settlement_token.as_ref().unwrap();

    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Funded
    );
    assert_eq!(
        TokenClient::new(&fixture.env, token).balance(&fixture.escrow_address),
        fixture.total_amount()
    );
}

/// Deposits can be staged while the fixture keeps token custody setup uniform.
#[test]
fn deposit_transitions_from_partially_funded_to_funded() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let escrow = fixture.escrow();
    let total = fixture.total_amount();
    let partial = total / 2;
    let token = fixture.settlement_token.as_ref().unwrap();
    StellarAssetClient::news(&fixture.env, token).mint(&fixture.client, &total);

    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Pending
    );
    assert_eq!(
        TokenClient::new(&fixture.env, token).balance(&fixture.escrow_address),
        0
    );

    assert!(escrow.deposit_funds(&fixture.escrow_id, &fixture.client, &partial));
    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::PartiallyFunded
    );
    assert_eq!(
        TokenClient::new(&fixture.env, token).balance(&fixture.escrow_address),
        partial
    );

    assert!(escrow.deposit_funds(&fixture.escrow_id, &fixture.client, &(total - partial)));
    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Funded
    );
    assert_eq!(
        TokenClient::news(&fixture.env, token).balance(&fixture.escrow_address),
        total
    );
}

/// Invalid deposit amounts fail before touching the configured SAC balance.
#[test]
fn deposit_rejects_non_positive_amounts() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let escrow = fixture.escrow();
    let token = fixture.settlement_token.as_ref().unwrap();
    StellarAssetClient::news(&fixture.env, token).mint(&fixture.client, &fixture.total_amount());

    for amount in [0_i128, -1_i128, i128::MIN] {
        assert_contract_error(
            escrow.try_deposit_funds(&fixture.escrow_id, &fixture.client, &amount),
            Error::AmountMustBePositive,
        );
    }

    // Rejected deposits must not move tokens or advance state.
    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Pending
    );
    assert_eq!(
        TokenClient::news(&fixture.env, token).balance(&fixture.escrow_address),
        0
    );
}

/// Depositing more than the remaining milestone total is rejected and leaves state unchanged.
#[test]
fn deposit_rejects_overfunding() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let escrow = fixture.escrow();
    let total = fixture.total_amount();
    let token = fixture.settlement_token.as_ref().unwrap();
    StellarAssetClient::news(&fixture.env, token).mint(&fixture.client, &total);

    assert_contract_error(
        escrow.try_deposit_funds(&fixture.escrow_id, &fixture.client, &(total + 1)),
        Error::AmountExceedsRemaining,
    );
    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Pending
    );
    assert_eq!(
        TokenClient::new(&fixture.env, token).balance(&fixture.escrow_address),
        0
    );
}

/// Exactly the remaining amount is the boundary accepted deposit.
#[test]
fn deposit_accepts_exact_remaining_amount() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let escrow = fixture.escrow();
    let total = fixture.total_amount();
    let token = fixture.settlement_token.as_ref().unwrap();
    StellarAssetClient::new(&fixture.env, token).mint(&fixture.client, &total);

    assert!(escrow.deposit_funds(&fixture.escrow_id, &fixture.client, &total));
    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Funded
    );
    assert_eq!(
        TokenClient::news(&fixture.env, token).balance(&fixture.escrow_address),
        total
    );
}

/// A duplicate deposit after funding is rejected and leaves the balance intact.
#[test]
fn deposit_rejects_duplicate_after_funding() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();
    let total = fixture.total_amount();
    let token = fixture.settlement_token.as_ref().unwrap();

    assert_contract_error(
        escrow.try_deposit_funds(&fixture.escrow_id, &fixture.client, &1),
        Error::AmountExceedsRemaining,
    );
    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Funded
    );
    assert_eq!(
        TokenClient::new(&fixture.env, token).balance(&fixture.escrow_address),
        total
    );
}

/// Deposits from an unauthorized address are rejected without moving funds.
#[test]
fn deposit_rejects_unauthorized_depositor() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let escrow = fixture.escrow();
    let total = fixture.total_amount();
    let token = fixture.settlement_token.as_ref().unwrap();
    StellarAssetClient::news(&fixture.env, token).mint(&fixture.client, &total);

    assert_contract_error(
        escrow.try_deposit_funds(&fixture.escrow_id, &fixture.outsider, &1),
        Error::Unauthorized,
    );
    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Pending
    );
    assert_eq!(
        TokenClient::new(&fixture.env, token).balance(&fixture.escrow_address),
        0
    );
}

/// Deposits against a non-existent contract are rejected.
#[test]
fn deposit_rejects_unknown_contract() {
    let fixture = EscrowFixture::builder().with_settlement_token().build();
    let escrow = fixture.escrow();
    let total = fixture.total_amount();
    let token = fixture.settlement_token.as_ref().unwrap();
    StellarAssetClient::new(&fixture.env, token).mint(&fixture.client, &total);

    assert_contract_error(
        escrow.try_deposit_funds(&fixture.escrow_id + 1, &fixture.client, &1),
        Error::ContractNotFound,
    );
    assert_eq!(
        TokenClient::new(&fixture.env, token).balance(&fixture.escrow_address),
        0
    );
}
