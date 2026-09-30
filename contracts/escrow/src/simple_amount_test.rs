/// Simple standalone test for amount validation functionality
///
/// This test verifies that the amount validation implementation works correctly
/// without depending on the complex existing test infrastructure.
///
/// ## Compatibility Contract
///
/// This module is the canonical guard for the public behavior of the
/// `amount_validation` family of helpers. The assertions below encode the
/// external contract that callers (deposit and milestone entrypoints) rely
/// on. Any change to the underlying implementation must keep these
/// assertions green, or ship with an explicit, tested migration path.
///
/// ### Invariants
///
/// 1. Positivity: any amount `<= 0` is rejected with `AmountMustBePositive`.
/// 2. Upper bound: any amount `> MAX_SINGLE_AMOUNT_STROOPS` is rejected
///    with `InvalidMilestoneAmount`.
/// 3. Aggregate bound: the sum of all milestones must not exceed the
///    contract maximum.
/// 4. Overflow safety: safe addition/subtraction return `None` on
///    overflow/underflow instead of panicking.
/// 5. Determinism: equal inputs always produce equal outcomes; no hidden
///    state or ordering effects.

/#[cfg(test)]
mod tests {
    use crate::amount_validation {
        accumulate_amounts, safe_add_amounts, safe_subtract_amounts,
        validate_contract_total, validate_deposit_amount,
        validate_milestone_amounts, validate_single_amount, EscrowError,
        MAX_SINGLE_AMOUNT_STROOPS, MIN_POSITIVE_AMOUNT,
    };
    use crate::MAX_TOTAL_ESCROW_STROOPS;

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
        // Underflow must return None, not panic.
        assert_eq!(safe_subtract_amounts(0, 1), None);
        assert_eq!(safe_subtract_amounts(i128::MIN, 1), None);
    }

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
        assert_eq!(MAX_SINGLE_AMOUNT_STROOPS, 1_000_000_0000000); // 1MM tokens
        assert_eq!(MAX_TOTAL_ESCROW_STROOPS, 1_000_000_0000000); // 1M tokens

        // Verify max single amount doesn't exceed contract max
        assert!(MAX_SINGLE_AMOUNT_STROOPS <= MAX_TOTAL_ESCROW_STROOPS);
    }

    // ----------------------------------------------------------------------
    // Compatibility contract regression tests
    // ----------------------------------------------------------------------

    /// Empty input must be a zero total, consistently and without panic.
    #[test]
    fn test_empty_milestones_are_zero_total() {
        let empty: [i128; 0] = [];
        assert_eq!(
            validate_milestone_amounts(&empty, MAX_TOTAL_ESCROW_STROOPS),
            Ok(0)
        );
        assert_eq!(accumulate_amounts(empty), Ok(0));
    }

    /// Duplicate amounts are summed exactly once each; no deduplication.
    #[test]
    fn test_duplicate_amounts_are_summed_exactly() {
        let dups = [100_0000000, 100_0000000, 100_0000000];
        assert_eq!(
            validate_milestone_amounts(&dups, MAX_TOTAL_ESCROW_STROOPS),
            Ok(300_0000000)
        );
    }

    /// Accumulator must match the validator for the same input.
    #[test]
    fn test_accumulator_matches_validator() {
        let amounts = [1_0000000, 2_0000000, 3_0000000];
        let via_validator =
            validate_milestone_amounts(&amounts, MAX_TOTAL_ESCROW_STROOPS).unwrap();
        let via_accumulator = accumulate_amounts(amounts).unwrap();
        assert_eq!(via_validator, via_accumulator);
    }

    /// Accumulator rejects non-positive entries just like the validator.
    #[test]
    fn test_accumulator_rejects_non_positive() {
        assert_eq!(
            accumulate_amounts([1, 0, 1]),
            Err(EscrowError::AmountMustBePositive)
        );
        assert_eq!(
            accumulate_amounts([1, -1, 1]),
            Err(EscrowError::AmountMustBePositive)
        );
    }

    /// Accumulator rejects over-bound individual amounts.
    #[test]
    fn test_accumulator_rejects_over_bound() {
        assert_eq!(
            accumulate_amounts([MAX_SINGLE_AMOUNT_STROOPS + 1]),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    /// Accumulator detects overflow without panicking.
    #[test]
    fn test_accumulator_detects_overflow() {
        // Use amounts that are individually valid but whose sum overflows i128.
        // MAX_SINGLE_AMOUNT_STROOPS is far below i128::MAX, so we can add
        // many of them to exercise the overflow path.
        let big = MAX_SINGLE_AMOUNT_STROOPS;
        // i128::MAX / big >> 1, so this iterator eventually overflows.
        let count = (i128::MAX / big) as usize + 3;
        let result = accumulate_amounts(core::iter::repeat(big).take(count));
        assert_eq!(result, Err(EscrowError::PotentialOverflow));
    }

    /// Determinism: equal inputs must yield equal outcomes across repeated
    /// invocations, and order must not matter for the sum.
    #[test]
    fn test_determinism_and_order_independence() {
        let a = [10_0000000, 20_0000000, 30_0000000];
        let b = [30_0000000, 10_0000000, 20_0000000];
        let ra1 = validate_milestone_amounts(&a, MAX_TOTAL_ESCROW_STROOPS);
        let ra2 = validate_milestone_amounts(&a, MAX_TOTAL_ESCROW_STROOPS);
        let rb = validate_milestone_amounts(&b, MAX_TOTAL_ESCROW_STROOPS);
        assert_eq!(ra1, ra2);
        assert_eq!(ra1, rb);
    }

    /// Deposit boundary: exactly-remaining succeeds, and one stroop over
    /// fails. This is the canonical compatibility contract for deposit
    /// entrypoints.
    #[test]
    fn test_deposit_exact_boundary() {
        let max = 1000;
        assert_eq!(validate_deposit_amount(500, 500, max), Ok());
        assert_eq!(
            validate_deposit_amount(501, 500, max),
            Err(EscrowError::InvalidMilestoneAmount)
        );
        // Already fully funded contract rejects any further deposit.
        assert_eq!(
            validate_deposit_amount(1, max, max),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    /// Deposit overflow path must surface as PotentialOverflow, not panic.
    #[test]
    fn test_deposit_overflow_is_reported() {
        assert_eq!(
            validate_deposit_amount(1, i128::MAX, i128::MAX),
            Err(EscrowError::PotentialOverflow)
        );
    }

    /// Regression: validators must not mutate their inputs and must be
    /// pure functions of their arguments.
    #[test]
    fn test_validators_are_pure() {
        let input = [1_0000000, 2_0000000];
        let before = input;
        let _ = validate_milestone_amounts(&input, MAX_TOTAL_ESCROW_STROOPS);
        assert_eq!(input, before);
    }

    /// Regression: mixed invalid input must report the first failure in
    /// iteration order (deterministic error selection).
    #[test]
    fn test_error_selection_is_deterministic() {
        // Positivity failure comes before bound failure in iteration order.
        let input = [0, MAX_SINGLE_AMOUNT_STROOPS + 1];
        assert_eq!(
            validate_milestone_amounts(&input, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::AmountMustBePositive)
        );
        // Reversed order surfaces the bound failure first.
        let reversed = [MAX_SINGLE_AMOUNT_STROOPS + 1, 0];
        assert_eq!(
            validate_milestone_amounts(&reversed, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    /// Regression: contract total validator must accept exactly the max
    /// and reject one stroop over, for any max value.
    #[test]
    fn test_contract_total_boundary() {
        for max in [1, 1000, 1_000_000_0000000] {
            assert!(validate_contract_total(max, max).is_ok());
            assert_eq!(
                validate_contract_total(max + 1, max),
                Err(EscrowError::InvalidMilestoneAmount)
            );
        }
    }

    /// Regression: validate_single_amount must reject every non-positive
    /// value and accept every positive value up to the max.
    #[test]
    fn test_single_amount_boundaries() {
        for v in [i128::MIN, -1, 0] {
            assert_eq!(
                validate_single_amount(v),
                Err(EscrowError::AmountMustBePositive)
            );
        }
        for v in [1, 2, MAX_SINGLE_AMOUNT_STROOPS] {
            assert!(validate_single_amount(v).is_ok());
        }
        assert_eq!(
            validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS + 1),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }
}
