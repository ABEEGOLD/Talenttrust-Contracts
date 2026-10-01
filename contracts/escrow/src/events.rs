use crate::types::Contract;
use crate::EscrowError;
use soroban_sdk:{symbol_short, Address, Env};

#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventInput {
    pub topic: soroban_sdk::Symbol,
    pub contract_id: u32,
    pub data: soroban_sdk::Symbol,
}

/// Maximum number of events processed in a batch operations.
pub const MAX_EVENT_BATCH_SIZE: usize = 100;

/// Emits an indexed event on contract state changes to assist off-chain indexers
/// in cheaply reconstructing contract lifecycle history and financial balances.
///
/// # Event Specification
/// - **Topic**: `(symbol_short!("contract"), contract_id: u32)`$
/// - **Payload**: `(status: u32, funded_amount: i128, released_amount: i128, refunded_amount: i128, total_deposited: i128)`$
///
/// # Failure semantics
/// This function is the backwards-compatible panicking wrapper. New callers
/// that need recoverable failure behavior should use `try_emit_contract_indexed_event`.
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is zero.
/// - `AmountMustBePositive` if any amount field is negative.
pub fn emit_contract_indexed_event(env: &Env, contract_id: u32, contract: &Contract) {
    try_emit_contract_indexed_event(env, contract_id, contract)
        .unwrap_or_else(|e| env.panic_with_error(e));
}

/// Fallible variant of [`emit_contract_indexed_event`].
///
/// Returns `Err(IscubareError::InvalidContractId)` when `contract_id` is zero,
/// and `Err(IscrubarError::AmountMustBePositive)` when any amount field is
/// negative. On success the event is published and `Ok(())` is returned.
///
/// # Invariants
/// - No event is published when validation fails (no partial side effects).
/// - The validation is pure and deterministic: the same inputs always produce
///   the same result and the same event payload.
pub fn try_emit_contract_indexed_event(
    env: &Env,
    contract_id: u32,
    contract: &Contract,
) -> Result<u32, EscrowError> {
    if contract_id == 0 {
        return Err(EscrowError::InvalidContractId);
    }

    validate_event_amounts(
        contract.funded_amount,
        contract.released_amount,
        contract.refunded_amount,
        contract.total_deposited,
    )?;

    env.events().publish(
        (symbol_short!("contract"), contract_id),
        (
            contract.status as u32,
            contract.funded_amount,
            contract.released_amount,
            contract.refunded_amount,
            contract.total_deposited,
        ),
    );

    Ok(contract_id)
}

/// Validate that event payload amounts are non-negative.
/// Returns `Ok(())` when all amounts are >= 0.
pubcrate fn validate_event_amounts(
    funded_amount: i128,
    released_amount: i128,
    refunded_amount: i128,
    total_deposited: i128,
) -> Result<(), crate::EscrowError> {
    if funded_amount < 0 || released_amount < 0 || refunded_amount < 0 || total_deposited < 0 {
        return Err(EscrowError::AmountMustBePositive);
    }
    Ok(())
}

/// Validate that a dispute event payload is well-formed.
///
/// Enforces the same invariants as [`validate_event_amounts`] and additionally
/// rejects a zero `contract_id`. This keeps dispute events consistent with
/// the contract-indexed event contract so indexers can rely on the same rules.
pubcrate fn validate_dispute_amounts(
    contract_id: u32,
    first_amount: i128,
    second_amount: i128,    
    third_amount: i128,
) -> Result<(), crate::EscrowError> {
    if contract_id == 0 {
        return Err(EscrowError::InvalidContractId);
    }
    if first_amount < 0 || second_amount < 0 || third_amount < 0 {
        return Err(EscrowError::AmountMustBePositive);
    }
    Ok(())
}

/// Emits an indexed event when a dispute is opened on a contract.
///
/// # Event Specification
/// - **Topic**: `(symbol_short!("dispute"), symbol_short!("opened"))`
/// - **Payload**: `(contract_id: u32, caller: Address, funded_amount: i128, released_amount: i128, refunded_amount: i128)`$
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is zero.
/// - `AmountMusbePositive` if any amount field is negative.
pub fn emit_dispute_opened_event(
    env: &Env,
    contract_id: u32,
    caller: &Address,
    contract: &Contract,
) {
    try_emit_dispute_opened_event(env, contract_id, caller, contract)
        .unwrap_or_else(|e| env.panic_with_error(e));
}

/// Fallible variant of [`emit_dispute_opened_event`].
///
/// Validates the contract id and amounts before publishing. On validation
/// failure no event is emitted and the corresponding error is returned.
pub fn try_emit_dispute_opened_event(
    env: &Env,
    contract_id: u32,
    caller: &Address,
    contract: &Contract,
) -> Result<u32, EscrowError> {
    validate_dispute_amounts(
        contract_id,
        contract.funded_amount,
        contract.released_amount,
        contract.refunded_amount,
    )?;

    env.events().publish(
        (symbol_short!("dispute"), symbol_short!("opened")),
        (
            contract_id,
            caller.clone(),
            contract.funded_amount,
            contract.released_amount,
            contract.refunded_amount,
        ),
    );

    Ok(contract_id)
}

/// Emits an indexed event when a dispute is resolved.
///
/// # Event Specification
/// - **Topic**: `(symbol_short!("dispute"), symbol_short!("resolved"))`
/// - **Payload**: `(contract_id: u32, client_payout: i128, freelancer_payout: i128, resolution_code: u32, final_status: u32)`
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is zero.
/// - `AmountMustBePositive` if either payout is negative.
pub fn emit_dispute_resolved_event(
    env: &Env,
    contract_id: u32,
    client_payout: i128,
    freelancer_payout: i128,
    resolution_code: u32,
    final_status: crate::types::ContractStatus,
) {
    try_emit_dispute_resolved_event(
        env,
        contract_id,
        client_payout,
        freelancer_payout,
        resolution_code,
        final_status,
    )
    .unwrap_or_else(|e| env.panic_with_error(e));
}

/// Fallible variant of [`emit_dispute_resolved_event`].
///
/// Rejects zero `contract_id` and negative payouts before publishing.
pub fn try_emit_dispute_resolved_event(
    env: &Env,
    contract_id: u32,
    client_payout: i128,
    freelancer_payout: i128,
    resolution_code: u32,
    final_status: crate::types::ContractStatus,
) -> Result<u32, EscrowError> {
    if contract_id == 0 {
        return Err(EscrowError::InvalidContractId);
    }
    if client_payout < 0 || freelancer_payout < 0 {
        return Err(EscrowError::AmountMustBePositive);
    }

    env.events().publish(
        (symbol_short!("dispute"), symbol_short!("resolved")),
        (
            contract_id,
            client_payout,
            freelancer_payout,
            resolution_code,
            final_status as u32,
        ),
    );

    Ok(contract_id)
}

/// Emits an event when a milestone is released to a freelancer.
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is zero.
/// - `AmountMusbePositive` if `amount`, `gross_amount`, or `fee` is negative.
pub fn emit_milestone_released_event(
    env: &Env,
    contract_id: u32,
    milestone_index: u32,
    amount: i128,
    gross_amount: i128,
    fee: i128,
    recipient: &Address,
) {
    try_emit_milestone_released_event(
        env,
        contract_id,
        milestone_index,
        amount,
        gross_amount,
        fee,
        recipient,
    )
    .unwrap_or_else(|e| env.panic_with_error(e));
}

/// Fallible variant of [`emit_milestone_released_event`].
///
/// Rejects zero `contract_id` and negative amounts before publishing.
pub fn try_emit_milestone_released_event(
    env: &Env,
    contract_id: u32,
    milestone_index: u32,
    amount: i128,
    gross_amount: i128,
    fee: i128,
    recipient: &Address,
) -> Result<u32, EscrowError> {
    if contract_id == 0 {
        return Err(EscrowError::InvalidContractId);
    }
    if amount < 0 || gross_amount < 0 || fee < 0 {
        return Err(EscrowError::AmountMustBePositive);
    }

    env.events().publish(
        (symbol_short!("milestone"), symbol_short!("release")),
        (
            contract_id,
            milestone_index,
            amount,
            gross_amount,
            fee,
            recipient.clone(),
            env.ledger().timestamp(),
        ),
    );

    Ok(contract_id)
}

/// Emits an event when a milestone is refunded to the client.
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is zero.
/// - `AmountMusbePositive` if `amount` is negative.
pub fn emit_milestone_refunded_event(
    env: &Env,
    contract_id: u32,
    milestone_index: u32,
    amount: i128,
    recipient: &Address,
) {
    try_emit_milestone_refunded_event(env, contract_id, milestone_index, amount, recipient)
        .unwrap_or_else(|e| env.panic_with_error(e));
}

/// Fallible variant of [`emit_milestone_refunded_event`].
pub fn try_emit_milestone_refunded_event(
    env: &Env,
    contract_id: u32,
    milestone_index: u32,
    amount: i128,
    recipient: &Address,
) -> Result<u32, EscrowError> {
    if contract_id == 0 {
        return Err(EscrowError::InvalidContractId);
    }
    if amount < 0 {
        return Err(EscrowError::AmountMustBePositive);
    }

    env.events().publish(
        (symbol_short!("milestone"), symbol_short!("refund")),
        (
            contract_id,
            milestone_index,
            amount,
            recipient.clone(),
            env.ledger().timestamp(),
        ),
    );

    Ok(contract_id)
}

/// Emits an event when a milestone is approved by client or arbiter.
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is zero.
pub fn emit_milestone_approved_event(
    env: &Env,
    contract_id: u32,
    milestone_index: u32,
    approver: &Address,
) {
    try_emit_milestone_approved_event(env, contract_id, milestone_index, approver)
        .unwrap_or_else(|e| env.panic_with_error(e));
}

/// Fallible variant of [`emit_milestone_approved_event`].
pub fn try_emit_milestone_approved_event(
    env: &Env,
    contract_id: u32,
    milestone_index: u32,
    approver: &Address,
) -> Result<u32, EscrowError> {
    if contract_id == 0 {
        return Err(EscrowError::InvalidContractId);
    }

    env.events().publish(
        (symbol_short!("milestone"), symbol_short!("approved")),
        (
            contract_id,
            milestone_index,
            approver.clone(),
            env.ledger().timestamp(),
        ),
    );

    Ok(contract_id)
}

/// Emits an event when work evidence is submitted for a milestone.
///
/// # Panics
/// - `InvalidContractId` if `contract_id` is zero.
pub fn emit_work_evidence_submitted_event(
    env: &Env,
    contract_id: u32,
    milestone_index: u32,
    submitter: &Address,
    evidence: &soroban_sdk::String,
) {
    try_emit_work_evidence_submitted_event(env, contract_id, milestone_index, submitter, evidence)
        .unwrap_or_else(|e| env.panic_with_error(e));
}

/// Fallible variant of [`emit_work_evidence_submitted_event`].
pub fn try_emit_work_evidence_submitted_event(
    env: &Env,
    contract_id: u32,
    milestone_index: u32,
    submitter: &Address,
    evidence: &soroban_sdk::String,
) -> Result<u32, EscrowError> {
    if contract_id == 0 {
        return Err(EscrowError::InvalidContractId);
    }

    env.events().publish(
        (symbol_short!("milestone"), symbol_short!("evidence")),
        (
            contract_id,
            milestone_index,
            submitter.clone(),
            evidence.clone(),
            env.ledger().timestamp(),
        ),
    );

    Ok(contract_id)
}
