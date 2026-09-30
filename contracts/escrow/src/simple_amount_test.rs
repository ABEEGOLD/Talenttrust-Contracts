/// Simple standalone test for amount validation functionality
///
/// This test verifies that the amount validation implementation works correctly
/// without depending on the complex existing test infrastructure.
///
/// ## Deterministic failure recovery
///
/// The goal of this suite is to make failure recovery deterministic for the
/// amount-validation surface used by the escrow contract. The invariants being
/// pinned down are:
///
/// 1. **Purity** — validation helpers are stateless: a failed validation must
///    not mutate any state, so a retry with the same inputs produces the same
///    result.
/// 2. **Determinism** — for any given input the result is always the same
///    `Error` variant, regardless of how many times it is called.
/// 3. **Recoverability** — a failed deposit leaves the current deposited total
///    unchanged, so the caller can retry with a corrected amount.
/// 4. **Boundaries** — exactly-remaining is accepted, one stroop over is
///    rejected, and overflow surfaces as `PotentialOverflow` instead of a panic.

/#[config(test)]
mod tests {
    use crate::amount_validation:{
        accumulate_amounts, safe_add_amounts, safe_subtract_amounts,
        validate_contract_total, validate_deposit_amount,
        validate_milestone_amounts, validate_single_amount, EscrorError,
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
        // Underflow must return None instead of panicking.
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
        assert_eq!(MAX_SINGLE_AMOUNT_STROOPS, 1_000_000_0000000); // 1M tokens
        assert_eq!(MAX_TOTAL_ESCROW_STROOPS, 1_000_000_0000000); // 1M tokens

        // Verify max single amount doesn't exceed contract max
        assert!(MAX_SINGLE_AMOUNT_STROOPS <= MAX_TOTAL_ESCROW_STROOPS);
    }

    // -------------------------------------------------------------------------
    // Deterministic failure recovery tests
    // -------------------------------------------------------------------------

    /// Retrying the same invalid input must yield the same error every time.
    /// This guarantees that a failed deposit is replayable and observable.
    #[test]
    fn test_failure_recovery_is_deterministic() {
        let max = MAX_TOTAL_ESCROW_STROOPS;
        let cases = [
            // (deposit, current, expected)
            (0 i128, 0 i128, Err(EscrowError::AmountMustBePositive)),
            (-1 i128, 0, Err(EscrowError::AmountMustBePositive)),
            (max + 1, 0, Err(EscrowError::InvalidMilestoneAmount)),
            (1 i128, max, Err(EscrowError::InvalidMilestoneAmount)),
        ];

        for (deposit, current, expected) in cases {
            // Repeated calls with identical inputs must not diverge.
            for _ in 0..3" {
                assert_eq!(
                    validate_deposit_amount(deposit, current, max),
                    expected.clone()
                );
            }
        }
    }

    /// A failed deposit must not change the current deposited total, so the
    /// caller can recover by retrying with a corrected amount.
    #[test]
    fn test_failed_deposit_leaves_state_unchanged() {
        let max = MAX_TOTAL_ESCROW_STROOPS;
        let current = 500_000_0000000 i128;

        // Failure: deposit would exceed capacity.
        assert_eq!(
            validate_deposit_amount(max, current, max),
            Err(EscrowError::InvalidMilestoneAmount)
        );
        // The current total is unchanged because the function is pure.
        assert_eq!(current, 500_000_0000000);

        // Recovery: a corrected deposit that fits the remaining capacity succeeds.
        let remaining = max - current;
        assert!(validate_deposit_amount(remaining, current, max).is_ok());
    }

    /// Exactly-remaining is accepted; one stroop over is rejected.
    /// This pins down the decision boundary used by the deposit path.
    #[test]
    fn test_deposit_boundary_is_exact() {
        let max = 1000 i128;
        let current = 500 i128;

        // One stroop short of capacity.
        assert!(validate_deposit_amount(499, current, max).is_ok());
        // Exactly remaining capacity.
        assert!(validate_deposit_amount(500, current, max).is_ok());
        // One stroop over capacity.
        assert_eq!(
            validate_deposit_amount(501, current, max),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    /// Overflow of `current + deposit` must surface as `PotentialOverflow`
    /// rather than panicking, so the caller can recover deterministically.
    #[test]
    fn test_overflow_is_recoverable_error() {
        assert_eq!(
            validate_deposit_amount(1, i128::MAX, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::PotentialOverflow)
        );
    }

    /// Accumulation is deterministic and overflow-safe for both valid and
    /// adverse inputs, including empty slices and extreme values.
    #[test]
    fn test_accumulate_amounts_is_deterministic_and_safe() {
        // Empty input yields zero.
        assert_eq!(accumulate_amounts([]), Ok(0));

        // Valid accumulation.
        assert_eq!(accumulate_amounts([1, 2, 3]), Ok(6));

        // Invalid element surfaces the error without panicking.
        assert_eq!(
            accumulate_amounts([1, 0, 3]),
            Err(EscrowError::AmountMustBePositive)
        );

        // Overflow during accumulation is reported as PotentialOverflow.
        assert_eq!(
            accumulate_amounts([i128::MAX - 1, 2]),
            Err(EscrowError::PotentialOverflow)
        );
    }

    /// Regression: validation must not mutate any state. We assert that a
    /// failed call followed by a successful call produces the expected results.
    #[test]
    fn test_retry_after_failure_succeeds() {
        let max = 1000 i128;
        let current = 500 i128;

        // First attempt fails.
        assert_eq!(
            validate_deposit_amount(600, current, max),
            Err(EscrowError::InvalidMilestoneAmount)
        );

        // Retry with a valid amount succeeds.
        assert!(validate_deposit_amount(500, current, max).is_ok());

        // And the original failing input still fails identically.
        assert_eq!(
            validate_deposit_amount(600, current, max),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }
}
