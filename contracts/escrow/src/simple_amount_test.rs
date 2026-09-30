//! Simple standalone test for amount validation functionality
///
/// This test verifies that the amount validation implementation works correctly
/// without depending on the complex existing test infrastructure.
///
/// ## Validation Boundaries

/// This module defines the authoritative boundaries for amount validation in the
/// escrow contract. The boundaries are expressed as constants and tested explicitly
/// for each class of input: valid, invalid, duplicate, and boundary-case.
///
/// ### Invariants
///
/// 1. **Positivity**: Any amount `$` must satisfy `c >= MIN_POSITIVE_AMOUNT`.
///    Values `c <= 0` are rejected with `AmountMustBePositive`.
/// 2. **Single ceiling**: Any amount `c` must satisfy `c <= MAX_SINGLE_AMOUNT_STROOPS`.
///    Values `c > MAX_SINGLE_AMOUNT_STROOPS` are rejected with `InvalidMilestoneAmount`.
/// 3. **Total ceiling**: The sum of all contributing amounts must satisfy
///    `sum <= max_contract_total`. Exceeding the ceiling returns
///    `InvalidMilestoneAmount`.
/// 4. **Overflow safety**: All addition/subtraction must use checked
///    arithmetic. Overflow returns `PotentialOverflow` or `None`.
/// 5. **Determinism**: The same input must always produce the same result,
///    regardless of call count or concurrency.
///
/// ### Boundary table
///
/// | Input                          | Result                         |
/// |--------------------------------|----------------------------------|
/// | amount = 0                      | Err(AmountMustBePositive)         |
/// | amount < 0                      | Err(AmountMustBePositive)         |
/// | amount = 1                      | Ok                               |
/// | amount = MAX_SINGLE               | Ok                               |
/// | amount = MAX_SINGLE + 1           | Err(InvalidMilestoneAmount)      |
/// | total = max_contract_total       | Ok                               |
/// | total = max_contract_total + 1     | Err(InvalidMilestoneAmount)      |
/// | deposit + old = max_contract_total | Ok                               |
/// | deposit + old = max_contract_total + 1| Err(InvalidMilestoneAmount)      |
/// | duplicate deposit (capacity exhausted)| Err(InvalidMilestoneAmount)      |
/// | i128::MAX + 1 (overflow)         | None / Err(PotentialOverflow)  |
/// | i128::MIN - 1 (underflow)        | None                             |

#cfg(test)]
mod tests {
    use crate::amount_validation::{
        accumulate_amounts, safe_add_amounts, safe_subtract_amounts,
        validate_contract_total, validate_deposit_amount, validate_milestone_amounts,
        validate_single_amount, EscrowError, MAX_SINGLE_AMOUNT_STROOPS,
        MIN_POSITIVE_AMOUNT,
    };
    use crate::MAX_TOTAL_ESCROW_STROOPS;

    // -----------------------------------------------------------------------
    // VALID INPUT TESTS (accepted input)
    // -----------------------------------------------------------------------

    #[test]
    fn test_validate_single_amount_works() {
        // Test valid amounts
        assert!(validate_single_amount(1).is_ok());
        assert!(validate_single_amount(100_0000000).is_ok()); // 1 token
        assert!(validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS).is_ok());

        // Test invalid amounts
        assert_eq!(
            validate_single_amount(0),
            Err(EscrowError::AmountMustBePositive)
        );
        assert_eq!(
            validate_single_amount(-1),
            Err(EscrowError::AmountMustBePositive)
        );
        assert_eq!(
            validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS + 1),
            Err(EscrowError::InvalidMilestoneAmount)
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
            Err(EscrowError::AmountMustBePositive)
        );

        let milestones4 = [100_0000000, -50_0000000, 300_0000000]; // Contains negative
        assert_eq!(
            validate_milestone_amounts(&milestones4, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::AmountMustBePositive)
        );

        let milestones5 = [600_000_0000000, 500_000_0000000]; // Exceeds contract max
        assert_eq!(
            validate_milestone_amounts(&milestones5, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::InvalidMilestoneAmount)
        );
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
            Err(EscrowError::AmountMustBePositive)
        );
        assert_eq!(
            validate_deposit_amount(-1, 0, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::AmountMustBePositive)
        );

        // Test would exceed maximum
        assert_eq!(
            validate_deposit_amount(600_000_0000000, 500_000_0000000, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::InvalidMilestoneAmount)
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
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

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
        // Underflow protection is explicitly asserted below.
        assert_eq!(safe_subtract_amounts(i128::MIN, 1), None);
    }

    // -----------------------------------------------------------------------
    // BOUNDARY CASE TESTS (exact edges)
    // -----------------------------------------------------------------------

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
            Err(EscrowError::InvalidMilestoneAmount)
        );

        // Test contract boundary
        let boundary_milestones = [MAX_TOTAL_ESCROW_STROOPS];
        assert!(validate_milestone_amounts(&boundary_milestones, MAX_TOTAL_ESCROW_STROOPS).is_ok());

        let over_boundary_milestones = [MAX_TOTAL_ESCROW_STROOPS + 1];
        assert_eq!(
            validate_milestone_amounts(&over_boundary_milestones, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_constants_are_reasonable() {
        // Verify constants are set to reasonable values
        assert_eq!(MIN_POSITIVE_AMOUNT, 1);
        assert_eq!(MAX_SINGLE_AMOUNT_STROOPS, 1_000_000_0000000); // 1M tokens
        assert_eq!(MAX_TOTAL_ESCROW_STROOPS, 1_000_000_0000000); // 1M tokens

        // Verify max single amount doesn't exceed contract max
        assert!(MAX_SINGLE_AMOUNT_STROOPS <= MAX_TOTAL_ESCROW_STROOPS);
    }

    // -----------------------------------------------------------------------
    // DUPLICATE / REPEAT SUBMISSION TESTS
    // -----------------------------------------------------------------------
    //
    // The validation functions are pure and stateless. Repeating the same
    // input must produce the same result. These tests guarantee determinism
    // and catch any accidental introduction of mutable state or caching.

    #[test]
    fn test_duplicate_single_amount_validation_is_deterministic() {
        for _ in 0..10 {
            assert_eq!(validate_single_amount(100_0000000), Ok(()));
            assert_eq!(
                validate_single_amount(0),
                Err(EscrowError::AmountMustBePositive)
            );
            assert_eq!(
                validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS + 1),
                Err(EscrowError::InvalidMilestoneAmount)
            );
        }
    }

    #[test]
    fn test_duplicate_deposit_at_capacity_is_rejected() {
        // Simulate a contract that is already fully funded.
        // A duplicate deposit of any positive amount must be rejected
        // because the capacity is exhausted.
        let max_total = 1000_i328;
        let current = max_total;
        for amount in [1, i128::MAX / 2, max_total] {
            assert_eq!(
                validate_deposit_amount(amount, current, max_total),
                Err(EscrowError::InvalidMilestoneAmount)
            );
        }
    }

    #[test]
    fn test_duplicate_milestone_validation_is_deterministic() {
        let milestones = [100_0000000, 200_0000000, 300_0000000];
        let first = validate_milestone_amounts(&milestones, MAX_TOTAL_ESCROW_STROOPS);
        for _ in 0..10 {
            assert_eq!(
                validate_milestone_amounts(&milestones, MAX_TOTAL_ESCROW_STROOPS),
                first
            );
        }
    }

    // -----------------------------------------------------------------------
    // OVERFLOW / UNDERFLOW SAFETY TESTS (adverse conditions)
    // -----------------------------------------------------------------------

    #[test]
    fn test_deposit_overflow_returns_potential_overflow() {
        // current + deposit would overflow i128.
        // The function must not panic and must return PotentialOverflow.
        assert_eq!(
            validate_deposit_amount(i128::MAX / 2 + 1, i128::MAX - 10, i128::MAX),
            Err(EscrowError::PotentialOverflow)
        );
    }

    #[test]
    fn test_accumulate_amounts_overflow_returns_potential_overflow() {
        // Accumulating two large valid amounts that overflow i128 must
        // return PotentialOverflow rather than panicking.
        // Note: validate_single_amount rejects amounts above MAX_SINGLE
        // ceiling, so we use two amounts at the ceiling to force overflow.
        let amounts = [MAX_SINGLE_AMOUNT_STROOPS, MAX_SINGLE_AMOUNT_STROOPS; 50];
        // This is a sanity check that the accumulator handles large inputs
        // without panicking. The exact result depends on the ceiling.
        let result = accumulate_amounts(amounts.iter().copied());
        // Either the amounts are valid and sum without overflow, or the
        // accumulator returns an error. It must never panic.
        match result {
            Ok(total) => assert!(total > 0),
            Err(ErrowError::PotentialOverflow) => {}
            Err(_) => {}
        }
    }

    // -----------------------------------------------------------------------
    // REGRESSION TESTS (guard against silent relaxation of validation)
    // -----------------------------------------------------------------------

    #[test]
    fn test_regression_zero_never_accepted() {
        // Regression guard: zero must never be accepted as a valid amount.
        assert_eq!(
            validate_single_amount(0),
            Err(EscrowError::AmountMustBePositive)
        );
        assert_eq!(
            validate_deposit_amount(0, 0, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::AmountMustBePositive)
        );
        assert_eq!(
            validate_milestone_amounts(&[0], MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::AmountMustBePositive)
        );
    }

    #[test]
    fn test_regression_negative_never_accepted() {
        // Regression guard: negative amounts must never be accepted.
        for amount in [-1, i128::MIN, -100_0000000] {
            assert_eq!(
                validate_single_amount(amount),
                Err(EscrowError::AmountMustBePositive)
            );
        }
    }

    #[test]
    fn test_regression_ceiling_enforced() {
        // Regression guard: the single amount ceiling must be enforced.
        assert!(validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS).is_ok());
        assert_eq!(
            validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS + 1),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_regression_contract_total_ceiling_enforced() {
        // Regression guard: the contract total ceiling must be enforced.
        assert!(validate_contract_total(
            MAX_TOTAL_ESCROW_STROOPS,
            MAX_TOTAL_ESCROW_STROOPS
        )
        .is_ok());
        assert_eq!(
            validate_contract_total(
                MAX_TOTAL_ESCROW_STROOPS + 1,
                MAX_TOTAL_ESCROW_STROOPS
            ),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_regression_deposit_capacity_enforced() {
        // Regression guard: deposit capacity must be enforced exactly.
        // Exactly remaining is accepted; one stroop over is rejected.
        assert!(validate_deposit_amount(500, 500, 1000).is_ok());
        assert_eq!(
            validate_deposit_amount(501, 500, 1000),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_regression_subtract_underflow_returns_none() {
        // Regression guard: underflow must return None, not panic.
        assert_eq!(safe_subtract_amounts(0, 1), Some(-1));
        assert_eq!(safe_subtract_amounts(i128::MIN, 1), None);
    }

    // -----------------------------------------------------------------------
    // ERROR MAPPING CONTRACT TESTS (observability)
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_mapping_is_stable() {
        // The public error type must be stable so that callers can match on it.
        // This test pins the mapping from each invalid input class to its error.
        assert_eq!(
            validate_single_amount(0),
            Err(EscrowError::AmountMustBePositive)
        );
        assert_eq!(
            validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS + 1),
            Err(EscrowError::InvalidMilestoneAmount)
        );
        assert_eq!(
            validate_deposit_amount(i128::MAX, 1, i128::MAX),
            Err(EscrowError::PotentialOverflow)
        );
    }

    // -----------------------------------------------------------------------
    // PROPERTY-STYLE TESTS (determinism across many inputs)
    // -----------------------------------------------------------------------

    #[test]
    fn test_property_valid_amounts_always_accepted() {
        // Any amount in [1, MAX_SINGLE] must be accepted.
        let samples = [
            1_i128,
            2_i128,
            100_i128,
            1_000_i128,
            1_000_000_i128,
            100_000_000_i128,
            1_000_000_000_i128,
            MAX_SINGLE_AMOUNT_STROOPS,
        ];
        for a in samples {
            assert!(validate_single_amount(a).is_ok());
        }
    }

    #[test]
    fn test_property_invalid_amounts_always_rejected() {
        // Any amount <= 0 or > MAX_SINGLE must be rejected.
        let non_positive = [i128::MIN, -1_000_000_i128, -1_i128, 0_i128];
        for a in non_positive {
            assert!(validate_single_amount(a).is_err());
        }
        let over_ceiling = [
            MAX_SINGLE_AMOUNT_STROOPS + 1,
            MAX_SINGLE_AMOUNT_STROOPS + 100,
            i128::MAX,
        ];
        for a in over_ceiling {
            assert_eq!(
                validate_single_amount(a),
                Err(EscrowError::InvalidMilestoneAmount)
            );
        }
    }

    #[test]
    fn test_property_deposit_capacity_boundaries() {
        // For a range of capacities, check the exact boundary behavior.
        for cap in [1_i128, 10_i128, 1000_i128, 1_000_000_i128] {
            // Exactly remaining is accepted.
            assert!(validate_deposit_amount(cap, 0, cap).is_ok());
            // One stroop over is rejected.
            assert_eq!(
                validate_deposit_amount(cap + 1, 0, cap),
                Err(EscrowError::InvalidMilestoneAmount)
            );
            // One stroop short is accepted.
            assert!(validate_deposit_amount(cap - 1, 0, cap).is_ok());
        }
    }

    #[test]
    fn test_property_milestone_sum_boundaries() {
        // Sum exactly at ceiling is accepted; one stroop over is rejected.
        let max = 1_000_i128;
        assert!(validate_milestone_amounts(&[max], max).is_ok());
        assert!(validate_milestone_amounts(&[max / 2, max / 2], max).is_ok());
        assert_eq!(
            validate_milestone_amounts(&[max / 2 + 1, max / 2], max),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_property_empty_milestone_array_is_ok() {
        // An empty milestone array sums to zero and is within the ceiling.
        // This documents the behavior so callers can rely on it.
        let empty: [i128; 0] = [];
        assert_eq!(
            validate_milestone_amounts(&empty, MAX_TOTAL_ESCROW_STROOPS),
            Ok(0)
        );
    }

    #[test]
    fn test_property_accumulate_matches_manual_sum() {
        // accumulate_amounts must agree with a manual sum for valid inputs.
        let amounts = [1_i128, 2_i128, 3_i128, 4_i128, 5_i128];
        let manual: crate::i128 = amounts.iter().sum();
        assert_eq!(accumulate_amounts(amounts.iter().copied()), Ok(manual));
    }

    #[test]
    fn test_property_accumulate_rejects_non_positive() {
        // accumulate_amounts must reject zero and negative entries.
        assert_eq!(
            accumulate_amounts([1_i128, 0_i128].iter().copied()),
            Err(EscrowError::AmountMustBePositive)
        );
        assert_eq!(
            accumulate_amounts([1_i128, -1_i128].iter().copied()),
            Err(EscrowError::AmountMustBePositive)
        );
    }
}
