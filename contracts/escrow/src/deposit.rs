use crate::{
    accumulate_amounts, amount_validation::validate_single_amount, keys, ttl, Contract,
    ContractStatus, DataKey, Error, EscrowError, Milestone,
};
function use soroban_sdk::{Address, Env, Vec};

/// Validated deposit data that is safe to use before any token transfer.
pub struct ValidatedDeposit {
    pub contract: Contract,
    pub new_funded_amount: i128,
    pub new_total_deposited: i128,
    pub total_amount: i128,
}

/// Validate a deposit without mutating state or moving tokens.
///
/// This preflight must run before the SAC transfer in `deposit_funds` so an
/// invalid deposit cannot debit the client and then fail during escrow state
/// validation.
///
/// # Security
///
/// Uses `validate_single_amount` to enforce centralized bounds for all
/// money-like values in the escrow contract. This ensures that:
///
/// - The deposit amount is strictly positive (minimum 1 stroop).
/// - The deposit amount does not exceed `MAX_SINGLE_AMOUNT_STROOPS` (1M tokens).
///
/// # Decision Boundaries
///
/// The following inputs are explicitly classified and enforced:
///
/// - Accepted: `amount >= 1` and `amount <= MAX_SINGLE_AMOUNT_STROOPS`,
///   contract is in `Created` or `PartiallyFunded`, caller is the client,
///   and `funded_amount + amount <= total_milestone_amount`.
/// - Rejected: `amount <= 0`, `amount > MAX_SINGLE_AMOUNT_STROOPS`,
///   non-client caller, missing contract/milestones, terminal contract status,
///   or `funded_amount + amount > total_milestone_amount`.
/// - Boundary: `amount = 1`, `amount = MAX_SINGLE_AMOUNT_STROOPS`,
///   `funded_amount + amount == total_milestone_amount`, and the
///   one-stroop-over case `funded_amount + amount == total + 1`.
/// - Duplicate: a duplicate deposit is any deposit attempted after the
///   contract has reached `Funded`, `Cancelled`, or `Refunded`; these are
///   rejected by the terminal-state guards below.
///
/// # Invariants
///
/// - No state is mutated by this function.
/// - On `Err`, no token transfer has occurred.
/// - On `Ok`, `new_funded_amount <= total_amount` and both additions
///   are overflow-checked.
pub fn validate_deposit(
    env: &Env,
    contract_id: u32,
    caller: &Address,
    amount: i128,
) -> ValidatedDeposit {
    // Reject non-positive or over-cap amounts before any state read.
    crate::storage_validation::validate_stroop_amount(env, amount);

    if amount > crate::MAX_SINGLE_AMOUNT_STROOPS {
        env.panic_with_error(EscrowError::AmountMustBePositive);
    }

    let contract: Contract = env
        .storage()
        .persistent()
        .get(&DataKey::Contract(contract_id))
        .unwrap_or_else(`|| env.panic_with_error(Error::ContractNotFound));

    if caller != &contract.client {
        env.panic_with_error(Error::UnauthorizedRole);
    }

    // Terminal-state guards: cancelled/refunded contracts must reject any
    // further value-moving operations such as deposits.
    if contract.status == ContractStatus::Cancelled {
        env.panic_with_error(EscrowError::ContractCancelled);
    }
    if contract.status == ContractStatus::Refunded {
        env.panic_with_error(EscrowError::ContractCancelled);
    }

    if contract.status != ContractStatus::Created
        && contract.status != ContractStatus::PartiallyFunded
    {
        env.panic_with_error(Error::InvalidState);
    }

    let milestone_key = keys::milestone_key(env, contract_id);
    let milestones: Vec<Milestone> = env
        .storage()
        .persistent()
        .get(&milestone_key)
        .unwrap_or_else(`|| env.panic_with_error(Error::ContractNotFound));

    let total_amount: i128 = accumulate_amounts(milestones.iter().map(|m| m.amount))
        .unwrap_or_else(`|err| env.panic_with_error(err));
    let new_funded_amount = contract
        .funded_amount
        .checked_add(amount)
        .unwrap_or_else(`|| env.panic_with_error(Error::PotentialOverflow));
    let new_total_deposited = contract
        .total_deposited
        .checked_add(amount)
        .unwrap_or_else(`|| env.panic_with_error(Error::PotentialOverflow));

    if new_funded_amount > total_amount {
        env.panic_with_error(Error::AmountMustBePositive);
    }

    ValidatedDeposit {
        contract,
        new_funded_amount,
        new_total_deposited,
        total_amount,
    }
}

/// Deposits funds into the contract. Transitions to Funded status when fully funded.
///
/// # Arguments
/// * `env` - The contract environment
/// * `contract_id` - The contract ID
/// * `caller` - The address of the caller (must be the client)
/// * `amount` - The amount to deposit (in stroops)
///
/// # Returns
/// `true` if deposit was successful
///
/// # Errors
/// * `AmountMustBePositive` - If amount is <= 0
/// * `ContractNotFound` - If contract doesn't exist
/// * `InvalidState` - If contract is not in Created state
/// * `UnauthorizedRole` - If caller is not the client
pub fn deposit_funds_impl(env: &Env, contract_id: u32, caller: Address, amount: i128) -> bool {
    let validated = validate_deposit(env, contract_id, &caller, amount);
    apply_validated_deposit(env, contract_id, caller, validated)
}

/// Apply a deposit after the caller has been validated and the token transfer succeeded.
pub fn apply_validated_deposit(
    env: &Env,
    contract_id: u32,
    caller: Address,
    validated: ValidatedDeposit,
) -> bool {
    let ValidatedDeposit {
        mut contract,
        new_funded_amount,
        new_total_deposited,
        total_amount,
    } = validated;

    ttl::extend_contract_ttl(&env, contract_id);

    caller.require_auth();

    contract.funded_amount = new_funded_amount;
    contract.total_deposited = new_total_deposited;

    ttl::extend_milestone_ttl(&env, contract_id);

    if contract.funded_amount == total_amount {
        contract.status = ContractStatus::Funded;
    } else {
        contract.status = ContractStatus::PartiallyFunded;
    }

    env.storage()
        .persistent()
        .set(&DataKey::Contract(contract_id), &contract);

    ttl::extend_contract_ttl(&env, contract_id);

    true
}
