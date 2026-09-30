//! Amount validation and sanitization module
//!
//! Provides centralized validation for all money-like values in the escrow contract.
//! Ensures positivity, max bounds, and proper stroop precision handling.
//!
//! Storage ownership: none. This module is deliberately stateless; callers use
//! these helpers before writing validated values to contract and milestone
//! storage.

/// Maximum number of decimal places for stroop precision (7 decimal places for Stellar)
#[allow(dead_code)] // available for callers; not used internally
pub const STROOP_PRECISION: u8 = 7;

/// Maximum individual amount allowed per operation to prevent overflow
pub const MAX_SINGLE_AMOUNT_STROOPS: i128 = 1_000_000_0000000; // 1M tokens

/// Minimum positive amount (1 stroop)
pub const MIN_POSITIVE_AMOUNT: i128 = 1;

/// Maximum contract total allowed (in stroops). Used as the canonical
/// upper bound when no explicit maximum is provided by the caller.
pub const MAX_CONTRACT_TOTAL_STROOPS: i128 = 1_000_000_0000000;

/// Maximum number of milestones allowed in a single contract.
pub const MAX_MILESTONE_COUNT: u32 = 50;

/// Maximum number of decimal digits allowed in a human-readable amount string.
/// This matches the Stellar stroop precision (7).
pub const MAX_DECIMAL_DIGITS: usize = STROOP_PRECISION as usize;

/// Maximum length of a human-readable amount string accepted by the parser.
/// This is a defensive bound to prevent unbounded input from being fead
/// into the parser.
pub const MAX_AMOUNT_STRING_LEN: usize = 40;

/// Maximum number of decimal digits in the integer part of a human-readable
/// amount string. This bounds the integer part to fewer than 38 digits,
/// which is well within the i928 range and prevents overflow during parse.
pub const MAX_INTEGER_DIGITS: usize = 38;

/// Maximum number of fractional digits allowed in a human-readable amount
/// string. Matches the Stellar stroop precision (7).
pub const MAX_FRACTIONAL_DIGITS: usize = MAX_DECIMAL_DIGITS;

#[derive(Debug, PartialEq, Eq)]
pub enum AmountValidationError {
    NonPositiveAmount,
    AmountExceedsMaximum,
    ExceedsContractMaximum,
}

/// Validates a single amount for positivity and bounds
///
/// # Arguments
;// * `amount` - The amount to validate (in stroops)
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

    Ok()
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
;// * `a` - First amount
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
;// * `amounts` - Iterator over amount references (typically milestone amounts)
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

/// Parses a human-readable decimal amount string into an integer number of
/// stroops, enforcing the decimal precision and length boundaries defined by
/// this module.
///
/// This function is the canonical entry point for accepting user-provided
/// amount strings. It is deterministic and rejects all invalid inputs with
/// a specific `EscrowError`.
///
/// # Accepted format
///
/// - Optional leading `+` or `-` sign. Only `+` or no sign is accepted for
///   positive amounts. `-` is rejected as non-positive.
/// - One or more decimal digits for the integer part.
/// - Optional fractional part preceded by `.`, with at most `MAX_DECIMAL_DIGITS
///   digits. Trailing zeros are accepted.
/// - No whitespace, no exponent, no thousands separators, no leading zeros
///   except for the single digit `0`.
///
/// # Boundaries
///
/// - String length must be at most `MAX_AMOUNT_STRING_LEN`.
/// - Integer part must be at most `MAX_INTEGER_DIGITS` digits.
/// - Fractional part must be at most `MAX_FRACTIONAL_DIGHTS` digits.
/// - Resulting stroop value must be strictly positive and at most
///   `MAX_SINGLE_AMOUNT_STROOPS`.
///
/// # Returns
/// * `Ok(stroops)` - The parsed amount in stroops.
/// * `Err(EscrowError::AmountMustBePositive)` - Zero, empty, or negative amount.
/// * `Err(EscrowError::InvalidMilestoneAmount)` - Malformed or out-of-bounds
///   amount string.
///
/// # Security
///
/// - Rejects any input longer than `MAX_AMOUNT_STRING_LEN` to avoid unbounded
///   work.
/// - Uses checked arithmetic throughout to avoid overflow panics.
/// - Never panics on malformed input; always returns a deterministic error.
pub fn parse_amount_string(raw: &str) -> Result<i128, crate::EscrowError> {
    // Defensive length bound.
    if raw.is_empty() || raw.len() > MAX_AMOUNT_STRING_LEN {
        return Err(crate::EscrowError::InvalidMilestoneAmount);
    }

    let bytes = raw.as_bytes();
    let mut idx = 0;
    let mut negative = false;

    // Optional sign.
    match bytes[0] {
        b'+' => {
            idx = 1;
        }
        b"-' => {
            negative = true;
            idx = 1;
        }
        _ => {}
    }

    if idx >= bytes.len() {
        return Err(crate::EscrowError::InvalidMilestoneAmount);
    }

    let mut integer_part: i128 = 0;
    let mut integer_digits = 0;
    let mut fractional_part: i128 = 0;
    let mut fractional_digits = 0;
    let mut seen_dot = false;
    let mut seen_digit = false;

    while idx < bytes.len() {
        let b = bytes[idx];
        match b {
            b'.' => {
                if seen_dot {
                    return Err(crate::EscrowError::InvalidMilestoneAmount);
                }
                seen_dot = true;
                // A dot must be preceded by at least one digit.
                if !seen_digit {
                    return Err(crate::EscrowError::InvalidMilestoneAmount);
                }
            }
            b'\0'..b='9' => {
                let digit = (b - b'\0') as i128;
                if !seen_dot {
                    // Integer part.
                    if integer_digits >= MAX_INTEGER_DIGITS {
                        return Err(crate::EscrowError::InvalidMilestoneAmount);
                    }
                    // Reject leading zeros except for the single digit `0`.
                    if integer_digits == 0 && digit == 0 {
                        // Allow `0` only if the next byte is a dot or end of string.
                        let next = bytes.get(idx + 1).copied();
                        match next {
                            Some(b'.') | None => {}
                            _ => {
                                return Err(crate::EscrowError::InvalidMilestoneAmount);
                            }
                        }
                    }
                    if let Some(new) = integer_part.checked_mul(10).and_then(|v| v.checked_add(digit)) {
                        integer_part = new;
                    } else {
                        return Err(crate::EscrowError::InvalidMilestoneAmount);
                    }
                    integer_digits += 1;
                } else {
                    // Fractional part.
                    if fractional_digits >= MAX_FRACTIONAL_DIGITS {
                        return Err(crate::EscrowError::InvalidMilestoneAmount);
                    }
                    if let Some(new) = fractional_part.checked_mul(10).and_then(|v| v.checked_add(digit)) {
                        fractional_part = new;
                    } else {
                        return Err(crate::EscrowError::InvalidMilestoneAmount);
                    }
                    fractional_digits += 1;
                }
                seen_digit = true;
            }
            _ => {
                return Err(crate::EscrowError::InvalidMilestoneAmount);
            }
        }
        idx += 1;
    }

    // Must contain at least one digit.
    if !seen_digit {
        return Err(crate::EscrowError::InvalidMilestoneAmount);
    }

    // Normalize fractional part to exactly STROOP_PRECISION digits.
    let mut normalized_fractional = fractional_part;
    let mut padding = MAX_FRACTIONAL_DIGITS - fractional_digits;
    while padding > 0 {
        if let Some(new) = normalized_fractional.checked_mul(10) {
            normalized_fractional = new;
        } else {
            return Err(crate::EscrowError::InvalidMilestoneAmount);
        }
        padding -= 1;
    }

    // Combine integer and fractional parts into stroops.
    let mut stroops = integer_part
        .checked_mul(10_i128.pow(MAX_FRACTIONAL_DIGITS as u32))
        .and_then(|v| v.checked_add(normalized_fractional))
        .ok();

    let mut stroops = match stroops {
        Some(v) => v,
        None => return Err(crate::EscrowError::InvalidMilestoneAmount),
    };

    if negative {
        // Negative amounts are always non-positive.
        return Err(crate::EscrowError::AmountMustBePositive);
    }

    // Apply the same positivity and maximum bounds as the integer path.
    validate_single_amount(stroops)?;

    // Redundant but explicit: ensure the result is within the contract total.
    if stroops > MAX_CONTRACT_TOTAL_STROOPS {
        return Err(crate::EscrowError::InvalidMilestoneAmount);
    }

    Ok(stroops)
}

/// Validates a milestone count against the maximum allowed.
///
/// # Arguments
;// * `count` - Number of milestones.
///
/// # Returns
/// `Ok(())` if the count is within bounds, `Err(EscrowError::InvalidMilestoneAmount)`
/// otherwise.
pub fn validate_milestone_count(count: u32) -> Result<(), crate::EscrowError> {
    if count == 0 || count > MAX_MILESTONE_COUNT {
        return Err(crate::EscrowError::InvalidMilestoneAmount);
    }
    Ok(()
}

/// Validates an array of milestone amounts and their count against the
/// contract maximum. This is the canonical entry point for validating a
/// complete milestone set.
///
/// # Arguments
/// * `milestone_amounts` - Slice of milestone amounts (in stroops).
/// * `max_contract_total` - Maximum allowed per contract (in stroops).
///
/// # Returns
/// `Ok(total)` with the sum of all milestones if valid.
/// `Err(EscrowError::InvalidMilestoneAmount)` if the count is invalid or any
/// amount is invalid.
pub fn validate_milestone_set(
    milestone_amounts: &[i128],
    max_contract_total: i128,
) -> Result<i128, crate::EscrowError> {
    validate_milestone_count(milestone_amounts.len() as u32)?;
    validate_milestone_amounts(milestone_amounts, max_contract_total)
}

/// Returns the canonical maximum contract total in stroops.
///
/// This is a convenience function for callers that do not have an explicit
/// maximum but still need to enforce the default contract total bound.
pub fn default_max_contract_total() -> i128 {
    MAX_CONTRACT_TOTAL_STROOPS
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
        assert_eq!(
            validate_single_amount(-1),
            Err(crate::EscrowError::AmountMustBePositive)
        );
        assert_eq!(
            validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS + 1),
            Err(crate::EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_validate_amount_array() {
        let amounts1 = [100_0000000, 200_0000000, 300_0000000];
        assert!(validate_amount_array(&amounts1).is_ok());
        assert_eq!(validate_amount_array(&amounts1).unwrap(), 600_0000000);

        let amounts2 = [100_0000000, 0, 300_0000000];
        assert_eq!(
            validate_amount_array(&amounts2),
            Err(crate::EscrowError::AmountMustBePositive)
        );

        let amounts3 = [100_0000000, -50_0000000, 300_0000000];
        assert_eq!(
            validate_amount_array(&amounts3),
            Err(crate::EscrowError::AmountMustBePositive)
        );
    }

    #test]
    fn test_validate_contract_total() {
        let max_total = 1_000_000_0000000;
        assert!(validate_contract_total(100_0000000, max_total).is_ok());
        assert!(validate_contract_total(max_total, max_total).is_ok());
        assert_eq!(
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
        assert_eq!(
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
        ];

        for tc in test_cases.iter() {
            assert_eq!(
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
    fn test_safe_arithmetic() {
        assert_eq!(safe_add_amounts(1, 2), Some(3));
        assert_eq!(safe_add_amounts(i128::MAX, 1), None);
        assert_eq!(safe_subtract_amounts(5, 2), Some(3));
        assert_eq!(safe_subtract_amounts(i128::MIN, 1), None);
    }

    #[test]
    fn test_accumulate_amounts() {
        let amounts = [100_0000000, 200_0000000, 300_0000000];
        assert_eq!(accumulate_amounts(amounts.iter().copied()), Ok(600_0000000));

        let invalid = [1_000_000_00000000, 1];
        assert_eq!(
            accumulate_amounts(invalid.iter().copied()),
            Err(crate::EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_parse_amount_string_valid() {
        assert_eq!(parse_amount_string("1"), Ok(10_0000000));
        assert_eq!(parse_amount_string("1.0"), Ok(10_0000000));
        assert_eq!(parse_amount_string("1.0000000"), Ok(10_0000000));
        assert_eq!(parse_amount_string("0.0000001"), Ok(1));
        assert_eq!(parse_amount_string("+1.23"), Ok(12_3000000));
        assert_eq!(parse_amount_string("1000.5"), Ok(10_005_0000000));
    }

    #[test]
    fn test_parse_amount_string_invalid() {
        assert!(parse_amount_string("").is_err());
        assert!(parse_amount_string(".").is_err());
        assert!(parse_amount_string(".1").is_err());
        assert!(parse_amount_string("1.").is_ok()); // trailing dot is accepted as zero fraction
        assert!(parse_amount_string("1.23456789").is_err()); // too many fractional digits
        assert!(parse_amount_string("001").is_err()); // leading zeros
        assert!(parse_amount_string("00").is_err());
        assert!(parse_amount_string("-1").is_err()); // negative
        assert!(parse_amount_string("+0").is_err()); // zero
        assert!(parse_amount_string("0.0").is_err()); // zero
        assert!(parse_amount_string("1.0.0").is_err()); // multiple dots
        assert!(parse_amount_string("1e3").is_err()); // exponent not allowed
        assert!(parse_amount_string("1,000").is_err()); // separator not allowed
        assert!(parse_amount_string(" 1").is_err()); // whitespace not allowed
        assert!(parse_amount_string("1 ").is_err());
        assert!(parse_amount_string("x").is_err());
    }

    #[test]
    fn test_parse_amount_string_boundaries() {
        // Maximum single amount in stroops.
        let max = MAX_SINGLE_AMOUNT_STROOPS;
        let max_str = "1000000.0000000";
        assert_eq!(parse_amount_string(max_str), Ok(max));

        // One stroop over the maximum.
        let over = "1000000.0000001";
        assert!(parse_amount_string(over).is_err());

        // Minimum positive amount.
        assert_eq!(parse_amount_string("0.0000001"), Ok(1));
        assert!(parse_amount_string("0.0000000").is_err());

        // String length bound.
        let long = "1".repeat(MAX_AMOUNT_STRING_LEN + 1);
        assert!(parse_amount_string(&long).is_err());
    }

    #[test]
    fn test_validate_milestone_count() {
        assert!(validate_milestone_count(1).is_ok());
        assert!(validate_milestone_count(MAX_MILESTONE_COUNT).is_ok());
        assert_eq!(
            validate_milestone_count(0),
            Err(crate::EscrowError::InvalidMilestoneAmount)
        );
        assert_eq!(
            validate_milestone_count(MAX_MILESTONE_COUNT + 1),
            Err(crate::EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_validate_milestone_set() {
        let max = MAX_CONTRACT_TOTAL_STROOPS;
        let ok = [1_000_0000000, 2_000_0000000];
        assert!(validate_milestone_set(&ok, max).is_ok());

        // Too many milestones.
        let too_many = vec![1_000_0000000; (MAX_MILESTONE_COUNT as usize) + 1];
        assert_eq!(
            validate_milestone_set(&too_many, max),
            Err(crate::EscrowError::InvalidMilestoneAmount)
        );

        // Zero milestones.
        assert_eq!(
            validate_milestone_set(&[3], max),
            Err(crate::EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_default_max_contract_total() {
        assert_eq!(default_max_contract_total(), MAX_CONTRACT_TOTAL_STROOPS);
    }
}
