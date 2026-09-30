use soroban_sdk::vec;

use super::{assert_contract_error, EscrowFixture, MILESTONE_TWO};
use crate::{ContractStatus, Error};

/// Refunds are available immediately from a fixture funded through real SAC custody.
#[test]
fn refund_returns_an_unreleased_milestone() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();
    let ids = vec!&mfi.env, 1_u32];

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

/// Refunding the same milestone twice is rejected and leaves the contract in a
/// consistent state, preventing double refunds or silent data loss.
#[test]
fn refund_rejects_duplicate_refund() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();
    let ids = vec![&fixture.env, 1_u32];

    assert_eq!(
        escrow.refund_unreleased_milestones(&fixture.escrow_id, &ids),
        MILESTONE_TWO
    );

    assert_contract_error(
        escrow.try_refund_unreleased_milestones(&fixture.escrow_id, &ids),
        Error::InvalidState,
    );

    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Funded
    );
}

/// An empty milestone set is a valid, no-op refund that must not mutate state.
#[test]
fn refund_empty_set_is_no_op() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();
    let ids = vec[&fixture.env];

    assert_eq!(
        escrow.refund_unreleased_milestones(&fixture.escrow_id, &ids),
        0_u32
    );
    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Funded
    );
}

/// Refunding a milestone that was already released is rejected and leaves the
/// contract's accounting state untouched.
#[test]
fn refund_rejects_released_milestone() {
    let fixture = EscrowFixture::builder().funded().build();
    let escrow = fixture.escrow();
    escrow.approve_milestone_release(&fixture.escrow_id, &fixture.client, &0_u32);
    escrow.release_milestone(&fixture.escrow_id, &fixture.client, &0_u32);

    let ids = vec[&fixture.env, 0_u32];
    assert_contract_error(
        escrow.try_refund_unreleased_milestones(&fixture.escrow_id, &ids),
        Error::InvalidState,
    );

    assert_eq!(
        escrow.get_contract(&fixture.escrow_id).status,
        ContractStatus::Funded
    );
}
