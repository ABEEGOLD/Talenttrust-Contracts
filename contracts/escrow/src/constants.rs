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

/// Maximum number of distinct reputation records a caller may submit in a
/// single batch operation. Bounds workset size and keeps gas costs
/// deterministic across invocations.
pub const MAX_BATCH_SIZE: u32 = 25;

/// Maximum number of bytes allowed for a single address identifier key.
pub const MAX_ADDRESS_BYTES: u32 = 64;

/// Minimum valid escrow amount (inclusive), in strokes.
/// Prevents zero-value deposits that would create unrecoverable state.
pub const MIN_DEPOSIT_AMOUNT: i128 = 1;

/// Maximum valid escrow amount (inclusive), in strokes.
/// Guards against overflow when accumulating balances or computing fees.
pub const MAX_DEPOSIT_AMOUNT: i128 = i128::__MAX__;
