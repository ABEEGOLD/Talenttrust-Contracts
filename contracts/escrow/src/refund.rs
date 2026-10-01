//! Atomic refund transition for the public escrow entrypoint.
//! Retries retain the existing rejection semantics: an already refunded milestone
//! is rejected, rather than reported as a second successful payout.

use crate::{
    rollback, ttl, Contract, ContractStatus, DataKey, Error, Escrow, EscrowError, Milestone,
};
use soroban_sdk::{symbol_short, token, Env, Vec};

pub(crate) fn execute(env: Env, contract_id: u32, milestone_indices: Vec<u32>) -> i128 {
    Escrow::require_not_paused(&env);
    // Validate non-empty request
    if milestone_indices.is_empty() {
        env.panic_with_error(EscrowError::EmptyRefundRequest);
    }

    // Check for duplicates
    for i in 0..milestone_indices.len() {
        for j in (i + 1)..milestone_indices.len() {
            if milestone_indices.get(i).unwrap() == milestone_indices.get(j).unwrap() {
                env.panic_with_error(EscrowError::DuplicateMilestoneInRefund);
            }
        }
    }

    let mut contract: Contract = Escrow::require_active_contract(&env, contract_id);
    let was_disputed = contract.status == ContractStatus::Disputed;

    // Only allow refunds while the contract is still in an active,
    // unreleased state. Cancelled, Completed, and Refunded contracts
    // must not be refundable again.
    if contract.status != ContractStatus::Created
        && contract.status != ContractStatus::Funded
        && contract.status != ContractStatus::Disputed
    {
        env.panic_with_error(EscrowError::InvalidState);
    }

    contract.client.require_auth();

    let mut milestones: Vec<Milestone> = ttl::load_milestones(&env, contract_id);

    let mut total_refund_amount: i128 = 0;

    // Validate all milestones first
    for idx in milestone_indices.iter() {
        if idx >= milestones.len() {
            env.panic_with_error(Error::IndexOutOfBounds);
        }

        let milestone = milestones.get(idx).unwrap();

        // SECURITY: Check if milestone is already released
        if milestone.released {
            env.panic_with_error(Error::MilestoneAlreadyReleased);
        }

        // SECURITY: Check if milestone is already refunded
        if milestone.refunded {
            env.panic_with_error(EscrowError::AlreadyRefunded);
        }

        // SECURITY: Check timeout refund conditions - milestone must be overdue if deadline is set
        if milestone.deadline.is_some() {
            // Milestone has a deadline - check if it's overdue
            if !Escrow::is_milestone_overdue(env.clone(), contract_id, idx) {
                // Deadline set but milestone not yet overdue
                env.panic_with_error(Error::MilestoneNotOverdue);
            }
        }
        // If no deadline (None), allow refund anytime (backward compatibility)

        if milestone.amount <= 0 {
            env.panic_with_error(Error::AmountMustBePositive);
        }
        total_refund_amount = total_refund_amount
            .checked_add(milestone.amount)
            .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
    }

    // released_amount records net payouts, but retained fees are already spent
    // by this escrow. Reserve each released milestone's gross amount, rather
    // than the global fee counter (which also includes unrelated escrows).
    let mut gross_released = 0_i128;
    for milestone in milestones.iter().filter(|milestone| milestone.released) {
        if milestone.amount <= 0 {
            env.panic_with_error(Error::AccountingInvariantViolated);
        }
        gross_released = gross_released
            .checked_add(milestone.amount)
            .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
    }
    if contract.funded_amount < 0
        || contract.released_amount < 0
        || contract.refunded_amount < 0
        || contract.released_amount > gross_released
    {
        env.panic_with_error(Error::AccountingInvariantViolated);
    }
    let available_balance = contract
        .funded_amount
        .checked_sub(gross_released)
        .and_then(|remaining| remaining.checked_sub(contract.refunded_amount))
        .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));
    if available_balance < total_refund_amount {
        env.panic_with_error(EscrowError::InsufficientFunds);
    }

    let token = Escrow::read_settlement_token(&env)
        .unwrap_or_else(|| env.panic_with_error(Error::SettlementTokenNotConfigured));

    // Soroban serializes ledger writes. Competing/overlapping refunds therefore
    // observe the committed flags, while failed invocations roll back all writes,
    // events and token movements. Finalize effects before the external transfer;
    // never persist a snapshot again after that interaction.
    // Mark milestones as refunded
    for idx in milestone_indices.iter() {
        let mut milestone = milestones.get(idx).unwrap();
        milestone.refunded = true;
        milestone.refunded_amount = milestone.amount;
        milestones.set(idx, milestone);
    }

    contract.refunded_amount = contract
        .refunded_amount
        .checked_add(total_refund_amount)
        .unwrap_or_else(|| env.panic_with_error(Error::PotentialOverflow));

    // Check if all unreleased milestones are refunded
    let all_refunded_or_released = milestones.iter().all(|m| m.released || m.refunded);
    if all_refunded_or_released {
        let all_refunded = milestones.iter().all(|m| m.refunded);
        if all_refunded {
            contract.status = ContractStatus::Refunded;
        } else {
            // Some released, some refunded
            contract.status = ContractStatus::Completed;
            Escrow::grant_pending_reputation_credit(&env, &contract.freelancer);
        }
    }

    ttl::store_milestones(&env, contract_id, &milestones);
    env.storage()
        .persistent()
        .set(&DataKey::Contract(contract_id), &contract);

    if was_disputed {
        rollback::clear_dispute_rollback(&env, contract_id);
    }

    // Extend TTL on contract write (milestone TTL already extended by store_milestones)
    ttl::extend_contract_ttl(&env, contract_id);

    // Emit `refunded` event after all state mutations succeed.
    //
    // Topics : `(symbol_short!("refunded"), contract_id: u32)`
    // Data   : `(total_refund_amount: i128, new_status: ContractStatus, timestamp: u64)`
    env.events().publish(
        (symbol_short!("refunded"), contract_id),
        (
            total_refund_amount,
            contract.status,
            env.ledger().timestamp(),
        ),
    );

    let token_client = token::Client::new(&env, &token);
    token_client.transfer(
        &env.current_contract_address(),
        &contract.client,
        &total_refund_amount,
    );

    total_refund_amount
}
