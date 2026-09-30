use soroban_sdk::vec;

use super::{assert_contract_error, EscrowFixture, MILESTONE_TWO};
use crate::{ContractStatus, Error};

/// Refunds are available immediately from a fixture funded through real SAC custody.
#[test]
fn refund_returns_an_unreleased_milestone() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();
    let ids = vec![&fixture.env, 1_u32];

    assert_eq!(
        escrow.refund_unreleased_milestones(&fixture.escrow_id, &ids),
        MILESTONE_TWO
    );
    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Funded
    );
}

/// A completed fixture rejects refunds, preserving its terminal accounting state.
#[test]
fn refund_rejects_completed_contract() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();
    for index in 0..3_u32 {
        escrow.approve_milestone_release(&fixture.escrow_id, &fixture.client, &index);
        escrow.release_milestone(&fixture.escrow_id, &fixture.client, &index);
    }
    let ids = vec![&fixture.env, 0_u32];
    assert_contract_error(
        escrow.try_refund_unreleased_milestones(&fixture.escrow_id, &ids),
        Error::InvalidState,
    );
}

/// Invalid request shapes are rejected before escrow accounting or milestone flags change.
#[test]
fn refund_rejects_empty_duplicate_and_out_of_range_requests_without_mutation() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();

    assert_contract_error(
        escrow.try_refund_unreleased_milestones(&fixture.escrow_id, &vec![&fixture.env]),
        EscrowError::EmptyRefundRequest,
    );
    assert_contract_error(
        escrow.try_refund_unreleased_milestones(
            &fixture.escrow_id,
            &vec![&fixture.env, 1_u32, 1_u32],
        ),
        EscrowError::DuplicateMilestoneInRefund,
    );
    assert_contract_error(
        escrow.try_refund_unreleased_milestones(&fixture.escrow_id, &vec![&fixture.env, u32::MAX]),
        Error::IndexOutOfBounds,
    );

    let contract = escrow.get_contract(&fixture.escrow_id);
    assert_eq!(contract.refunded_amount, 0);
    assert_eq!(
        escrow.get_refundable_balance(&fixture.escrow_id),
        fixture.total_amount()
    );
    assert!(escrow
        .get_milestones(&fixture.escrow_id)
        .iter()
        .all(|m| !m.refunded));
}

/// Corrupt accounting fails closed with a diagnostic invariant error.
#[test]
fn refund_rejects_corrupt_available_balance_before_transfer() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();

    fixture.env.as_contract(&fixture.escrow_address, || {
        let key = crate::DataKey::Contract(fixture.escrow_id);
        let mut contract: crate::Contract = fixture.env.storage().persistent().get(&key).unwrap();
        contract.released_amount = contract.funded_amount + 1;
        fixture.env.storage().persistent().set(&key, &contract);
    });

    assert_contract_error(
        escrow.try_refund_unreleased_milestones(&fixture.escrow_id, &vec![&fixture.env, 0_u32]),
        Error::AccountingInvariantViolated,
    );

    let milestones = escrow.get_milestones(&fixture.escrow_id);
    assert!(!milestones.get(0).unwrap().refunded);
}

/// Malformed stored milestone amounts cannot produce zero or negative SAC transfers.
#[test]
fn refund_rejects_non_positive_milestone_amount() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();

    fixture.env.as_contract(&fixture.escrow_address, || {
        let key = (
            crate::DataKey::Contract(fixture.escrow_id),
            soroban_sdk::Symbol::new(&fixture.env, "milestones"),
        );
        let mut milestones: Vec<crate::Milestone> =
            fixture.env.storage().persistent().get(&key).unwrap();
        let mut milestone = milestones.get(0).unwrap();
        milestone.amount = 0;
        milestones.set(0, milestone);
        fixture.env.storage().persistent().set(&key, &milestones);
    });

    assert_contract_error(
        escrow.try_refund_unreleased_milestones(&fixture.escrow_id, &vec![&fixture.env, 0_u32]),
        Error::AmountMustBePositive,
    );
    assert!(
        !escrow
            .get_milestones(&fixture.escrow_id)
            .get(0)
            .unwrap()
            .refunded
    );
}
