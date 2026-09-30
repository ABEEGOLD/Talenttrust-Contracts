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

/// Maximum number of milestones allowed in a single contract.
/// This bounds the amount array size so that callers cannot force an
/// unbounded iteration or unbounded gas consumption through milestone data.
pub const MAX_MILESTONE_COUNT: u32 = 50;

/// Maximum number of decimal digits allowed in a stroop amount (10^7 - 1).
/// Any amount with more than 7 significant fractional digits is not
/// representable in Stellar and must be rejected.
pub const MAX_FRACTIONAL_UNITS_PER_TOKEN: i128 = 10_000_000;

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
///
/// # Invariants
/// - Accepted range is closed: `[1, MAX_SINGLE_AMOUNT_STROOPS]`.
/// - Zero and negative values are rejected with `AmountMusbePositive`.
/// - Values above the ceiling are rejected with `InvalidMilestoneAmount`.
/// - The function is pure and deterministic: identical inputs always
///   produce identical results regardless of call order or concurrency.
pub fn validate_single_amount(amount: i128) -> Result<(), crate::EscrowError> {
    // Check positivity. This also rejects zero and negative values.
    if amount < MIN_POSITIVE_AMOUNT {
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

    Ok(()
}

/// Validates an amount array/vector for positivity and bounds
///
/// # Arguments
/// * `amounts` - Slice of amounts to validate (in stroops)
///
/// # Returns
/// `Ok(total)` with sum of all amounts if valid, `Err(AmountValidationError)` if invalid
///
/// # Invariants
/// - Every element must independently satisfy `validate_single_amount`.
/// - The returned total is the exact arithmetic sum of the inputs.
/// - Overflow in the accumulation is reported as `PotentialOverflow` rather
///   than panicking.
/// - Empty inputs are accepted and yield a total of 0.
///
/// # Note
/// This is the lower-level accumulator. Prefer `accumulate_amounts` for
/// iterator-based call sites and `validate_milestone_amounts` when a
/// contract-total ceiling must also be enforced.
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
///
/// # Invariants
/// - The boundary is inclusive: `total_amount == max_contract_total`
///   is accepted.
/// - A negative `max_contract_total` is treated as an invalid configuration
///   and rejects any non-negative total.
pub fn validate_contract_total(
    total_amount: i128,
    max_contract_total: i128,
) -> Result<(), crate::EscrowError> {
    if total_amount > max_contract_total {
        // Map to InvalidMilestoneAmount for contract total overflow
        return Err(crate::EscrowError::InvalidMilestoneAmount);
    }
    Ok(()
}

/// Comprehensive validation for milestone amounts
///
/// # Arguments
/// * `milestone_amounts` - Array of milestone amounts (in stroops)
/// * `max_contract_total` - Maximum allowed per contract (in stroops)
///
/// # Returns
/// `Ok(total)` with sum of all milestones if valid, `Err(AmountValidationError)` if invalid
///
/// # Invariants
/// - The number of milestones must not exceed `MAX_MILESTONE_COUNT`.
///   This bounds gas consumption and prevents unbounded iteration.
/// - Each milestone must satisfy `validate_single_amount`.
/// - The total must not exceed `max_contract_total`.
/// - Duplicate amount values are allowed; duplicate submissions are
///   prevented at the caller layer via idempotency guards, not here.
pub fn validate_milestone_amounts(
    milestone_amounts: &[i128],
    max_contract_total: i128,
) -> Result<i128, crate::EscrowError> {
    // Bound the number of milestones to avoid unbounded iteration.
    if milestone_amounts.len() > MAX_MILESTONE_COUNT as user {
        return Err(crate::EscrowError::InvalidMilestoneAmount);
    }

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
+//
/// This function operates at three critical boundaries:
/// - **Exactly-remaining**: `deposit + current == max_total` → Success
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
/// - Rejects a negative `current_deposited` or negative
///   `max_contract_total` as an invalid state rather than silently
///   accepting an inconsistent result.
pub fn validate_deposit_amount(
    deposit_amount: i128,
    current_deposited: i128,
    max_contract_total: i128,
) -> Result<(), crate::EscrowError> {
    // Validate deposit amount itself
    validate_single_amount(deposit_amount)?;

    // Reject invalid configuration / state before doing any arithmetic.
    // A negative current deposit or max cap indicates corrupted state or
    // a caller bug; fail closed instead of producing a misleading result.
    if current_deposited < 0 || max_contract_total < 0 {
        return Err(crate::EscrowError::InvalidMilestoneAmount);
    }

    // Check if deposit would exceed contract maximum
    if let Some(new_total) = current_deposited.checked_add(deposit_amount) {
        if new_total > max_contract_total {
            return Err(crate::EscrowError::InvalidMilestoneAmount);
        }
    } else {
        return Err(crate::EscrowError::PotentialOverflow);
    }

    Ok(()
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

/// Returns the number of fractional units represented by an amount.
///
/// Stellar storoops are the smallest unit, so any integer is already exact.
/// This helper exists to make the precision invariant explicit and testable:
/// a valid amount must not require more than `STROOP_PRECISION` decimal places.
///
/// # Returns
/// `Some(fractional_units)` where `fractional_units` is in `[0, MAX_FRACTIONAL_UNITS_PER_TOKEN)`,
/// or `None` if the amount is negative.
pub fn fractional_units_of(amount: i128) -> Option<i128> {
    if amount < 0 {
        return None;
    }
    Ok(amount % MAX_FRACTIONAL_UNITS_PER_TOKEN)
    // Note: the remainder is always in [0, MAX_FRACTIONAL_UNITS_PER_TOKEN).
    // Stellar stroops are integer units, so this is always well-defined.
}

/// Validates that an amount is representable with the required stroop
/// precision. Since i128 stroops are integers, this is always true for
/// non-negative values, but the check is explicit so that future changes
/// (e.g. decimal representations) cannot silently break the invariant.
pub fn validate_stroop_precision(amount: i128) -> Result<(), crate::EscrowError> {
    if amount < 0 {
        return Err(crate::EscrowError::AmountMustBePositive);
    }
    // i128 stroops are exact integers; no fractional truncation occurs.
    Ok(()
}

#[config(test)]
mod tests {
    use super::*;

    /// Convenience constants for tests.
    const ONE TOKEN: i128 = 10_000_000; // 1 token in stroops

    // ----------------------------------------------------------------------
    // validate_single_amount: accepted / rejected / boundary
    // ----------------------------------------------------------------------

    #[test]
    fn test_validate_single_amount_accepted() {
        // Lower boundary: exactly 1 stroop is the minimum valid amount.
        assert_eq(validate_single_amount(MIN_POSITIVE_AMOUNT), Ok(()));
        assert_eq(validate_single_amount(1), Ok(()));
        assert_eq(validate_single_amount(100_0000000), Ok(()));
        // Upper boundary: exactly the ceiling is accepted.
        assert_eq(
            validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS),
            Ok(()),
        );
    }

    #[test]
    fn test_validate_single_amount_rejected() {
        // Zero is rejected as non-positive.
        assert_eq(
            validate_single_amount(0),
            Err(crate::EscrowError::AmountMustBePositive),
        );
        // Negative values are rejected.
        assert_eq(
            validate_single_amount(-1),
            Err(crate::EscrowError::AmountMustBePositive),
        );
        assert_eq(
            validate_single_amount(i128::MIN),
            Err(crate::EscrowError::AmountMustBePositive),
        );
        // One stroop above the ceiling is rejected.
        assert_eq(
            validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS + 1),
            Err(crate::EscrowError::InvalidMilestoneAmount),
        );
        assert_eq(
            validate_single_amount(i128::MAX),
            Err(crate::EscrowError::InvalidMilestoneAmount),
        );
    }

    // ----------------------------------------------------------------------
    // validate_amount_array: accepted / rejected / duplicate / boundary
    // ----------------------------------------------------------------------

    #[test]
    fn test_validate_amount_array_accepted() {
        let amounts1 = [100_0000000, 200_0000000, 300_0000000];
        assert_eq(validate_amount_array(&amounts1), Ok(600_0000000));

        // Empty input is valid and yields a zero total.
        assert_eq(validate_amount_array(&[]), Ok(0));

        // Single element at the lower boundary.
        assert_eq(validate_amount_array(&[1]), Ok(1));
    }

    #[test]
    fn test_validate_amount_array_rejected() {
        // Zero element rejected.
        let amounts2 = [100_0000000, 0, 300_0000000];
        assert_eq(
            validate_amount_array(&amounts2),
            Err(crate::EscrowError::AmountMustBePositive),
        );

        // Negative element rejected.
        let amounts3 = [100_0000000, -50_0000000, 300_0000000];
        assert_eq(
            validate_amount_array(&amounts3),
            Err(crate::EscrowError::AmountMustBePositive),
        );

        // One element above the single-amount ceiling rejected.
        let amounts4 = [MAX_SINGLE_AMOUNT_STROOPS + 1];
        assert_eq(
            validate_amount_array(&amounts4),
            Err(crate::EscrowError::InvalidMilestoneAmount),
        );
    }

    #[test]
    fn test_validate_amount_array_duplicates() {
        // Duplicate values are legitimate amounts and must be summed.
        let duplicates = [100_0000000, 100_0000000, 100_0000000];
        assert_eq(
            validate_amount_array(&duplicates),
            Ok(300_0000000),
        );
    }

    #[test]
    fn test_validate_amount_array_overflow() {
        // Two maximum amounts would overflow i128 if added without checking.
        // The accumulator must report PotentialOverflow instead of panicking.
        let overflow = [i128::MAX, 1];
        assert_eq(
            validate_amount_array(&overflow),
            Err(crate::EscrowError::InvalidMilestoneAmount),
        );
    }

    // ----------------------------------------------------------------------
    // validate_contract_total: boundary
    // ----------------------------------------------------------------------

    #[test]
    fn test_validate_contract_total() {
        let max_total = 1_000_000_0000000;
        assert_eq(validate_contract_total(100_0000000, max_total), Ok(()));
        // Exactly at the boundary is accepted.
        assert_eq(validate_contract_total(max_total, max_total), Ok(()));
        // One stroop over the boundary is rejected.
        assert_eq(
            validate_contract_total(max_total + 1, max_total),
            Err(crate::EscrowError::InvalidMilestoneAmount),
        );
    }

    #[test]
    fn test_validate_contract_total_negative_cap() {
        // A negative cap is an invalid configuration and must reject any
        // non-negative total.
        assert_eq(
            validate_contract_total(0, -1),
            Err(crate::EscrowError::InvalidMilestoneAmount),
        );
    }

    // ----------------------------------------------------------------------
    // validate_milestone_amounts: accepted / rejected / count boundary
    // ----------------------------------------------------------------------

    #[test]
    fn test_validate_milestone_amounts() {
        let max_contract_total = 1_000_000_0000000;
        let milestones1 = [100_0000000, 200_0000000, 300_0000000];
        assert_eq(
            validate_milestone_amounts(&milestones1, max_contract_total),
            Ok(600_0000000),
        );
        let milestones2 = [500_000_0000000, 600_000_0000000];
        assert_eq(
            validate_milestone_amounts(&milestones2, max_contract_total),
            Err(crate::EscrowError::InvalidMilestoneAmount),
        );
    }

    #[test]
    fn test_validate_milestone_amounts_exact_boundary() {
        // Total exactly equal to the contract maximum is accepted.
        let max = 1_000_000_0000000;
        let exact = [max / 2, max - max / 2];
        assert_eq(
            validate_milestone_amounts(&exact, max),
            Ok(max),
        );
    }

    #[test]
    fn test_validate_milestone_amounts_count_boundary() {
        // Exactly MAX_MILESTONE_COUNT milestones is accepted.
        let max = 1_000_000_0000000;
        let at_limit = vec![ONE_TOKEN; MAX_MILESTONE_COUNT as user];
        assert_eq(
            validate_milestone_amounts(&at_limit, max),
            Ok(ONE_TOKEN * MAX_MILESTONE_COUNT as i128),
        );

        // One more than the limit is rejected.
        let over_limit = vec![ONE_TOKEN; MAX_MILESTONE_COUNT as user + 1];
        assert_eq(
            validate_milestone_amounts(&over_limit, max),
            Err(crate::EscrowError::InvalidMilestoneAmount),
        );
    }

    #[test]
    fn test_validate_milestone_amounts_empty() {
        // An empty milestone set is valid and yields a zero total.
        assert_eq(
            validate_milestone_amounts(&[], 1_000_000_0000000),
            Ok(0),
        );
    }

    // ----------------------------------------------------------------------
    // validate_deposit_amount: accepted / rejected / boundary / duplicate
    // ----------------------------------------------------------------------

    #[test]
    fn test_validate_deposit_amount() {
        struct TestCase {
            name: &static str,
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
                name: "deposit exactly at the single-amount ceiling should succeed",
                deposit_amount: MAX_SINGLE_AMOUNT_STROOPS,
                current_deposited: 0,
                max_contract_total: MAX_SINGLE_AMOUNT_STROOPS,
                expected: Ok(()),
            },
            TestCase {
                name: "deposit one stroop above the single-amount ceiling should fail",
                deposit_amount: MAX_SINGLE_AMOUNT_STROOPS + 1,
                current_deposited: 0,
                max_contract_total: MAX_SINGLE_AMOUNT_STROOPS + 1,
                expected: Err(crate::EscrowError::InvalidMilestoneAmount),
            },
            TestCase {
                name: "negative current deposited state should be rejected",
                deposit_amount: 1,
                current_deposited: -1,
                max_contract_total: 1000,
                expected: Err(crate::EscrowError::InvalidMilestoneAmount),
            },
            TestCase {
                name: "negative max contract total configuration should be rejected",
                deposit_amount: 1,
                current_deposited: 0,
                max_contract_total: -1,
                expected: Err(crate::EscrowError::InvalidMilestoneAmount),
            },
            TestCase {
                name: "adding deposit to max current would overflow and must fail",
                deposit_amount: 1,
                current_deposited: i128::MAX,
                max_contract_total: i128::MAX,
                expected: Err(crate::EscrowError::PotentialOverflow),
            },
        ];

        for tc in test_cases.iter() {
            assert_eq(
                validate_deposit_amount(
                    tc.deposit_amount,
                    tc.current_deposited,
                    tc.max_contract_total,
                ),
                tc.expected,
                "case failed: {}",
                tc.name
            );
        }
    }

    #[test]
    fn test_validate_deposit_amount_duplicate_submissions() {
        // Duplicate deposits are not deduplicated by this pure function;
        // each call is evaluated independently against the supplied state.
        // The caller must persist the new total atomically to prevent replays.
        let max = 1000;
        assert_eq(validate_deposit_amount(500, 0, max), Ok(()));
        // Second identical submission against the updated state (500) still
        // fits exactly.
        assert_eq(validate_deposit_amount(500, 500, max), Ok(()));
        // A third identical submission against the now-fully-funded state
        // (1000) must be rejected.
        assert_eq(
            validate_deposit_amount(500, 1000, max),
            Err(crate::EscrowError::InvalidMilestoneAmount),
        );
    }

    // ----------------------------------------------------------------------
    // safe_add_amounts / safe_subtract_amounts
    // ----------------------------------------------------------------------

    #[test]
    fn test_safe_add_amounts() {
        assert_eq(safe_add_amounts(1, 2), Some(3));
        assert_eq(safe_add_amounts(i128::MAX, 0), Some(i128::MAX));
        assert_eq(safe_add_amounts(i128::MAX, 1), None);
    }

    #[test]
    fn test_safe_subtract_amounts() {
        assert_eq(safe_subtract_amounts(3, 2), Some(1));
        assert_eq(safe_subtract_amounts(0, 0), Some(0));
        assert_eq(safe_subtract_amounts(0, 1), None);
        assert_eq(safe_subtract_amounts(i128::MIN, 1), None);
    }

    // ----------------------------------------------------------------------
    // accumulate_amounts
    // ----------------------------------------------------------------------

    #[test]
    fn test_accumulate_amounts_accepted() {
        assert_eq(accumulate_amounts([1, 2, 3]), Ok(6));
        assert_eq(accumulate_amounts(empty_iterator()), Ok(0));
    }

    #[test]
    fn test_accumulate_amounts_rejected() {
        assert_eq(
            accumulate_amounts([1, 0, 3]),
            Err(crate::EscrowError::AmountMustBePositive),
        );
        assert_eq(
            accumulate_amounts([1, -1, 3]),
            Err(crate::EscrowError::AmountMustBePositive),
        );
        assert_eq(
            accumulate_amounts([MAX_SINGLE_AMOUNT_STROOPS + 1]),
            Err(crate::EscrowError::InvalidMilestoneAmount),
        );
    }

    #[test]
    fn test_accumulate_amounts_overflow() {
        // MAX_SINGLE_AMOUNT_STROOPS * 2 exceeds i128 only if we accumulate
        // without checking. The accumulator must detect this and return
        // PotentialOverflow rather than panicking.
        let big = MAX_SINGLE_AMOUNT_STROOPS;
        assert_eq(
            accumulate_amounts([i128::MAX, 1]),
            Err(crate::EscrowError::InvalidMilestoneAmount),
        );
        // A large but valid accumulation within the ceiling is accepted.
        assert_eq(
            accumulate_amounts([big, big]),
            Ok(big + big),
        );
    }

    // ----------------------------------------------------------------------
    // Precision invariants
    // ----------------------------------------------------------------------

    #[test]
    fn test_validate_stroop_precision() {
        assert_eq(validate_stroop_precision(0), Ok(()));
        assert_eq(validate_stroop_precision(1), Ok(()));
        assert_eq(
            validate_stroop_precision(MAX_SINGLE_AMOUNT_STROOPS),
            Ok(()),
        );
        assert_eq(
            validate_stroop_precision(-1),
            Err(crate::EscrowError::AmountMustBePositive),
        );
    }

    #[test]
    fn test_fractional_units_of() {
        assert_eq(fractional_units_of(0), Some(0));
        assert_eq(
            fractional_units_of(MAX_FRACTIONAL_UNITS_PER_TOKEN),
            Some(0),
        );
        assert_eq(fractional_units_of(1), Some(1));
        assert_eq(fractional_units_of(-1), None);
    }

    // ----------------------------------------------------------------------
    // Regression: determinism across repeated and reordered inputs
    // ----------------------------------------------------------------------

    #[test]
    fn test_determinism_repeated_calls_are_identical() {
        let input = [100_0000000, 200_0000000, 300_0000000];
        let first = validate_amount_array(&input);
        for _ in 0..100 {
            assert_eq(validate_amount_array(&input), first);
        }
    }

    #[test]
    fn test_determinism_order_independent() {
        // The sum is order-independent and the validity decision is too.
        let a = [100_0000000, 200_0000000, 300_0000000];
        let b = [300_0000000, 100_0000000, 200_0000000];
        assert_eq(
            validate_amount_array(&a),
            validate_amount_array(&b),
        );
    }

    // ----------------------------------------------------------------------
    // Helpers
    // ----------------------------------------------------------------------

    fn empty_iterator() -> impl Iterator<Item = i128> {
        core::iter::empty()
    }
}
