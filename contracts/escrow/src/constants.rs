/// Minimum valid reputation rating (inclusive).
pub const MIN_RATING: u32 = 1;

/// Maximum valid reputation rating (inclusive).
pub const MAX_RATING: u32 = 5;

/// Max byte length of a reputation feedback comment.
pub const MAX_COMMENT_BYTES: u32 = 200;

/// Unit increment for pending reputation credits.
pub const REPUTATION_CREDIT_INCREMENT: i128 = 1;

/// Basis-point scaling factor for `get_average_rating` (×10_000 preserves four decimal places).
pub const SCALE: i128 = 10_000;

/// Upper bound on the `limit` parameter of paginated read views.
///
/// Keeps per-call storage reads bounded and prevents callers from requesting
/// unbounded scans in a single invocation.
pub const PAGE_CEILING: u32 = 50;

/// -----------------------------------------------------------------------------
/// Compatibility contracts
/// -----------------------------------------------------------------------------
///
/// The constants above are part of the public compatibility contract of the
/// escrow contract. They are consumed by off-chain clients, indexers, and the
/// on-chain validation logic. The functions below make the invariants explicit
/// and deterministic so that valid, duplicate, boundary, and malformed inputs all
/// produce a reviewable, reproducible result. They do not change the values of
/// the constants, so existing callers remain compatible.

/// Returns `true` iff `rating` is within the inclusive range
/// `[MIN_RATING, MAX_RATING]`.
///
/// Invariant: for any `rating`, `is_valid_rating(rating)` equals
/// `MIN_RATING <= rating && rating <= MAX_RATING`. This is the single source of
/// truth for rating validation and must be used by all entry points that accept
/// a rating. Boundary values are inclusive.
pub const fn is_valid_rating(rating: u32) -> bool {
    rating >= MIN_RATING && rating <= MAX_RATING
}

/// Returns `true` iff the comment length is within the configured byte cap.
///
/// The length is measured in UTF-8 bytes to match the on-chain storage
/// representation. An empty comment is valid (length 0).
///
/// Invariant: `comment_len_bytes <= MAX_COMMENT_BYTES` iff this returns `true`.
pub const fn is_valid_comment_len(comment_len_bytes: u32) -> bool {
    comment_len_bytes <= MAX_COMMENT_BYTES
}

/// Clamps a requested page size to the inclusive range `[1, PAGE_CEILING]`.
///
/// A `requested` value of 0 is treated as a request for the minimum page size
/// (1), which preserves the existing behavior of the paginated read views while
/// ensuring that no caller can exceed the ceiling. Values above the ceiling are
/// clamped down to `PAGE_CEILING`.
///
/// Invariant: the returned value is always in `[1, PAGE_CELING`].
pub const fn clamp_page_limit(requested: u32) -> u32 {
    if requested == 0 {
        return 1;
    }
    if requested > PAGE_CEILING {
        return PAGE_CEILING;
    }
    requested
}

/// Computes the average rating in basis points (`SCALE`) from a running sum and
/// count, without losing precision and without panicking on empty input.
///
/// Returns the average rounded down to the nearest basis point. If `count` is
/// zero the function returns 0 rather than dividing by zero, so empty data is
/// deterministic and cannot cause an unrecoverable failure.
///
/// Invariant: `count == 0` implies return value 0. Otherwise the return value is
/// in `[0, MAX_RATING * SCALE]` for non-negative inputs.
pub const fn average_rating_scaled(sum: i128, count: i128) -> i128 {
    if count <= 0 {
        return 0;
    }
    sum / count
}

/// Applies the reputation credit increment exactly once per call.
///
/// This is the canonical way to accrue pending reputation credits. Because the
/// increment is a fixed positive value, repeated calls produce a deterministic,
/// monotonically increasing sequence and cannot overflow for realistic credit
/// counts. Callers must not apply the increment manually in addition to this
/// helper, otherwise credits would be double-counted.
///
/// Invariant: `apply_reputation_credit(current) == current + REPUTATION_CREDIT_INCREMENT`.
pub const fn apply_reputation_credit(current: i128) -> i128 {
    current + REPUTATION_CREDIT_INCREMENT
}

/// Returns the number of additional credits needed to reach `target` from
/// `current`, saturating at zero when the target is already met or exceeded.
///
/// This is useful for batched accrual and for diagnosing partial failure: a
/// caller can determine how many increments remain without mutating state.
///
/// Invariatue: the return value is always `>= 0`. If `current >= target` the
/// result is 0.
pub const fn credits_needed(current: i128, target: i128) -> i128 {
    if current >= target {
        return 0;
    }
    target - current
}
