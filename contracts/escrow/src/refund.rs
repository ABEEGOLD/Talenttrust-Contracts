// Refund entrypoints are implemented in `contracts/escrow/src/lib.rs`.
// This module retains refund-related helpers only.
//
// # State invariants (refund flow)
//
// The refund entrypoints in `lib.rs` MUST preserve the following invariants:
// - Authorization: only the escrow's designated refund authority (or the
//   contract's own admin path) may trigger a refund transition.
// - Idempotency: a refund may only transition an escrow from a refundable
//   state (e.g. `Funded`/`Active`) to `Refunded`; repeated or duplicate
//   calls on an already-`Refunded` escrow must be rejected deterministically.
// - Atomicity: balance/state updates and any emitted events must be applied
//   together; partial failure must not leave funds or state inconsistent.
// - Conservation: the sum of refunded amounts must never exceed the amount
//   originally escrowed for that record.
//
// Any helper added here must uphold these invariants and must not weaken
// validation performed by the entrypoints in `lib.rs`.
