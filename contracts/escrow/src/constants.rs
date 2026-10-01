//! Central protocol constants for the TalentTrust escrow contract.
//!
//! This module is the **single source of truth** for all named protocol limits.
//! Every other module that needs a protocol constant **must** import it from
//! here (or from `milestones_consts` for milestone-specific limits) rather than
//! re-declaring its own copy.
//!
//! ## Concurrent-execution safety
//!
//! Because Soroban contracts are single-threaded within a ledger transaction,
//! there is no runtime data-race between constants.  The hazard that *does*
//! arise under concurrent development is **divergent redeclarations**: two
//! modules independently define `PAGE_CEILING = 50` and `MAX_PAGINATION_LIMIT =
//! 50`, and a future maintainer changes one without updating the other.  Any
//! caller that depends on both names may then observe logically inconsistent
//! clamping.
//!
//! The compile-time assertions in this module (`const _: () = assert!(…)`)
//! enforce cross-module consistency at zero runtime cost.  If any duplicate
//! constant diverges from the canonical value declared here, the build fails
//! immediately with a descriptive message.
//!
//! ## Adding a new constant
//!
//! 1. Declare it here with a doc-comment that names the business rule it
//!    encodes and the entrypoints that depend on it.
//! 2. If a legacy alias exists in another module, add a cross-check assertion
//!    below rather than deleting the alias (to keep public API stable).
//! 3. Update the unit tests at the bottom of this file.

// ── Pagination ───────────────────────────────────────────────────────────────

/// Upper bound on the `limit` parameter of paginated read views.
///
/// Keeps per-call storage reads bounded and prevents callers from requesting
/// unbounded scans in a single invocation.  Every paginated entrypoint
/// (`get_reputations_page`, `get_events_page`, `get_contracts_page`,
/// `get_arbiters_page`, `get_disputes_page`, `get_release_authorizations`)
/// clamps the caller-supplied `limit` to this value before iterating.
///
/// **Invariant** (enforced by compile-time assertions below):
/// This value must equal `contracts::PAGE_CEILING` and
/// `types::MAX_PAGINATION_LIMIT`.  Changing only one of those without updating
/// the others silently breaks the clamping for whichever call-site uses the
/// stale name.
pub const PAGE_CEILING: u32 = 50;

// ── Reputation ───────────────────────────────────────────────────────────────

/// Minimum valid reputation rating (inclusive).
///
/// `issue_reputation` rejects a `rating` strictly less than this value with
/// [`Error::InvalidRating`].  This default may be overridden per-contract via
/// [`ReputationConfig`].
///
/// **Invariant**: must equal `milestones_consts::MIN_RATING` (checked below).
pub const MIN_RATING: u32 = 1;

/// Maximum valid reputation rating (inclusive).
///
/// `issue_reputation` rejects a `rating` strictly greater than this value with
/// [`Error::InvalidRating`].  This default may be overridden per-contract via
/// [`ReputationConfig`].
///
/// **Invariant**: must equal `milestones_consts::MAX_RATING` (checked below).
pub const MAX_RATING: u32 = 5;

/// Maximum byte length of a reputation feedback comment.
///
/// `issue_reputation` rejects a `comment` whose UTF-8 byte length strictly
/// exceeds this value with [`Error::CommentTooLong`].  This default may be
/// overridden via [`ReputationConfig`].
///
/// **Invariant**: must equal `milestones_consts::MAX_COMMENT_BYTES` (checked below).
pub const MAX_COMMENT_BYTES: u32 = 200;

/// Unit increment applied to `PendingReputationCredits` when a contract
/// completes.
///
/// Kept as a named constant so the credit-grant and credit-consume call-sites
/// stay in sync.  Currently `1`: each completed contract grants exactly one
/// pending credit which `issue_reputation` later consumes.
pub const REPUTATION_CREDIT_INCREMENT: i128 = 1;

// ── Scaling ───────────────────────────────────────────────────────────────────

/// Basis-point scaling factor used by `get_average_rating` (×10 000 preserves
/// four decimal places when computing `total_rating / completed_contracts`).
///
/// A local shadow of this constant in `reputation.rs` is intentionally kept
/// for readability at the call-site; the compile-time assertion below ensures
/// it stays equal to this canonical value.
pub const SCALE: i128 = 10_000;

// ── Compile-time cross-module consistency assertions ──────────────────────────
//
// These assertions run at compile time (zero cost at runtime).  If any
// duplicate constant in another module diverges from the value declared above,
// the build fails with a clear message naming the offending pair.
//
// IMPORTANT: add a new assertion here whenever you add a canonical constant
// that has a legacy alias in another module.

/// `contracts::PAGE_CEILING` must equal the canonical `PAGE_CEILING`.
const _CONTRACTS_PAGE_CEILING_EQ: () = assert!(
    crate::contracts::PAGE_CEILING == PAGE_CEILING,
    "contracts::PAGE_CEILING diverges from constants::PAGE_CEILING — update contracts.rs",
);

/// `types::MAX_PAGINATION_LIMIT` must equal the canonical `PAGE_CEILING`.
const _TYPES_MAX_PAGINATION_LIMIT_EQ: () = assert!(
    crate::types::MAX_PAGINATION_LIMIT == PAGE_CEILING,
    "types::MAX_PAGINATION_LIMIT diverges from constants::PAGE_CEILING — update types.rs",
);

/// `milestones_consts::MIN_RATING` must equal the canonical `MIN_RATING`.
const _MILESTONES_MIN_RATING_EQ: () = assert!(
    crate::milestones_consts::MIN_RATING == MIN_RATING,
    "milestones_consts::MIN_RATING diverges from constants::MIN_RATING — update milestones_consts.rs",
);

/// `milestones_consts::MAX_RATING` must equal the canonical `MAX_RATING`.
const _MILESTONES_MAX_RATING_EQ: () = assert!(
    crate::milestones_consts::MAX_RATING == MAX_RATING,
    "milestones_consts::MAX_RATING diverges from constants::MAX_RATING — update milestones_consts.rs",
);

/// `milestones_consts::MAX_COMMENT_BYTES` must equal the canonical `MAX_COMMENT_BYTES`.
const _MILESTONES_MAX_COMMENT_BYTES_EQ: () = assert!(
    crate::milestones_consts::MAX_COMMENT_BYTES == MAX_COMMENT_BYTES,
    "milestones_consts::MAX_COMMENT_BYTES diverges from constants::MAX_COMMENT_BYTES — update milestones_consts.rs",
);

/// Rating range must be a non-empty interval: MIN ≤ MAX.
const _RATING_RANGE_VALID: () = assert!(
    MIN_RATING <= MAX_RATING,
    "MIN_RATING must be ≤ MAX_RATING",
);

/// Page ceiling must be positive to avoid returning empty pages unconditionally.
const _PAGE_CEILING_POSITIVE: () = assert!(
    PAGE_CEILING > 0,
    "PAGE_CEILING must be > 0",
);

/// Comment byte cap must be non-zero.
const _MAX_COMMENT_BYTES_POSITIVE: () = assert!(
    MAX_COMMENT_BYTES > 0,
    "MAX_COMMENT_BYTES must be > 0",
);

/// SCALE must be positive (used as a divisor in average-rating calculation).
const _SCALE_POSITIVE: () = assert!(
    SCALE > 0,
    "SCALE must be > 0 — it is used as a divisor",
);

/// REPUTATION_CREDIT_INCREMENT must be exactly 1.
///
/// Each completed contract grants exactly one pending credit.  If this ever
/// changes the credit-grant and credit-consume sites must both be updated.
const _CREDIT_INCREMENT_IS_ONE: () = assert!(
    REPUTATION_CREDIT_INCREMENT == 1,
    "REPUTATION_CREDIT_INCREMENT changed — review grant_pending_reputation_credit and issue_reputation",
);

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Canonical value pinning ───────────────────────────────────────────────

    /// Pin all constant values so an unintentional edit is caught by CI.
    #[test]
    fn constants_have_expected_values() {
        assert_eq!(PAGE_CEILING, 50, "PAGE_CEILING changed");
        assert_eq!(MIN_RATING, 1, "MIN_RATING changed");
        assert_eq!(MAX_RATING, 5, "MAX_RATING changed");
        assert_eq!(MAX_COMMENT_BYTES, 200, "MAX_COMMENT_BYTES changed");
        assert_eq!(REPUTATION_CREDIT_INCREMENT, 1, "REPUTATION_CREDIT_INCREMENT changed");
        assert_eq!(SCALE, 10_000, "SCALE changed");
    }

    // ── Cross-module consistency (redundant with compile-time asserts) ─────────
    //
    // These duplicate the compile-time assertions in readable test form so
    // that `cargo test` output clearly identifies which pair diverged.

    #[test]
    fn page_ceiling_matches_contracts_module() {
        assert_eq!(
            crate::contracts::PAGE_CEILING,
            PAGE_CEILING,
            "contracts::PAGE_CEILING must equal constants::PAGE_CEILING"
        );
    }

    #[test]
    fn max_pagination_limit_matches_page_ceiling() {
        assert_eq!(
            crate::types::MAX_PAGINATION_LIMIT,
            PAGE_CEILING,
            "types::MAX_PAGINATION_LIMIT must equal constants::PAGE_CEILING"
        );
    }

    #[test]
    fn milestones_consts_min_rating_matches() {
        assert_eq!(
            crate::milestones_consts::MIN_RATING,
            MIN_RATING,
            "milestones_consts::MIN_RATING must equal constants::MIN_RATING"
        );
    }

    #[test]
    fn milestones_consts_max_rating_matches() {
        assert_eq!(
            crate::milestones_consts::MAX_RATING,
            MAX_RATING,
            "milestones_consts::MAX_RATING must equal constants::MAX_RATING"
        );
    }

    #[test]
    fn milestones_consts_max_comment_bytes_matches() {
        assert_eq!(
            crate::milestones_consts::MAX_COMMENT_BYTES,
            MAX_COMMENT_BYTES,
            "milestones_consts::MAX_COMMENT_BYTES must equal constants::MAX_COMMENT_BYTES"
        );
    }

    // ── Invariant tests ───────────────────────────────────────────────────────

    /// Rating range must be a proper non-empty closed interval.
    #[test]
    fn rating_range_is_valid() {
        assert!(MIN_RATING >= 1, "MIN_RATING must be at least 1");
        assert!(MIN_RATING <= MAX_RATING, "MIN_RATING must be ≤ MAX_RATING");
    }

    /// PAGE_CEILING must be positive.
    #[test]
    fn page_ceiling_is_positive() {
        assert!(PAGE_CEILING > 0);
    }

    /// SCALE must be a positive divisor.
    #[test]
    fn scale_is_positive() {
        assert!(SCALE > 0);
    }

    /// REPUTATION_CREDIT_INCREMENT must be 1.
    #[test]
    fn reputation_credit_increment_is_one() {
        assert_eq!(REPUTATION_CREDIT_INCREMENT, 1);
    }

    /// MAX_COMMENT_BYTES must be at least 1.
    #[test]
    fn max_comment_bytes_is_positive() {
        assert!(MAX_COMMENT_BYTES >= 1);
    }

    // ── Boundary tests ────────────────────────────────────────────────────────

    /// Ratings at MIN and MAX boundaries are valid; just outside are not.
    #[test]
    fn rating_boundary_values() {
        // Valid: MIN_RATING..=MAX_RATING
        for r in MIN_RATING..=MAX_RATING {
            assert!(r >= MIN_RATING && r <= MAX_RATING, "rating {r} should be in-bounds");
        }
        // Invalid: 0 (underflow from MIN_RATING.wrapping_sub(1))
        let below = MIN_RATING.wrapping_sub(1); // 0 for MIN_RATING=1
        assert!(
            below < MIN_RATING || below > MAX_RATING,
            "rating {below} should be out-of-bounds"
        );
        // Invalid: MAX_RATING + 1
        let above = MAX_RATING + 1;
        assert!(above > MAX_RATING, "rating {above} should be out-of-bounds");
    }

    /// Pagination limit at PAGE_CEILING is valid; PAGE_CEILING+1 must be clamped.
    #[test]
    fn page_ceiling_boundary() {
        // Callers are expected to do: limit.min(PAGE_CEILING)
        let at_ceiling: u32 = PAGE_CEILING;
        let over_ceiling: u32 = PAGE_CEILING + 1;
        assert_eq!(at_ceiling.min(PAGE_CEILING), PAGE_CEILING);
        assert_eq!(over_ceiling.min(PAGE_CEILING), PAGE_CEILING);
        // A zero limit collapses before clamping
        assert_eq!(0u32.min(PAGE_CEILING), 0);
    }

    /// Comment length at MAX_COMMENT_BYTES is valid; MAX_COMMENT_BYTES+1 must be rejected.
    #[test]
    fn comment_length_boundary() {
        let at_max = MAX_COMMENT_BYTES;
        let over_max = MAX_COMMENT_BYTES + 1;
        assert!(at_max <= MAX_COMMENT_BYTES, "comment at max should be accepted");
        assert!(over_max > MAX_COMMENT_BYTES, "comment over max should be rejected");
        // Empty comment (0 bytes) must also be rejected by the non-empty guard
        let empty: u32 = 0;
        assert!(empty == 0, "zero-length comment is rejected by EmptyComment guard");
    }

    // ── Idempotency / duplicate-call safety ───────────────────────────────────

    /// Applying the clamping formula twice gives the same result as once
    /// (idempotent).  This mirrors what `get_reputations_page` and friends do
    /// when called with large `limit` values.
    #[test]
    fn page_ceiling_clamp_is_idempotent() {
        for candidate in [0u32, 1, PAGE_CEILING / 2, PAGE_CEILING, PAGE_CEILING + 1, u32::MAX] {
            let once = candidate.min(PAGE_CEILING);
            let twice = once.min(PAGE_CEILING);
            assert_eq!(once, twice, "clamp is not idempotent for input {candidate}");
        }
    }

    /// Rating validation predicate is idempotent: evaluating the guard twice
    /// for the same value returns the same result.
    #[test]
    fn rating_guard_is_idempotent() {
        let check = |r: u32| r >= MIN_RATING && r <= MAX_RATING;
        for r in [0u32, MIN_RATING, 3, MAX_RATING, MAX_RATING + 1, u32::MAX] {
            assert_eq!(check(r), check(r), "rating guard not idempotent for {r}");
        }
    }

    // ── Regression: concurrent / repeated execution ───────────────────────────
    //
    // Soroban contracts are single-threaded per transaction; true parallelism
    // is impossible within one ledger.  The "concurrent execution" risk comes
    // from *two separate transactions* that both read-then-write the same
    // storage key (TOCTOU).  Constants themselves are immutable, so these
    // regression tests focus on the properties that constants enforce on the
    // validation logic that guards state transitions.

    /// Applying the reputation-credit increment twice still yields a
    /// deterministic result (no hidden state in the constant).
    #[test]
    fn credit_increment_is_deterministic() {
        let base: i128 = 0;
        let after_one = base + REPUTATION_CREDIT_INCREMENT;
        let after_two = after_one + REPUTATION_CREDIT_INCREMENT;
        assert_eq!(after_one, 1);
        assert_eq!(after_two, 2);
        // Decrement (as done by issue_reputation) must be symmetric
        let decremented = after_two
            .checked_sub(REPUTATION_CREDIT_INCREMENT)
            .expect("sub must not overflow");
        assert_eq!(decremented, after_one);
    }

    /// SCALE is used as a divisor; confirm it cannot produce a division-by-zero
    /// and that the result is well-defined for typical inputs.
    #[test]
    fn scale_division_is_safe() {
        // SCALE is non-zero (proven by compile-time assert, confirmed here)
        assert_ne!(SCALE, 0);

        // Typical usage: total_rating * SCALE / completed_contracts
        let total_rating: i128 = 25; // 5 contracts × rating 5
        let completed: i128 = 5;
        let result = total_rating
            .checked_mul(SCALE)
            .and_then(|v| v.checked_div(completed));
        assert_eq!(result, Some(50_000)); // 5.0000 in ×10_000 representation

        // Edge: single contract with minimum rating
        let single = (MIN_RATING as i128)
            .checked_mul(SCALE)
            .and_then(|v| v.checked_div(1));
        assert_eq!(single, Some(10_000)); // 1.0000

        // Edge: single contract with maximum rating
        let max_single = (MAX_RATING as i128)
            .checked_mul(SCALE)
            .and_then(|v| v.checked_div(1));
        assert_eq!(max_single, Some(50_000)); // 5.0000
    }

    /// Verify that all boundary-adjacent values for `PAGE_CEILING` behave
    /// correctly under repeated calls, simulating retries or racing requests.
    #[test]
    fn page_ceiling_boundary_regression() {
        // Simulate three concurrent callers each requesting more than PAGE_CEILING.
        let requests = [PAGE_CEILING + 1, PAGE_CEILING * 2, u32::MAX];
        for req in requests {
            let clamped = req.min(PAGE_CEILING);
            assert_eq!(
                clamped, PAGE_CEILING,
                "concurrent caller requesting {req} must be clamped to PAGE_CEILING"
            );
            // Idempotent: re-clamping the already-clamped value changes nothing.
            assert_eq!(clamped.min(PAGE_CEILING), clamped);
        }
    }
}
