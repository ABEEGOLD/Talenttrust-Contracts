//! Amount validation and sanitization module
///
/// Provides centralized validation for all money-like values in the escrow contract.
/// Ensures positivity, max bounds, and proper stroop precision handling.
///
/// Storage ownership: none. This module is deliberately stateless; callers use
/// these helpers before writing validated values to contract and milestone
/// storage.

/// Maximum number of decimal places for stroop precision (7 decimal places for Stellar)
#[allow(dead_code)] // available for callers; not used internally
pub const STROOP_PRECISION: u8 = 7;

/// Maximum individual amount allowed per operation to prevent overflow
pub const MAX_SINGLE_AMOUNT_STROOPS: i128 = 1_000_000_0000000; // 1M tokens

/// Minimum positive amount (1 stroop)
pub const MIN_POSITIVE_AMOUNT: i128 = 1;

#[derive(Debug, PartialEq, Eq)]
pub enum AmountValidationError {
    NonPositiveAmount,
    AmountExceedsMaximum,
    ExceedsContractMaximum,
}

/// Validates a single amount for positivity and bounds
///
/// # Arguments
/// * `amount` - The amount to validate (in stroops)
///
/// # Returns
/// `Ok(())` if valid, `Err(AmountValidationError)` if invalid
pub fn validate_single_amount(amount: i128) -> Result<(), crate::EscrowError> {
    // Check positivity
    if amount <= MIN_POSITIVE_AMOUNT - 1 {
        return Err(crate::EscrowError::AmountMustBePositive);
    }

    // Check maximum bounds
    if amount > MAX_SINGLE_AMOUNT_STROOPS {
        // Map large amounts to generic invalid milestone amount
        return Err(crate::EscrowError::InvalidMilestoneAmount);
    }

    // Check stroop precision (must be integer, which i128 already guarantees)
    // In Stellar, stroop is the smallest unit, so any integer is valid
    // This check is more for documentation and future-proofing

    Ok(())
}

/// Validates an amount array/vector for positivity and bounds
///
/// # Arguments
/// * `amounts` - Slice of amounts to validate (in stroops)
///
/// # Returns
/// `Ok(total)` with sum of all amounts if valid, `Err(AmountValidationError)` if invalid
pub fn validate_amount_array(amounts: &[i128]) -> Result<i128, crate::EscrowError> {
    let mut total: i128 = 0;

    for &amount in amounts.iter() {
        // Validate individual amount
        validate_single_amount(amount)?;

        // Check for potential overflow in addition
        if let Some(new_total) = total.checked_add(amount) {
            total = new_total;
        } else {
            return Err(crate::EscrowError::PotentialOverflow);
        }
    }

    Ok(total)
}

/// Validates total amount against contract maximum
///
/// # Arguments
/// * `total_amount` - The total amount to validate
/// * `max_contract_total` - Maximum allowed per contract (in stroops)
///
/// # Returns
/// `Ok(())` if valid, `Err(AmountValidationError)` if invalid
pub fn validate_contract_total(
    total_amount: i128,
    max_contract_total: i128,
) -> Result<(), crate::EscrowError> {
    if total_amount > max_contract_total {
        // Map to InvalidMilestoneAmount for contract total overflow
        return Err(crate::EscrowError::InvalidMilestoneAmount);
    }
    Ok(())
}

/// Comprehensive validation for milestone amounts
///
/// # Arguments
/// * `milestone_amounts` - Array of milestone amounts (in stroops)
/// * `max_contract_total` - Maximum allowed per contract (in stroops)
///
/// # Returns
/// `Ok(total)` with sum of all milestones if valid, `Err(AmountValidationError)` if invalid
pub fn validate_milestone_amounts(
    milestone_amounts: &[i128],
    max_contract_total: i128,
) -> Result<i128, crate::EscrowError> {
    // Validate each milestone amount and calculate total
    let total = validate_amount_array(milestone_amounts)?;

    // Validate total against contract maximum
    validate_contract_total(total, max_contract_total)?;

    Ok(total)
}

/// Validates deposit amount against remaining contract capacity.
///
/// This is the canonical validation path for all deposit entrypoints. It rejects
/// any amount that is not strictly positive or that exceeds the distributed
/// single-milestone ceiling enforced across the escrow contract.
///
/// # Decision Boundaries
///
/// This function operates at three critical boundaries:
/// - **Exactly-remaining**: `deposit + current == max_total` ℒ Success
/// - **One stroop short**: `deposit + current == max_total - 1` → Success
/// - **One stroop over**: `deposit + current == max_total + 1` → Failure (`InvalidMilestoneAmount`)
///
/// # Arguments
/// * `deposit_amount` - Amount to deposit (in stroops, must be positive)
/// * `current_deposited` - Current total deposited amount (in stroops)
/// * `max_contract_total` - Maximum allowed per contract (in stroops)
///
/// # Returns
/// * `Ok(())` - Deposit is valid and won't exceed capacity
/// * `Err(EscrowError::AmountMustBePositive)` - Deposit amount is ≤ 0
/// * `Err(EscrowError::InvalidMilestoneAmount)` - Deposit would exceed capacity or single amount is too large
/// * `Err(EscrowError::PotentialOverflow)` - Adding deposit to current would overflow i128
///
/// # Security
///
/// - Uses checked arithmetic to prevent integer overflow panics
/// - Rejects any deposit when contract is already fully funded
/// - Validates deposit amount bounds before checking capacity
pub fn validate_deposit_amount(
    deposit_amount: i128,
    current_deposited: i128,
    max_contract_total: i128,
) -> Result<(), crate::EscrowError> {
    // Validate deposit amount itself
    validate_single_amount(deposit_amount)?;

    // Check if deposit would exceed contract maximum
    if let Some(new_total) = current_deposited.checked_add(deposit_amount) {
        if new_total > max_contract_total {
            return Err(crate::EscrowError::InvalidMilestoneAmount);
        }
    } else {
        return Err(crate::EscrowError::PotentialOverflow);
    }

    Ok(())
}

/// Utility function to safely add amounts with overflow protection
///
/// # Arguments
/// * `a` - First amount
/// * `b` - Second amount
///
/// # Returns
/// `Some(sum)` if addition succeeds, `None` if overflow would occur
pub fn safe_add_amounts(a: i128, b: i128) -> Option<i128> {
    a.checked_add(b)
}

/// Utility function to safely subtract amounts with underflow protection
///
/// # Arguments
/// * `a` - Minuend
/// * `b` - Subtrahend
///
/// # Returns
/// `Some(difference)` if subtraction succeeds, `None` if underflow would occur
pub fn safe_subtract_amounts(a: i128, b: i128) -> Option<i128> {
    a.checked_sub(b)
}

/// Safely accumulates amounts into a total with overflow protection.
///
/// Iterates through amounts, validating each amount for positivity and bounds,
/// and accumulating the total with checked arithmetic. Returns the total only if
/// all amounts are valid and no overflow occurs.
///
/// This function is intended for use in contexts like `deposit_funds` where an
/// unchecked `.sum()` could panic on overflow, creating a panicking code path
/// reachable by user-supplied milestone data.
///
/// # Arguments
/// * `amounts` - Iterator over amount references (typically milestone amounts)
///
/// # Returns
/// `Ok(total)` if all amounts are valid and accumulation succeeds, `Err(EscrowError)` if any validation fails
pub fn accumulate_amounts<I: IntoIterator<Item = i128>>(
    amounts: I,
) -> Result<i128, crate::EscrowError> {
    let mut total: i128 = 0;

    for amount in amounts.into_iter() {
        // Validate individual amount for positivity and bounds
        validate_single_amount(amount)?;

        // Check for potential overflow in accumulation
        if let Some(new_total) = total.checked_add(amount) {
            total = new_total;
        } else {
            return Err(crate::EscrowError::PotentialOverflow);
        }
    }

    Ok(total)
}

/// Records a deterministic failure observation for an amount-validation rejection.
///
/// This helper exists so that failure recovery is deterministic and observable:
/// every rejection path emits a stable, non-sensitive event containing only the
/// operation name and a code. No amounts, addresses, or user identifiers are
/// included, so the event is safe to log and match in tests.
///
/// # Invariants
/// - The emitted code is a deterministic function of the error variant.
/// - The function never panics and never mutates state.
/// - The operation name is a compile-time constant supplied by the caller.
pub fn failure_code(error: &crate::EscrowError) -> u32 {
    match error {
        crate::EscrowError::AmountMustBePositive => 1,
        crate::EscrowError::InvalidMilestoneAmount => 2,
        crate::EscrowError::PotentialOverflow => 3,
        _ => 0,
    }
}

/// Records a deterministic failure observation for an amount-validation rejection.
///
/// Wraps `failure_code` and the caller-supplied operation name into a single
/// deterministic event that can be consumed by logging or metrics layers. The
/// function is pure: it returns the event data and leaves emission to the caller,
/// which keeps this module stateless and testable.
///
/// # Returns
/// A `FailureObservation` containing the operation name and the deterministic
/// code for the given error.
pub fn observe_failure(operation: &str, error: &crate::EscrowError) -> FailureObservation {
    FailureObservation {
        operation: operation.to_string(),
        code: failure_code(error),
    }
}

/// Deterministic, non-sensitive description of an amount-validation failure.
///
/// This type is deliberately free of amounts, addresses, and any other user
/// data. It is safe to emit to logs or metrics and can be compared directly in
/// tests to assert failure-recovery behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureObservation {
    /// Stable operation name (e.g. `"deposit_funds"`).
    pub operation: String,
    /// Deterministic numeric code for the error variant.
    pub code: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_single_amount() {
        assert!(validate_single_amount(1).is_ok());
        assert!(validate_single_amount(100_0000000).is_ok());
        assert!(validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS).is_ok());

        assert_eq!(
            validate_single_amount(0),
            Err(crate::EscrowError::AmountMustBePositive)
        );
        assert_eq(
            validate_single_amount(-1),
            Err(crate::EscrowError::AmountMustBePositive)
        );
        assert_eq(
            validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS + 1),
            Err(crate::EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_validate_amount_array() {
        let amounts1 = [100_0000000, 200_0000000, 300_0000000];
        assert!(validate_amount_array(&amounts1).is_ok());
        assert_eq(validate_amount_array(&amounts1).unwrap(), 600_0000000);

        let amounts2 = [100_0000000, 0, 300_0000000];
        assert_eq(
            validate_amount_array(&amounts2),
            Err(crate::EscrowError::AmountMustBePositive)
        );

        let amounts3 = [100_0000000, -50_0000000, 300_0000000];
        assert_eq(
            validate_amount_array(&amounts3),
            Err(crate::EscrowError::AmountMustBePositive)
        );
    }

    #[test]
    fn test_validate_contract_total() {
        let max_total = 1_000_000_0000000;
        assert!(validate_contract_total(100_0000000, max_total).is_ok());
        assert!(validate_contract_total(max_total, max_total).is_ok());
        assert_eq(
            validate_contract_total(max_total + 1, max_total),
            Err(crate::EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_validate_milestone_amounts() {
        let max_contract_total = 1_000_000_0000000;
        let milestones1 = [100_0000000, 200_0000000, 300_0000000];
        assert!(validate_milestone_amounts(&milestones1, max_contract_total).is_ok());
        let milestones2 = [500_000_0000000, 600_000_0000000];
        assert_eq(
            validate_milestone_amounts(&milestones2, max_contract_total),
            Err(crate::EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_validate_deposit_amount() {
        struct TestCase {
            name: &'static str,
            deposit_amount: i128,
            current_deposited: i128,
            max_contract_total: i128,
            expected: Result<(), crate::EscrowError>,
        }

        let test_cases = [
            TestCase {
                name: "zero deposit amount should fail with AmountMustBePositive",
                deposit_amount: 0,
                current_deposited: 0,
                max_contract_total: 1000,
                expected: Err(crate::EscrowError::AmountMustBePositive),
            },
            TestCase {
                name: "negative deposit amount should fail with AmountMustBePositive",
                deposit_amount: -1,
                current_deposited: 0,
                max_contract_total: 1000,
                expected: Err(crate::EscrowError::AmountMustBePositive),
            },
            TestCase {
                name: "one stroop under remaining capacity should succeed",
                deposit_amount: 499,
                current_deposited: 500,
                max_contract_total: 1000,
                expected: Ok(()),
            },
            TestCase {
                name: "exactly remaining capacity should succeed",
                deposit_amount: 500,
                current_deposited: 500,
                max_contract_total: 1000,
                expected: Ok(()),
            },
            TestCase {
                name: "one stroop over remaining capacity should fail with InvalidMilestoneAmount",
                deposit_amount: 501,
                current_deposited: 500,
                max_contract_total: 1000,
                expected: Err(crate::EscrowError::InvalidMilestoneAmount),
            },
            TestCase {
                name: "already fully funded contract should reject any further deposit",
                deposit_amount: 1,
                current_deposited: 1000,
                max_contract_total: 1000,
                expected: Err(crate::EscrowError::InvalidMilestoneAmount),
            },
            TestCase {
                name: "deposit exactly filling remaining capacity should succeed",
                deposit_amount: 1000,
                current_deposited: 0,
                max_contract_total: 1000,
                expected: Ok(()),
            },
            TestCase {
                name: "deposit over single amount max should fail with InvalidMilestoneAmount",
                deposit_amount: MAX_SINGLE_AMOUNT_STROOPS + 1,
                current_deposited: 0,
                max_contract_total: i128::MAX,
                expected: Err(crate::EscrowError::InvalidMilestoneAmount),
            },
            TestCase {
                name: "adding deposit to current overflowing i128 should fail with PotentialOverflow",
                deposit_amount: 1,
                current_deposited: i128::MAX,
                max_contract_total: i128::MAX,
                expected: Err(crate::EscrowError::PotentialOverflow),
            },
        ];

        for tc in test_cases {
            assert_eq!(
                validate_deposit_amount(tc.deposit_amount, tc.current_deposited, tc.max_contract_total),
                tc.expected,
                "case failed: {}",
                tc.name
            );
        }
    }

    #[test]
    fn test_safe_add_amounts() {
        assert_eq(safe_add_amounts(1, 2), Some(3));
        assert_eq(safe_add_amounts(i128::MAX, 1), None);
        assert_eq(safe_add_amounts(i128::MIN, -1), None);
    }

    #[test]
    fn test_safe_subtract_amounts() {
        assert_eq(safe_subtract_amounts(3, 1), Some(2));
        assert_eq(safe_subtract_amounts(i128::MIN, 1), None);
    }

    #[test]
    fn test_accumulate_amounts() {
        let amounts = [100_0000000, 200_0000000, 300_0000000];
        assert_eq(accumulate_amounts(amounts), Ok(600_0000000));

        let invalid = [100_0000000, 0];
        assert_eq(
            accumulate_amounts(invalid),
            Err(crate::EscrowError::AmountMustBePositive)
        );

        // Overflow detection during accumulation.
        let overflowing = [i128::MAX - 1, 2];
        assert_eq(
            accumulate_amounts(overflowing),
            Err(crate::EscrowError::PotentialOverflow)
        );
    }

    #[test]
    fn test_failure_code_is_deterministic() {
        assert_eq(failure_code(&crate::EscrowError::AmountMustBePositive), 1);
        assert_eq(failure_code(&crate::EscrowError::InvalidMilestoneAmount), 2);
        assert_eq(failure_code(&crate::EscrowError::PotentialOverflow), 3);
        // Unknown variants map to 0 deterministically.
        assert_eq(failure_code(&crate::EscrowError::Unauthorized), 0);
    }

    #[test]
    fn test_observe_failure_is_non_sensitive() {
        let obs = observe_failure("deposit_funds", &crate::EscrowError::InvalidMilestoneAmount);
        assert_eq(
            obs,
            FailureObservation {
                operation: "deposit_funds".to_string(),
                code: 2,
            }
        );
        // The observation must not contain any amount or address data.
        assert!(!obs.operation.contains('1'));
    }

    #[test]
    fn test_recovery_is_deterministic_across_retries() {
        // Repeated validation of the same invalid input must produce the same
        // error and the same observation every time, ensuring retries cannot
        // change the outcome or leak different information.
        for _ in 0..10 {
            let result = validate_deposit_amount(0, 0, 1000);
            assert_eq(result, Err(crate::EscrowError::AmountMustBePositive));
            let obs = observe_failure("deposit_funds", &result.unwrap_err());
            assert_eq(obs.code, 1);
        }
    }

    #[test]
    fn test_partial_failure_does_not_corrupt_total() {
        // A failure in the middle of an array must not produce a partial total.
        // The result is an error, never a partially accumulated value.
        let amounts = [100_0000000, 0, 300_0000000];
        match validate_amount_array(&amounts) {
            Ok(_) => panic!("expected failure for partially valid array"),
            Err(e) => assert_eq!(e, crate::EscrowError::AmountMustBePositive),
        }
    }

    #[test]
    fn test_concurrent_repeated_deposits_cannot_exceed_cap() {
        // Simulate two independent deposits that together would exceed the cap.
        // Each individual deposit is valid against the current state, but the
        // second must be rejected once the first has been applied.
        let max = 1000;
        let mut current = 0;
        assert!(validate_deposit_amount(600, current, max).is_ok());
        current += 600;
        assert_eq!(
            validate_deposit_amount(600, current, max),
            Err(crate::EscrowError::InvalidMilestoneAmount)
        );
        // The cap is never exceeded and the current total is unchanged by the
        // rejected attempt.
        assert_eq(current, 600);
    }
}
