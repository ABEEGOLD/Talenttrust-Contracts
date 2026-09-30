/// Simple standalone test for amount validation functionality
///
/// This test verifies that the amount validation implementation works correctly
/// without depending on the complex existing test infrastructure.
///
/// ## Validation Boundaries

/// This module defines and enforces the valid, invalid, duplicate, and
/// boundary-case input handling for the amount validation surface.
///
/// ### Invariants enforced by this suite
///
/// 1. **Positivity**: every accepted amount is strictly greater than zero.
///    The lowest accepted value is `MIN_POSITIVE_AMOUNT` (1 stroop).
/// 2. **Single-amount ceiling**: no individual amount may exceed
///    `MAX_SINGLE_AMOUNT_STROOPS`. The exact ceiling is accepted; one
///    stroop above is rejected.
/// 3. **Contract total ceiling**: the sum of all milestones must not
///    exceed `max_contract_total`. The exact ceiling is accepted; one
///    stroop above is rejected.
/// 4. **Deposit capacity**: `current_deposited + deposit_amount` must
///    not exceed `max_contract_total`. The exact remaining capacity is
///    accepted; one stroop over is rejected.
/// 5. **Overflow safety**: all arithmetic uses checked operations;
///    overflow yields `EscrowError::PotentialOverflow` rather than a
///    panic.
/// 6. **Determinism**: identical inputs always produce identical
///    results; validation is pure and stateless.
/// 7. **Duplicate submissions**: repeated validation of the same
///    input is idempotent and never mutates state.
///
/// ### Failure modes
///
/// - Zero or negative amounts return `AmountMustBePositive`.
/// - Amounts above the single-ceiling or total ceiling return
///   `InvalidMilestoneAmount`.
/// - Arithmetic overflow returns `PotentialOverflow`.
/// - No error path panics or silently truncates a value.

/// ### Observability
///
/// Every rejection carries a distinct `EscrowError` variant so that
/// callers and off-chain monitoring can diagnose the failure without
/// exposing sensitive data. The validators never log or return the
/// raw amount in an error payload.

#[config(test)]mod tests {
    use crate::amount_validation::{
        accumulate_amounts, safe_add_amounts, safe_subtract_amounts,
        validate_amount_array, validate_contract_total, validate_deposit_amount,
        validate_milestone_amounts, validate_single_amount, EscrowError,
        MAX_SINGLE_AMOUNT_STROOPS, MIN_POSITIVE_AMOUNT, STROOP_PRECISION,
    };
    use crate::MAX_TOTAL_ESCROW_STROOPS;

    // -------------------------------------------------------------------
    // Boundary constants used throughout the suite. These are derived from
    // the production constants so the tests fail if the boundaries drift.
    // -------------------------------------------------------------------
    const MAX_SINGLE: i128 = MAX_SINGLE_AMOUNT_STROOPS;
    const MIN_POSITIVE: i128 = MIN_POSITIVE_AMOUNT;
    const MAX_TOTAL: i128 = MAX_TOTAL_ESCROW_STROOPS;

    // -------------------------------------------------------------------
    // Accepted input
    // -------------------------------------------------------------------

    #[test]
    fn test_validate_single_amount_works() {
        // Test valid amounts
        assert!(validate_single_amount(1).is_ok());
        assert!(validate_single_amount(100_0000000).is_ok()); // 1 token
        assert!(validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS).is_ok());

        // Test invalid amounts
        assert_eq!(
            validate_single_amount(0),
            Error::AmountMustBePositive
        );
        assert_eq!(
            validate_single_amount(-1),
            Error::AmountMustBePositive
        );
        assert_eq!(
            validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS + 1),
            Error::InvalidMilestoneAmount
        );
    }

    // -------------------------------------------------------------------
    // Boundary cases: exactly at the ceiling and one stroop above
    // -------------------------------------------------------------------

    #[test]
    fn test_single_amount_exact_ceiling_is_accepted() {
        // The exact ceiling must be accepted.
        assert!(validate_single_amount(MAX_SINGLE).is_ok());
        // One stroop above the ceiling must be rejected.
        assert_eq!(
            validate_single_amount(MAX_SINGLE + 1),
            Error::InvalidMilestoneAmount
        );
    }

    #[test]
    fn test_single_amount_exact_floor_is_accepted() {
        // The lowest positive value is accepted.
        assert!(validate_single_amount(MIN_POSITIVE).is_ok());
        // One stroop below the floor is rejected as non-positive.
        assert_eq!(
            validate_single_amount(MIN_POSITIVE - 1),
            Error::AmountMustBePositive
        );
    }

    #[test]
    fn test_single_amount_extreme_i128_boundaries() {
        // i128::MIN_POSITIVE - 1 is the most negative value and must be
        // rejected without panicking.
        assert_eq!(
            validate_single_amount(i128::MIN),
            Error::AmountMustBePositive
        );
        // i128::MAX is far above the ceiling and must be rejected.
        assert_eq!(
            validate_single_amount(i128::MAX),
            Error::InvalidMilestoneAmount
        );
    }

    #[test]
    fn test_validate_milestone_amounts_works() {
        // Test valid milestone arrays
        let milestones1 = [100_0000000, 200_0000000, 300_0000000];
        assert!(validate_milestone_amounts(&milestones1, MAX_TOTAL_ESCROW_STROOPS).is_ok());
        assert_eq!(
            validate_milestone_amounts(&milestones1, MAX_TOTAL_ESCROW_STROOPS).unwrap(),
            600_0000000
        );

        // Test single milestone at maximum
        let milestones2 = [MAX_TOTAL_ESCROW_STROOPS];
        assert!(validate_milestone_amounts(&milestones2, MAX_TOTAL_ESCROW_STROOPS).is_ok());

        // Test invalid arrays
        let milestones3 = [100_0000000, 0, 300_0000000]; // Contains zero
        assert_eq!(
            validate_milestone_amounts(&milestones3, MAX_TOTAL_ESCROW_STROOPS),
            Error::AmountMustBePositive
        );

        let milestones4 = [100_0000000, -50_0000000, 300_0000000]; // Contains negative
        assert_eq!(
            validate_milestone_amounts(&milestones4, MAX_TOTAL_ESCROW_STROOPS),
            Error::AmountMustBePositive
        );

        let milestones5 = [600_000_0000000, 500_000_0000000]; // Exceeds contract max
        assert_eq!(
            validate_milestone_amounts(&milestones5, MAX_TOTAL_ESCROW_STROOPS),
            Error::InvalidMilestoneAmount
        );
    }

    // -------------------------------------------------------------------
    // Milestone total boundaries: exactly at the ceiling and one stroop above
    // -------------------------------------------------------------------

    #[test]
    fn test_milestone_total_exactly_at_ceiling_is_accepted() {
        // Split the ceiling across two milestones that exactly sum to the
        // contract maximum.
        let half = MAX_TOTAL / 2;
        let remainder = MAX_TOTAL - half;
        let milestones = [half, remainder];
        assert_eq!(
            validate_milestone_amounts(&milestones, MAX_TOTAL).unwrap(),
            MAX_TOTAL
        );
    }

    #[test]
    fn test_milestone_total_one_stroop_over_ceiling_is_rejected() {
        // One stroop over the contract ceiling must be rejected even when
        // each individual milestone is within the single-amount ceiling.
        let half = MAX_TOTAL / 2;
        let milestones = [half, MAX_TOTAL - half + 1];
        assert_eq!(
            validate_milestone_amounts(&milestones, MAX_TOTAL),
            Error::InvalidMilestoneAmount
        );
    }

    #[test]
    fn test_empty_milestone_array_is_accepted() {
        // An empty array sums to zero and is within bounds. This documents
        // the default behavior of the validator for degenerate input.
        let empty: [i128; 0] = [];
        assert_eq!validate_milestone_amounts(&empty, MAX_TOTAL).unwrap(), 0);
    }

    #[test]
    fn test_validate_deposit_amount_works() {
        // Test valid deposits
        assert!(validate_deposit_amount(100_0000000, 0, MAX_TOTAL_ESCROW_STROOPS).is_ok());
        assert!(
            validate_deposit_amount(100_0000000, 500_0000000, MAX_TOTAL_ESCROW_STROOPS).is_ok()
        );
        assert!(
            validate_deposit_amount(MAX_TOTAL_ESCROW_STROOPS, 0, MAX_TOTAL_ESCROW_STROOPS).is_ok()
        );

        // Test invalid deposits
        assert_eq!(
            validate_deposit_amount(0, 0, MAX_TOTAL_ESCROW_STROOPS),
            Error::AmountMustBePositive
        );
        assert_eq!(
            validate_deposit_amount(-1, 0, MAX_TOTAL_ESCROW_STROOPS),
            Error::AmountMustBePositive
        );

        // Test would exceed maximum
        assert_eq!(
            validate_deposit_amount(600_000_0000000, 500_000_0000000, MAX_TOTAL_ESCROW_STROOPS),
            Error::InvalidMilestoneAmount
        );
    }

    // -------------------------------------------------------------------
    // Deposit capacity boundaries: exactly remaining, one under, one over
    // -------------------------------------------------------------------

    #[test]
    fn test_deposit_exactly_remaining_capacity_is_accepted() {
        // deposit + current == max_total must succeed.
        assert!(validate_deposit_amount(500, 500, 1000).is_ok());
    }

    #[test]
    fn test_deposit_one_stroop_under_capacity_is_accepted() {
        // deposit + current == max_total - 1 must succeed.
        assert!(validate_deposit_amount(499, 500, 1000).is_ok());
    }

    #[test]
    fn test_deposit_one_stroop_over_capacity_is_rejected() {
        // deposit + current == max_total + 1 must fail.
        assert_eq!(
            validate_deposit_amount(501, 500, 1000),
            Error::InvalidMilestoneAmount
        );
    }

    #[test]
    fn test_deposit_on_fully_funded_contract_is_rejected() {
        // Any deposit when the contract is already fully funded must be
        // rejected, even a single stroop.
        assert_eq!(
            validate_deposit_amount(1, 1000, 1000),
            Error::InvalidMilestoneAmount
        );
    }

    #[test]
    fn test_deposit_overflow_is_reported_as_potential_overflow() {
        // Current deposited near i128::MAX plus a positive deposit would
        // overflow. The validator must return PotentialOverflow rather
        // than panicking.
        assert_eq!(
            validate_deposit_amount(1, i128::MAX, i128::MAX),
            Error::PotentialOverflow
        );
    }

    #[test]
    fn test_validate_contract_total_works() {
        // Test valid totals
        assert!(validate_contract_total(100_0000000, MAX_TOTAL_ESCROW_STROOPS).is_ok());
        assert!(
            validate_contract_total(MAX_TOTAL_ESCROW_STROOPS, MAX_TOTAL_ESCROW_STROOPS).is_ok()
        );

        // Test invalid totals
        assert_eq!(
            validate_contract_total(MAX_TOTAL_ESCROW_STROOPS + 1, MAX_TOTAL_ESCROW_STROOPS),
            Error::InvalidMilestoneAmount
        );
    }

    #[test]
    fn test_validate_contract_total_exact_ceiling_is_accepted() {
        // The exact ceiling is accepted.
        assert!(validate_contract_total(MAX_TOTAL, MAX_TOTAL).is_ok());
        // One stroop above the ceiling is rejected.
        assert_eq!(
            validate_contract_total(MAX_TOTAL + 1, MAX_TOTAL),
            Error::InvalidMilestoneAmount
        );
    }

    #[test]
    fn test_validate_contract_total_zero_is_accepted() {
        // Zero total is within bounds for the contract-total check.
        // This is the default state of a newly created contract.
        assert!(validate_contract_total(0, MAX_TOTAL).is_ok());
    }

    // -------------------------------------------------------------------
    // Array accumulation and overflow safety
    // -------------------------------------------------------------------

    #[test]
    fn test_validate_amount_array_accumulates_correctly() {
        let amounts = [100_0000000, 200_0000000, 300_0000000];
        assert_eq!(validate_amount_array(&amounts).unwrap(), 600_0000000);
    }

    #[test]
    fn test_validate_amount_array_rejects_non_positive() {
        let with_zero = [100_0000000, 0, 300_0000000];
        assert_eq!(
            validate_amount_array(&with_zero),
            Error::AmountMustBePositive
        );
        let with_negative = [100_0000000, -50_0000000, 300_0000000];
        assert_eq!(
            validate_amount_array(&with_negative),
            Error::AmountMustBePositive
        );
    }

    #[test]
    fn test_accumulate_amounts_rejects_overflow() {
        // Two valid single amounts whose sum overflows i128 must report
        // PotentialOverflow rather than panicking.
        let amounts = [i128::MAX - 1, i128::MAX].iter().copied();
        assert_eq!(
            accumulate_amounts(amounts),
            Error::PotentialOverflow
        );
    }

    #[test]
    fn test_accumulate_amounts_rejects_non_positive() {
        let amounts = [100, 0, 200].iter().copied();
        assert_eq!(
            accumulate_amounts(amounts),
            Error::AmountMustBePositive
        );
    }

    // -------------------------------------------------------------------
    // Safe arithmetic helpers
    // -------------------------------------------------------------------

    #[test]
    fn test_safe_arithmetic_works() {
        // Test safe addition
        assert_eq!(safe_add_amounts(100, 200), Some(300));
        assert_eq!(safe_add_amounts(0, 0), Some(0));
        assert_eq!(safe_add_amounts(i128::MAX, 1), None);
        assert_eq!(safe_add_amounts(i128::MIN, -1), None);

        // Test safe subtraction
        assert_eq!(safe_subtract_amounts(300, 100), Some(200));
        assert_eq!(safe_subtract_amounts(100, 100), Some(0));
        // Underflow must return None rather than panicking.
        assert_eq!(safe_subtract_amounts(i128::MIN, 1), None);
    }

    #[test]
    fn test_safe_subtract_amounts_underflow_is_none() {
        // 0 - 1 would underflow if we were working with unsigned integers.
        // For i128 this is a valid negative value, but the extreme boundary
        // must still report None.
        assert_eq!(safe_subtract_amounts(0, 1), Some(-1));
        assert_eq!(safe_subtract_amounts(i128::MIN, 1), None);
    }

    // -------------------------------------------------------------------
    // Edge cases and determinism
    // -------------------------------------------------------------------

    #[test]
    fn test_edge_cases() {
        // Test minimum positive amounts
        assert!(validate_single_amount(MIN_POSITIVE_AMOUNT).is_ok());
        let small_milestones = [1, 1, 1];
        assert!(validate_milestone_amounts(&small_milestones, MAX_TOTAL_ESCROW_STROOPS).is_ok());

        // Test boundary values
        assert!(validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS).is_ok());
        assert_eq!(
            validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS + 1),
            Error::InvalidMilestoneAmount
        );

        // Test contract boundary
        let boundary_milestones = [MAX_TOTAL_ESCROW_STROOPS];
        assert!(validate_milestone_amounts(&boundary_milestones, MAX_TOTAL_ESCROW_STROOPS).is_ok());

        let over_boundary_milestones = [MAX_TOTAL_ESCROW_STROOPS + 1];
        assert_eq!(
            validate_milestone_amounts(&over_boundary_milestones, MAX_TOTAL_ESCROW_STROOPS),
            Error::InvalidMilestoneAmount
        );
    }

    #[test]
    fn test_duplicate_submissions_are_idempotent() {
        // Repeated validation of the same input must produce identical
        // results and must not mutate any state. This guarantees that
        // duplicate submissions are safe at the validation layer.
        for _ in 0..10 {
            assert_eq!(
                validate_single_amount(MAX_SINGLE),
                Ok(())
            );
            assert_eq!(
                validate_single_amount(0),
                Error::AmountMustBePositive
            );
            assert_eq!(
                validate_deposit_amount(500, 500, 1000),
                Ok(())
            );
            assert_eq!(
                validate_deposit_amount(501, 500, 1000),
                Error::InvalidMilestoneAmount
            );
        }
    }

    #[test]
    fn test_constants_are_reasonable() {
        // Verify constants are set to reasonable values
        assert_eq!(MIN_POSITIVE_AMOUNT, 1);
        assert_eq!(MAX_SINGLE_AMOUNT_STROOPS, 1_000_000_0000000); // 1M tokens
        assert_eq!(MAX_TOTAL_ESCROW_STROOPS, 1_000_000_0000000); // 1M tokens

        // Verify max single amount doesn't exceed contract max
        assert!(MAX_SINGLE_AMOUNT_STROOPS <= MAX_TOTAL_ESCROW_STROOPS);

        // Stroop precision is documented as 7 decimal places.
        assert_eq!(STROOP_PRECISION, 7);
    }

    #[test]
    fn test_stroop_precision_documented() {
        // All i128 values are valid stroop amounts since stroop is the
        // smallest unit. This test documents the precision requirements
        // and guarantees that fractional token amounts remain representable.
        let valid_stroop_amounts = [
            1,           // 1 stroop
            100,         // 100 stroops
            1_0000000,   // 1 token
            123_1234567, // 123.1234567 tokens
        ];

        for amount in valid_stroop_amounts {
            assert!(validate_single_amount(amount).is_ok());
        }
    }

    #[test]
    fn test_large_amount_arrays_are_accumulated_safely() {
        // Test with the maximum number of milestones (10) at 1 token each.
        let many_milestones = [100_0000000; 10];
        assert_eq!(
            validate_milestone_amounts(&many_milestones, MAX_TOTAL).unwrap(),
            1_000_000_0000
        );
    }

    #[test]
    fn test_large_array_overflow_is_reported() {
        // A large array of valid single amounts whose sum overflows i128
        // must report PotentialOverflow rather than panicking.
        let amounts = vec![i128; 4];
        for _ in 0..4 {
            amounts.push(i128::MAX / 2);
        }
        assert_eq!(
            validate_amount_array(&amounts),
            Error::PotentialOverflow
        );
    }
}
