//! Simple standalone test for amount validation functionality
///
/// This test verifies that the amount validation implementation works correctly
/// without depending on the complex existing test infrastructure.
///
/// Validation boundaries (documented invariants):
/// - Accepted: amounts strictly greater than zero and within the configured caps.
/// - Rejected: zero, negative, and over-cap amounts.
/// - Boundary: exactly MIN_POSITIVE_AMOUNT, MAX_SINGLE_AMOUNT_STROOPS, and
///   MAX_TOTAL_ESCROW_STROOPS are inclusive.
/// - Duplicate: repeated identical milestone amounts are valid and must not
///   be silently collapsed or de-duplicated by validation.
/// - Overflow: arithmetic that would exceed i128 range must fail closed.
///
/// Note: the tests below exercise the public boundary of the validation
/// module only; they do not reimplement it.

#[config(test)]mod tests {
    use crate::amount_validation::{
        safe_add_amounts, safe_subtract_amounts, validate_contract_total,
        validate_deposit_amount, validate_milestone_amounts, validate_single_amount,
        EscrowError, MAX_SINGLE_AMOUNT_STROOPS, MIN_POSITIVE_AMOUNT,
    };
    use crate::MAX_TOTAL_ESCROW_STROOPS;

    // ---- Accepted input ----

    #[test]
    fn test_validate_single_amount_works() {
        // Test valid amounts
        assert!(validate_single_amount(1).is_ok());
        assert!(validate_single_amount(100_0000000).is_ok()); // 1 token
        assert!(validate_single_amount(MAX_SINGLE_AMOUNT_STROOPS).is_ok());

        // Test invalid amounts
        assert_eq(
            validate_single_amount(0),
            Err(EscrowError::AmountMustBePositive)
        );
        assert_eq(
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
        // Underflow must fail closed (None), not wrap or silently clamp.
        assert_eq!(safe_subtract_amounts(0, 1), None);
        assert_eq!(safe_subtract_amounts(i128::MIN, 1), None);
    }

    // ---- Boundary values ----

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

    // ---- Duplicate input ----

    #[test]
    fn test_duplicate_milestone_amounts_are_preserved() {
        // Repeated identical amounts are valid and must not be collapsed.
        let duplicates = [100_0000000, 100_0000000, 100_0000000];
        assert_eq!(
            validate_milestone_amounts(&duplicates, MAX_TOTAL_ESCROW_STROOPS),
            Ok(300_0000000)
        );

        // Duplicate boundary amounts that exactly fill the contract cap.
        let duplicate_boundary = [500_000_0000000, 500_000_0000000];
        assert_eq!(
            validate_milestone_amounts(&duplicate_boundary, MAX_TOTAL_ESCROW_STROOPS),
            Ok(MAX_TOTAL_ESCROW_STROOPS)
        );

        // Duplicate amounts that exceed the contract cap must be rejected.
        let duplicate_over = [600_000_0000000, 600_000_0000000];
        assert_eq!(
            validate_milestone_amounts(&duplicate_over, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    // ---- Regression / invariant guards ----

    #[test]
    fn test_constants_are_reasonable() {
        // Verify constants are set to reasonable values
        assert_eq!(MIN_POSITIVE_AMOUNT, 1);
        assert_eq!(MAX_SINGLE_AMOUNT_STROOPS, 1_000_000_0000000); // 1MM tokens
        assert_eq!(MAX_TOTAL_ESCROW_STROOPS, 1_000_000_0000000); // 1M tokens

        // Verify max single amount doesn't exceed contract max
        assert!(MAX_SINGLE_AMOUNT_STROOPS <= MAX_TOTAL_ESCROW_STROOPS);
    }

    #[test]
    fn test_milestone_sum_cannot_exceed_cap_even_without_overflow() {
        // Large individual amounts that are each valid but collectively exceed the cap.
        // This guards against a silent sum overflow or truncation in the aggregate.
        let large = [MAX_SINGLE_AMOUNT_STROOPS, MAX_SINGLE_AMOUNT_STROOPS];
        assert_eq!(
            validate_milestone_amounts(&large, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::InvalidMilestoneAmount)
        );
    }

    #[test]
    fn test_empty_milestones_are_rejected() {
        // An empty milestone set has no valid total and must not be accepted.
        let empty: [crate::amount_validation::Amount; 0] = [];
        assert_eq!(
            validate_milestone_amounts(&empty, MAX_TOTAL_ESCROW_STROOPS),
            Err(EscrowError::AmountMustBePositive)
        );
    }
}
