# Contracts Storage Layout & TTL Policy

> **Validation boundaries.**  This document is the authoritative reference
> for the *validation boundaries* of every storage key in the escrow
> contract: which inputs are accepted, which are rejected, how duplicates
> are handled, and what happens at boundary values.  See §10 for the
> consolidated boundary table and the invariants that must hold.

This document describes the on-chain storage layout used by the TalentTrust
escrow contract on Soroban: every storage key, its value shape, which Soroban
storage type it lives in (`persistent` vs `temporary`), and the deterministic
TTL / bump strategy that governs its lifetime.

All values are Soroban `#[contracttype]` types or primitives defined in
[`contracts/escrow/src/types.rs`](../contracts/escrow/src/types.rs).  TTL
constants and helpers live in
[`contracts/escrow/src/ttl.rs`](../contracts/escrow/src/ttl.rs).  The
canonical `DataKey` enum is defined in
[`types.rs#L59-L93`](../contracts/escrow/src/types.rs#L59-L93).

Every key below is subject to the validation boundaries enumerated in §10.
Where a key has no explicit boundary, the default is: **absent ⇒ fail-closed
with the documented error; present ⇒ validated against the type and range
constraints in §10 before use.**

---

## 1. Storage Types at a Glance

| Soroban storage kind | Used for | Eviction model |
|---|---|---|
| `env.storage().persistent()` | Contract state, accounting records, governance config, reputation, finalization, settlement-token binding | Manual TTL extension; evicted by the host after `PERSISTENT_TTL_LEDGERS` without a renewing access |
| `env.storage().temporary()` | Pending milestone approvals, pending client migrations | Auto-evicted by the host as soon as their TTL elapses; no on-chain eviction event |
| `env.storage().instance()` | (not used directly by the escrow; reserved for contract-level metadata) | — |

The contract never writes to `instance()` storage for its own records.

---

## 2. Unit Conversions

All TTL constants are denominated in **ledgers** (the native Soroban expiry
unit).  On Stellar mainnet one ledger closes roughly every 5 seconds.  The
conversion factor used everywhere is `LEDGERS_PER_DAY = 17 280`.

| Name | Ledgers | Approximate wall-clock |
|---|---:|---|
| `LEDGERS_PER_DAY` | 17 280 | 1 day |
| `PENDING_APPROVAL_TTL_LEDGERS` | 120 960 | 7 days |
| `PENDING_APPROVAL_BUMP_THRESHOLD` | 17 280 | 1 day |
| `PENDING_MIGRATION_TTL_LEDGERS` | 362 880 | 21 days |
| `PENDING_MIGRATION_BUMP_THRESHOLD` | 51 840 | 3 days |
| `PERSISTENT_TTL_LEDGERS` | 518 400 | 30 days |
| `PERSISTENT_BUMP_THRESHOLD` | 120 960 | 7 days |
| `ADMIN_ROTATION_MIN_DELAY_LEDGERS` | 34 560 | 2 days (timelock, **not** a storage TTL) |

Reference:
[`ttl.rs#L45-L61`](../contracts/escrow/src/ttl.rs#L45-L61).

---

## 3. Persistent Storage Keys

Each entry below lists: the key expression, the Rust value type, a short
description, the TTL renew strategy, and a code pointer that performs the
write or the canonical read.

### 3.1 Initialization & Admin

| Key | Value type | Description | TTL bump? | Write site |
|---|---|---|---|---|
| `DataKey::Initialized` | `bool` | Flipped to `true` exactly once by `initialize`.  Absent means the contract is not yet initialized. | Never bumped (effectively immortal because it is only read in guards and never written after init). | [`lib.rs#L367-L378`](../contracts/escrow/src/lib.rs#L367-L378) |
| `DataKey::Admin` | `Address` | Operational admin address.  Authorizes pause/emergency, protocol fees, governed parameters, settlement-token binding, admin rotation, and fee withdrawal.  Set during `initialize` and rotated via the two-step `PendingAdmin` proposal. | Never bumped explicitly; read on every admin-gated call, so in practice it is always hot. | [`lib.rs#L376-L378`](../contracts/escrow/src/lib.rs#L376-L378), [`governance.rs#L124-L133`](../contracts/escrow/src/governance.rs#L124-L133) |
| `DataKey::PendingAdmin` | `PendingAdminProposal { proposed: Address, proposed_at_ledger: u32 }` | Two-step admin-rotation proposal.  Cleared on accept or cancel.  A proposal must age at least `ADMIN_ROTATION_MIN_DELAY_LEDGERS` before it can be accepted (timelock enforced at accept time, not via storage TTL). | Never bumped; acceptance gate reads `proposed_at_ledger` and compares with the current sequence. | [`governance.rs#L85-L91`](../contracts/escrow/src/governance.rs#L85-L91), [`governance.rs#L107-L133`](../contracts/escrow/src/governance.rs#L107-L133) |

**Validation boundaries (initialization & admin).**
- `initialize` is idempotent-rejecting: a second call while
  `Initialized == true` panics with `AlreadyInitialized`.  Duplicate
  initialization is therefore impossible.
- `Admin` must be a valid `Address`; the zero/invalid address is rejected
  by the host's `Address` deserialization before any write.
- `PendingAdmin.proposed` must differ from the current `Admin`; proposing
  the same address is rejected with `InvalidState`.  A second proposal
  while one is pending overwrites the slot only if the previous proposal
  has already been accepted or cancelled — otherwise it panics with
  `InvalidState` (single-slot invariant).
- `PendingAdmin.proposed_at_ledger` is a `u32` ledger sequence; the
  timelock check uses `saturating_sub` so a boundary value of `0` is
  treated as "not yet aged" and rejected until the delay elapses.

### 3.2 Pause & Emergency

| Key | Value type | Description | TTL bump? | Write site |
|---|---|---|---|---|
| `DataKey::Paused` | `bool` | Normal operational pause.  When `true` every *mutating* entrypoint panics with `ContractPaused`; read-only queries still succeed.  `unpause` clears it; `activate_emergency_pause` *also* sets it. | Never bumped. | [`lib.rs#L1428-L1465`](../contracts/escrow/src/lib.rs#L1428-L1465) |
| `DataKey::Emergency` | `bool` | Emergency freeze.  When `true` the same mutation gate fires `EmergencyActive` and `unpause` itself is blocked; only `resolve_emergency` clears both `Emergency` and `Paused`.  Flipping `Emergency` on once also sets `ReadinessChecklist::emergency_controls_enabled = true` permanently so deployers can prove they tested the emergency circuit. | Never bumped. | [`lib.rs#L1486-L1566`](../contracts/escrow/src/lib.rs#L1486-L1566) |

**Validation boundaries (pause & emergency).**
- `pause` is idempotent-rejecting: calling it while `Paused == true`
  panics with `ContractPaused`.  Calling `unpause` while `Paused == false`
  panics with `ContractNotPaused`.
- `activate_emergency_pause` while `Emergency == true` panics with
  `EmergencyActive` (duplicate activation rejected).
- `resolve_emergency` while `Emergency == false` panics with
  `EmergencyNotActive` (no-op rejected).
- Both flags are `bool`; no numeric boundary applies.  The mutation gate
  is evaluated *before* any storage touch, so a rejected call cannot
  mutate state.

### 3.3 Contracts & Milestones

| Key | Value type | Description | TTL bump? | Write / load site |
|---|---|---|---|---|
| `DataKey::NextContractId` | `u32` | Monotonic allocator.  Starts at 1 after `initialize`; incremented after every successful `create_contract`.  Reads are cheap and do **not** extend TTL on `get_next_contract_id`; only the creation path calls `extend_next_contract_id_ttl` before touching it. | `PERSISTENT_BUMP_THRESHOLD` → `PERSISTENT_TTL_LEDGERS`, only from `create_contract`. | [`ttl.rs#L160-L168`](../contracts/escrow/src/ttl.rs#L160-L168), [`create_contract.rs#L115-L166`](../contracts/escrow/src/create_contract.rs#L115-L166) |
| `DataKey::Contract(contract_id: u32)` | [`Contract`](../contracts/escrow/src/types.rs#L213-L226) struct (`client`, `freelancer`, `arbiter: Option<Address>`, `status: ContractStatus`, `total_deposited`, `funded_amount`, `released_amount`, `refunded_amount`, `release_authorization: ReleaseAuthorization`, `reputation_issued: bool`) | Core accounting + lifecycle record for escrow `contract_id`.  All money-moving entrypoints read-then-write this key. | Bumped to `PERSISTENT_TTL_LEDGERS` (threshold = `PERSISTENT_BUMP_THRESHOLD`) on every read or write via `extend_contract_ttl`.  Exceptions: `contract_exists` is a pure existence probe and deliberately does **not** bump TTL, to prevent keep-alive abuse. | [`create_contract.rs#L136-L138`](../contracts/escrow/src/create_contract.rs#L136-L138), [`lib.rs#L1202-L1212`](../contracts/escrow/src/lib.rs#L1202-L1212), [`ttl.rs#L171-L177`](../contracts/escrow/src/ttl.rs#L171-L177) |
| `(DataKey::Contract(contract_id), Symbol::new(env, "milestones"))` | `Vec<`[`Milestone`](../contracts/escrow/src/types.rs#L228-L241)`>` (each: `amount`, `funded_amount`, `released: bool`, `refunded: bool`, `work_evidence: Option<String>`, `refunded_amount`, `deadline: Option<u64>`) | **Compound tuple key**, *not* a `DataKey` variant.  Stores the ordered milestone vector.  `Milestone.released` / `Milestone.refunded` flags are the single source of truth; the declared `DataKey::MilestoneReleased(u32, u32)` variant is **never written** (see §5). | Bumped whenever the vector is loaded or stored via `load_milestones` / `store_milestones` / `extend_milestone_ttl`.  The same `PERSISTENT_BUMP_THRESHOLD → PERSISTENT_TTL_LEDGERS` policy applies. | [`ttl.rs#L134-L186`](../contracts/escrow/src/ttl.rs#L134-L186), [`create_contract.rs#L140-L156`](../contracts/escrow/src/create_contract.rs#L140-L156) |

**Validation boundaries (contracts & milestones).**
- `contract_id` is a `u32` allocated by `NextContractId`.  `0` is never
  allocated (the counter starts at `1` after `initialize`), so
  `DataKey::Contract(0)` is always absent and any read of it panics with
  `ContractNotFound`.  The maximum representable id is `u32::MAX`; the
  allocator uses `checked_add` and panics with `Overflow` rather than
  wrapping, so the boundary is a hard stop, not a silent rollover.
- `NextContractId` starts at `1`; `initialize` writes `1`, and
  `create_contract` increments it *after* a successful write of the new
  `Contract(id)` record.  A failed `create_contract` therefore does not
  consume an id (no gaps from partial failure).
- Milestone vector: `len == 0` is rejected at `create_contract` time with
  `InvalidMilestones`; `len > MAX_MILESTONES` is rejected with
  `TooManyMilestones`.  Each `Milestone.amount` must be `> 0`; a zero or
  negative amount is rejected with `InvalidAmount`.  `funded_amount` and
  `released_amount` are bounded by `amount`; exceeding them is rejected
  with `Overflow`.
- `Milestone.released` / `Milestone.refunded` are mutually exclusive: a
  milestone that is already `released == true` cannot be refunded, and
  vice versa.  Duplicate release/refund attempts panic with
  `AlreadyReleased` / `AlreadyRefunded`.
- `Milestone.deadline` is `Option<u64>`; `None` means "no deadline".  A
  `Some(0)` deadline is treated as already-expired and rejected by the
  deadline gate with `DeadlinePassed`.

#### `ContractStatus` enum (written inside `Contract.status`)

```
Created = 0 → Accepted = 1 → Funded / PartiallyFunded = 2 / 7 → Completed = 3
                                                     ↘ Disputed = 4  ↗
                                            Cancelled = 5 / Refunded = 6 (terminal)
```

Defined at [`types.rs#L199-L210`](../contracts/escrow/src/types.rs#L199-L210).

### 3.4 Governance & Protocol Fees

| Key | Value type | Description | TTL bump? | Write site |
|---|---|---|---|---|
| `DataKey::ProtocolFeeBps` | `u32` | Release fee in basis points.  Defaults to `0` (no fee).  Max `10 000` (= 100 %).  Overridden atomically by `set_governed_params` which writes `GovernedParameters` instead; both keys are consulted. | Never bumped explicitly. | [`governance.rs#L32-L55`](../contracts/escrow/src/governance.rs#L32-L55) |
| `DataKey::GovernedParameters` | [`GovernedParameters { protocol_fee_bps: u32, max_escrow_total_stroops: i128 }`](../contracts/escrow/src/types.rs#L299-L304) | Canonical combined governance record.  Setting it via `set_governed_params` also flips `ReadinessChecklist::governed_params_set = true` to mark the deploy step complete. | Never bumped explicitly. | [`governance.rs#L200-L249`](../contracts/escrow/src/governance.rs#L200-L249) |
| `DataKey::AccumulatedProtocolFees` | `i128` | Running total of protocol fees retained inside the SAC balance, accrued on each `release_milestone`.  Drained by `withdraw_protocol_fees`.  Because fees are commingled with the escrow balance in the SAC token, this counter is the authoritative record of how much is owed to the protocol vs owed to counterparties. | Bumped on write in `withdraw_protocol_fees` using the persistent policy. | [`lib.rs#L849-L854`](../contracts/escrow/src/lib.rs#L849-L854), [`lib.rs#L2036-L2060`](../contracts/escrow/src/lib.rs#L2036-L2060) |

**Validation boundaries (governance & fees).**
- `ProtocolFeeBps` must satisfy `0 <= bps <= 10_000`.  A value above
  `10_000` is rejected with `InvalidFeeBps`; the boundary `10_000`
  (100 %) is accepted.
- `GovernedParameters.protocol_fee_bps` follows the same `[0, 10_000]`
  range.  `max_escrow_total_stroops` must be `>= 0`; a negative value is
  rejected with `InvalidAmount`.  `0` is accepted and means "no escrow
  total cap".
- `AccumulatedProtocolFees` is an `i128` that must never go negative.
  `withdraw_protocol_fees` rejects an amount greater than the accumulated
  balance with `InsufficientFees`; withdrawing exactly the balance is
  accepted and leaves the counter at `0`.
- `set_governed_params` is idempotent-safe: re-setting the same values is
  accepted (no duplicate-rejection), but the `ReadinessChecklist` flag is
  only flipped once.

### 3.5 Settlement-Token Custody

| Key | Value type | Description | TTL bump? | Write site |
|---|---|---|---|---|
| `DataKey::SettlementToken` | `Address` | Write-once SAC token address bound by `bind_settlement_token`.  All `deposit_funds`, `release_milestone`, `refund_*`, `cancel_contract`, and `withdraw_protocol_fees` paths perform `token::Client::transfer` against this address; absence of the binding panics with `SettlementTokenNotConfigured`. | Never bumped; read-only getters (`get_settlement_token`, `is_settlement_token_bound`) also do not extend TTL. | [`lib.rs#L182-L187`](../contracts/escrow/src/lib.rs#L182-L187), [`lib.rs#L256-L313`](../contracts/escrow/src/lib.rs#L256-L313) |

**Validation boundaries (settlement token).**
- Write-once: a second `bind_settlement_token` call panics with
  `SettlementTokenAlreadyBound`.  Duplicate binding is therefore
  impossible, and the token address cannot be silently swapped.
- The bound address must be a valid `Address`; the host rejects malformed
  addresses before any write.
- All money-moving paths fail-closed with `SettlementTokenNotConfigured`
  when the key is absent, so an unbound contract cannot move funds.

### 3.6 Finalization (Immutable Close Records)

| Key | Value type | Description | TTL bump? | Write site |
|---|---|---|---|---|
| `DataKey::Finalization(contract_id: u32)` | [`FinalizationRecord { finalizer: Address, timestamp: u64, summary: ContractSummary }`](../contracts/escrow/src/finalize.rs#L13-L22) | Immutable snapshot written when a participant closes a `Completed` or `Disputed` contract.  Once written, every contract-specific mutating entrypoint fails `require_not_finalized` with `AlreadyFinalized`. | Not bumped explicitly; written once and typically read shortly thereafter. | [`finalize.rs#L140-L168`](../contracts/escrow/src/finalize.rs#L140-L168) |

**Validation boundaries (finalization).**
- Write-once per `contract_id`: a second `finalize` call panics with
  `AlreadyFinalized`.  Duplicate finalization is therefore impossible.
- Only a participant (`client`, `freelancer`, or `arbiter`) may finalize;
  a non-participant is rejected with `Unauthorized`.
- Finalization is only allowed when `Contract.status` is `Completed` or
  `Disputed`; any other status is rejected with `InvalidState`.

### 3.7 Readiness Checklist

| Key | Value type | Description | TTL bump? | Write site |
|---|---|---|---|---|
| `DataKey::ReadinessChecklist` | [`ReadinessChecklist { initialized: bool, governed_params_set: bool, emergency_controls_enabled: bool }`](../contracts/escrow/src/types.rs#L277-L297) | Three-bit progress tracker for mainnet-deploy QA.  Each flag is flipped by the entrypoint that performs the corresponding step: `initialize`, `set_governed_params`, and `activate_emergency_pause` (the latter is sticky once flipped). | Never bumped. | [`lib.rs#L383-L391`](../contracts/escrow/src/lib.rs#L383-L391), [`governance.rs#L238-L246`](../contracts/escrow/src/governance.rs#L238-L246), [`lib.rs#L1504-L1512`](../contracts/escrow/src/lib.rs#L1504-L1512) |

**Validation boundaries (readiness checklist).**
- Each flag is monotonic: once `true`, it can never be flipped back to
  `false`.  Re-flipping an already-`true` flag is a no-op (idempotent),
  not an error.
- The checklist is never a gate on its own; it is a QA signal.  Missing
  entries are treated as all-`false` (fail-closed for deploy tooling).

### 3.8 Reputation

| Key | Value type | Description | TTL bump? | Write site |
|---|---|---|---|---|
| `DataKey::ReputationIssued(contract_id: u32)` | `bool` | Per-contract "already issued" guard.  Redundantly tracks `Contract.reputation_issued`; both are consulted in the summary path.  Written together with the reputation counters in `issue_reputation`. | Bumped at write-time in `issue_reputation` using the persistent policy. | [`lib.rs#L1724-L1735`](../contracts/escrow/src/lib.rs#L1724-L1735) |
| `DataKey::PendingReputationCredits(freelancer: Address)` | `i128` | Counter of completed contracts awaiting a client rating.  Incremented by `grant_pending_reputation_credit` (on final milestone release or dispute completion); decremented by exactly `1` per `issue_reputation` call.  Refunded contracts never grant a credit. | Not bumped explicitly; read/written without TTL extension. | [`lib.rs#L625-L629`](../contracts/escrow/src/lib.rs#L625-L629), [`lib.rs#L1737-L1742`](../contracts/escrow/src/lib.rs#L1737-L1742) |
| `DataKey::Reputation(freelancer: Address)` | [`Reputation { completed_contracts: i128, total_rating: i128, last_rating: i128 }`](../contracts/escrow/src/types.rs#L318-L324) | Aggregate counters per freelancer.  `get_average_rating` returns `(total_rating * 10_000 / completed_contracts)` when `completed_contracts > 0`; `None` otherwise. | Not bumped explicitly. | [`lib.rs#L1744-L1750`](../contracts/escrow/src/lib.rs#L1744-L1750), [`lib.rs#L1778-L1811`](../contracts/escrow/src/lib.rs#L1778-L1811) |
| `DataKey::ReputationComment(contract_id: u32)` | `String` (max 200 UTF-8 bytes) | Client-supplied free-form feedback written by `issue_reputation`.  Capped at 200 bytes to cap storage growth; validated at write time by `EmptyComment` / `CommentTooLong`. | Bumped at write-time in `issue_reputation` and on read in `get_reputation_comment` using the persistent policy. | [`lib.rs#L1752-L1758`](../contracts/escrow/src/lib.rs#L1752-L1758), [`lib.rs#L1765-L1776`](../contracts/escrow/src/lib.rs#L1765-L1776) |

**Validation boundaries (reputation).**
- `ReputationIssued(contract_id)` is a write-once guard: a second
  `issue_reputation` for the same `contract_id` panics with
  `ReputationAlreadyIssued`.  Duplicate issuance is therefore impossible.
- `PendingReputationCredits(freelancer)` must never go negative.
  `issue_reputation` decrements by exactly `1` and rejects with
  `NoPendingCredit` when the counter is `0` (boundary: `0` is the floor).
- `Reputation.completed_contracts` must be `>= 0`; `total_rating` must be
  `>= 0`; `last_rating` must be in `[0, 10_000]` (basis points).  A rating
  outside that range is rejected with `InvalidRating`.
- `ReputationComment` must be non-empty and at most `200` UTF-8 bytes.
  `""` is rejected with `EmptyComment`; `> 200` bytes is rejected with
  `CommentTooLong`.  Exactly `200` bytes is accepted (inclusive upper
  bound).
- `get_average_rating` returns `None` when `completed_contracts == 0`
  (boundary: division by zero is avoided, not panicked).

---

## 4. Temporary Storage Keys (TTL-governed, auto-evicting)

Everything in this section lives in `env.storage().temporary()` and is
subject to Soroban host auto-eviction.  The contract consistently treats a
missing / evicted entry as "not approved / not migrated" (fail-closed).

### 4.1 Pending Milestone Approvals

| Key | Value type | Description | TTL | Bump threshold |
|---|---|---|---|---|
| `DataKey::MilestoneApprovals(contract_id: u32, milestone_index: u32)` | [`MilestoneApprovals { client_approved: bool, freelancer_approved: bool, arbiter_approved: bool }`](../contracts/escrow/src/types.rs#L259-L266) | Bitmask of which parties have pre-approved a given milestone for release.  Required approvers depend on `Contract.release_authorization`: `ClientOnly`, `ClientAndArbiter`, `ArbiterOnly`, or `MultiSig` (client **and** freelancer).  Cleared explicitly by `clear_approvals` after a successful release. | 7 d = `PENDING_APPROVAL_TTL_LEDGERS` | 1 d = `PENDING_APPROVAL_BUMP_THRESHOLD` |

**Validation boundaries (pending approvals).**
- `contract_id` must refer to an existing `Contract`; otherwise the write
  panics with `ContractNotFound`.
- `milestone_index` must be `< milestones.len()`; an out-of-range index is
  rejected with `InvalidMilestoneIndex`.  The boundary `len - 1` is the
  last valid index; `len` is rejected.
- Duplicate approvals from the same role are rejected with
  `AlreadyApproved`.  Approvals from different roles accumulate in the
  same record; the record is cleared only after a successful release.
- An evicted (TTL-expired) approval record is treated as "not approved"
  (fail-closed); see §7.

- **Write path:** `approve_milestone` in
  [`approvals.rs#L46-L159`](../contracts/escrow/src/approvals.rs#L46-L159)
  calls `.temporary().set` then `.temporary().extend_ttl(threshold, ttl)`.
  Duplicate approvals from the same role return `AlreadyApproved`.
- **Bump-on-read:** `get_milestone_approvals` renews TTL when the entry is
  live; missing entries return `None` without writing.  See
  [`lib.rs#L1388-L1403`](../contracts/escrow/src/lib.rs#L1388-L1403).
- **Check path:** `check_approvals` in
  [`approvals.rs#L180-L212`](../contracts/escrow/src/approvals.rs#L180-L212)
  performs a plain `.get`; any `None` → `InsufficientApprovals` fail-closed.
- **Explicit removal:** `clear_approvals` after successful release
  ([`approvals.rs#L222-L225`](../contracts/escrow/src/approvals.rs#L222-L225)).

### 4.2 Pending Client Migrations

| Key | Value type | Description | TTL | Bump threshold |
|---|---|---|---|---|
| `DataKey::PendingClientMigration(contract_id: u32)` | [`PendingClientMigration { current_client: Address, proposed_client: Address, requested_at_ledger: u32, expires_at_ledger: u32 }`](../contracts/escrow/src/migration.rs#L5-L12) | Single-slot proposal to transfer the `client` role on a contract to a new address.  At most one proposal may be pending per contract; re-proposing panics with `InvalidState`.  Migrations are disallowed on `Completed`, `Cancelled`, `Refunded`, or `Disputed` contracts. | 21 d = `PENDING_MIGRATION_TTL_LEDGERS` | 3 d = `PENDING_MIGRATION_BUMP_THRESHOLD` |

**Validation boundaries (pending migrations).**
- Single-slot per `contract_id`: a second `propose_client_migration`
  while one is pending panics with `InvalidState`.  Duplicate proposals
  are therefore impossible.
- `proposed_client` must differ from `current_client`; proposing the same
  address is rejected with `InvalidState`.
- `contract_id` must refer to a contract whose status is not `Completed`,
  `Cancelled`, `Refunded`, or `Disputed`; otherwise the proposal is
  rejected with `InvalidState`.
- `expires_at_ledger` is informational; the authoritative TTL is the
  host-level one set by `store_with_ttl`.  An evicted proposal is treated
  as "no migration" (fail-closed); see §7.

- **Write path:** `propose_client_migration_impl` in
  [`migration.rs#L48-L90`](../contracts/escrow/src/migration.rs#L48-L90)
  writes via `ttl::store_with_ttl`.  `expires_at_ledger` in the struct is
  informational (for indexers); the authoritative TTL is the host-level one
  set by `store_with_ttl`.
- **Read path:** `read_if_live` wraps `.temporary().get`; `None` is treated
  as "no pending migration" whether due to eviction or to never being set.
  See [`migration.rs#L105-L125`](../contracts/escrow/src/migration.rs#L105-L125)
  and
  [`migration.rs#L156-L168`](../contracts/escrow/src/migration.rs#L156-L168).
- **Explicit removal:** `cancel_client_migration` via
  `ttl::remove_transient`
  ([`migration.rs#L131-L155`](../contracts/escrow/src/migration.rs#L131-L155)).

---

## 5. DataKey Variants Declared but **Not** Written

The `DataKey` enum declares the following variants that, as of this writing,
have no storage write site in the contract.  They are listed here so an
indexer does not expect them on-chain.

| Variant | Declared at | Status | Single source of truth instead |
|---|---|---|---|
| `DataKey::MilestoneReleased(u32, u32)` | [`types.rs#L70`](../contracts/escrow/src/types.rs#L70) | Never persisted.  Verified by the storage test comment in [`test/storage.rs#L272-L273`](../contracts/escrow/src/test/storage.rs#L272-L273) and again in [`test/summary.rs#L179`](../contracts/escrow/src/test/summary.rs#L179). | Each `Milestone.released` / `refunded` boolean inside the milestone vector compound key (§3.3). |
| `DataKey::GovernanceAdmin` | [`types.rs#L80`](../contracts/escrow/src/types.rs#L80) | Never used; superseded by `DataKey::Admin` during the initial implementation. | `DataKey::Admin`. |
| `DataKey::PendingGovernanceAdmin` | [`types.rs#L81`](../contracts/escrow/src/types.rs#L81) | Never used; superseded by `DataKey::PendingAdmin`. | `DataKey::PendingAdmin`. |
| `DataKey::ProtocolParameters` | [`types.rs#L82`](../contracts/escrow/src/types.rs#L82) | Never used; the combined-parameters struct lives under `GovernedParameters` and the legacy BPS value under `ProtocolFeeBps`. | `DataKey::GovernedParameters` + `DataKey::ProtocolFeeBps`. |

**Validation boundary for unwritten variants.**  Because these variants
are never written, any read of them returns `None` (or panics with the
documented error for the corresponding entrypoint).  Indexers must not
expect them on-chain.  If a future change starts writing one of them, it
must add a row to §3 or §4 and a boundary entry to §10 before landing.

---

## 6. TTL / Bump Strategy Summary

### 6.1 Persistent entries: 30-day renew on access

Every frequently-accessed persistent key is extended using the same two
constants via `extend_ttl(key, PERSISTENT_BUMP_THRESHOLD, PERSISTENT_TTL_LEDGERS)`:

- If remaining TTL < 7 days (120 960 ledgers): extend to 30 days.
- Otherwise: no-op (Soroban `extend_ttl` never shortens).

Keys that receive this treatment from the dedicated helpers in
[`ttl.rs#L171-L199`](../contracts/escrow/src/ttl.rs#L171-L199):

| Helper | Target key |
|---|---|
| `extend_contract_ttl(contract_id)` | `DataKey::Contract(contract_id)` |
| `extend_milestone_ttl(contract_id)` | `(DataKey::Contract(contract_id), "milestones")` — via `milestone_storage_key` |
| `extend_contract_and_milestones_ttl(contract_id)` | Both above in one call |
| `extend_next_contract_id_ttl()` | `DataKey::NextContractId` |
| `extend_participant_contract_index_ttl(&key)` | Any participant contract-index `DataKey` (currently wired through the helper but the concrete index keys are reserved for a future list API) |

Call-site TTL extensions:

- `ReputationIssued(contract_id)` — bumped inline in `issue_reputation`.
- `ReputationComment(contract_id)` — bumped inline in `issue_reputation` and `get_reputation_comment`.
- `AccumulatedProtocolFees` — bumped inline in `withdraw_protocol_fees`.

**Eviction risk:** Any single persistent entry that goes untouched for more
than `PERSISTENT_TTL_LEDGERS` (≈ 30 days) will be evicted by the Soroban
host.  Because the contract reads `Contract(id)` / milestones together,
active contracts stay hot; the deliberate design choice is that *inactive*
contracts and their associated records are archived automatically by the
network rather than persisting forever.  If the milestone vector is evicted
but the `Contract(id)` record is not, `load_milestones` still panics with
`ContractNotFound`, so callers observe a consistent "contract gone" state.

### 6.2 Temporary entries: bump on access within threshold

| Entry family | Full TTL | Bump threshold | Behavior below threshold |
|---|---:|---:|---|
| Milestone approvals (`MilestoneApprovals`) | 7 d | 1 d | On `approve_milestone` write, `get_milestone_approvals` read, and — via the host `extend_ttl(threshold, ttl)` semantics — whenever a read/write occurs inside the last day.  Outside the threshold, reads still succeed but do not extend. |
| Client migrations (`PendingClientMigration`) | 21 d | 3 d | Same semantics via `store_with_ttl` and `extend_if_below_threshold`.  Reads use `read_if_live`, which itself does **not** bump; explicit bump calls are placed in the acceptance / cancellation paths where needed. |

### 6.3 Helper API (from `ttl.rs`)

| Helper | Storage kind | Description |
|---|---|---|
| `compute_expiry(env, ttl_ledgers)` | pure | `sequence.saturating_add(ttl_ledgers)` — used by off-chain-facing deadline getters. |
| `store_with_ttl(env, key, value, ttl)` | temporary | `.set` + `.extend_ttl(ttl, ttl)` in one call. |
| `read_if_live::<V>(env, key) -> Option<V>` | temporary | Thin wrapper around `.get`.  `None` covers both "absent" and "evicted". |
| `extend_if_below_threshold(env, key, threshold, extend_to) -> bool` | temporary | Returns `false` when the key is absent / evicted; otherwise performs the thresholded extend.  The boolean reports **liveness**, not whether the host actually performed an extension. |
| `remove_transient(env, key)` | temporary | Idempotent `.remove`. |
| `has_transient(env, key) -> bool` | temporary | `.has` proxy; returns `false` after eviction just as it does for a never-set key. |
| `load_milestones(env, id) -> Vec<Milestone>` | persistent | `.get` (panics with `ContractNotFound` on absent) then `extend_milestone_ttl`. |
| `store_milestones(env, id, milestones)` | persistent | `.set` then `extend_milestone_ttl`. |
| `milestone_storage_key(env, id)` | pure | Returns the compound `(DataKey::Contract(id), Symbol("milestones"))` tuple. |
| `extend_*_ttl(...)` helpers listed in §6.1 | persistent | Consistent persistent-policy wrappers. |

Reference:
[`ttl.rs#L64-L199`](../contracts/escrow/src/ttl.rs#L64-L199).

---

## 7. Fail-Closed Semantics

The following security-relevant guarantees arise directly from the storage
layout:

1. **Missing or evicted approval ≠ not approved.** `release_milestone`
   calls `approvals::check_approvals`, which `.get`s the temporary record;
   `None` maps to `InsufficientApprovals` (see
   [`approvals.rs#L186-L211`](../contracts/escrow/src/approvals.rs#L186-L211)).
   An approval whose TTL expires between the `approve_*` and
   `release_milestone` calls therefore cannot be reused — the caller must
   re-approve.

2. **Missing or evicted migration ≠ no migration.**
   `accept_client_migration_impl` and `get_pending_client_migration_impl`
   use `read_if_live`; `None` panics with `InvalidState`, preventing a
   stale (evicted) proposal from being accepted and preventing a caller
   from reading a phantom record.

3. **Contract absence ≠ present data.** Every mutating entrypoint loads
   `Contract(id)` via `.get().unwrap_or_else(|| panic_with_error(ContractNotFound))`.
   The single exception is `contract_exists`, which is a pure `has()` probe
   that deliberately avoids bumping TTL so it cannot be abused as a
   keep-alive mechanism.

4. **`require_not_finalized` + `require_not_paused` gate state mutation
   before any storage touch.** See
   [`finalize.rs#L36-L65`](../contracts/escrow/src/finalize.rs#L36-L65) for
   both guards — they run before auth in every lifecycle path.

---

## 8. Storage Access & TTL Tests

| Test module | What it covers |
|---|---|
| [`test/storage.rs`](../contracts/escrow/src/test/storage.rs) | Per-key existence / correctness for `Initialized`, `Admin`, `Paused`, `Emergency`, `Contract(id)`, `NextContractId`, milestone vectors (and the `MilestoneReleased` no-write assertion), `ReputationIssued`, `PendingReputationCredits`, `Reputation`, `ReadinessChecklist`, released-amount accounting, and single-index milestone getters. |
| [`test/ttl_tests.rs`](../contracts/escrow/src/test/ttl_tests.rs) | TTL constants, `compute_expiry` (including saturating), `store_with_ttl`, `read_if_live`, eviction at +1 ledger, `extend_if_below_threshold` liveness boolean, exact-threshold no-op, `remove_transient` idempotency, `has_transient` tracking, determinism across independent envs, and integration of approval TTL with `approve_milestone` / `check_approvals`. |
| [`test/approval_expiry.rs`](../contracts/escrow/src/test/approval_expiry.rs) | Approval-expiry invariants for each `ReleaseAuthorization` mode. |
| [`test/persistence.rs`](../contracts/escrow/src/test/persistence.rs) | Absent-state read behavior across multiple lifecycle readers. |
| [`test/participant_index_pagination.rs`](../contracts/escrow/src/test/participant_index_pagination.rs) | Pagination behavior for the future `list_contracts_by_participant` indexer API (uses the `extend_participant_contract_index_ttl` helper wired in `ttl.rs`). |

### 8.1 Validation-boundary tests

The following tests are required for every key that has an explicit
boundary in §10.  They are grouped by scenario so a reviewer can map each
acceptance criterion to a concrete test:

| Scenario | Test location | What it asserts |
|---|---|---|
| Accepted input | `test/storage.rs`, `test/ttl_tests.rs` | The documented valid range is written and read back unchanged. |
| Rejected input | `test/storage.rs`, `test/approval_expiry.rs` | Out-of-range / malformed input panics with the documented error and does **not** mutate state. |
| Duplicate submission | `test/storage.rs`, `test/persistence.rs` | A second write of a write-once key panics with the documented duplicate error; the original value is unchanged. |
| Boundary values | `test/ttl_tests.rs`, `test/storage.rs` | The inclusive lower/upper bounds are accepted; the first value outside the bound is rejected. |
| Regression | `test/persistence.rs`, `test/summary.rs` | Previously-fixed boundary bugs remain fixed (e.g. `MilestoneReleased` never written, `contract_exists` does not bump TTL). |

---

## 9. Reviewer Checklist for Storage Changes

When introducing a new storage key, make sure all of the following are
addressed before landing:

1. Add the variant to `DataKey` in `types.rs`, or use a compound tuple key
   if the key depends on a sub-identifier (e.g. the milestone vector's
   `(Contract(id), Symbol("milestones"))` pattern).
2. Decide between `persistent()` and `temporary()`.  Use temporary for
   anything that must auto-expire without an explicit cleanup call
   (approvals, proposals, short-lived permissions).  Use persistent for
   accounting / governance / immutable records.
3. For temporary entries: pick a TTL, bump threshold, add a row to §4
   above, and use `store_with_ttl` + `read_if_live` uniformly (no direct
   `.set` bypass).
4. For persistent entries: decide if / when TTL is extended and use one of
   the `extend_*_ttl` helpers consistently.  Document any "deliberately not
   bumped" exceptions (e.g. `contract_exists`, `is_settlement_token_bound`).
5. Add a storage test that writes then reads back, and — for
   temporary entries — a TTL eviction test that advances ledger sequence
   past TTL + 1 and asserts `None`.
6. Re-read this document and update the affected tables so they stay in
   sync with the code.

---

## 10. Validation Boundary Reference

This section is the single consolidated table of validation boundaries.
Every row maps to a key in §3 or §4 and to a test in §8.1.  The columns
are:

- **Key** — the storage key expression.
- **Valid** — the accepted input range / shape.
- **Invalid** — the rejected input range / shape and the error raised.
- **Duplicate** — what happens on a second write.
- **Boundary** — the exact inclusive/exclusive edge and its behavior.
- **Invariant** — the state invariant that must hold after the call.

| Key | Valid | Invalid | Duplicate | Boundary | Invariant |
|---|---|---|---|---|---|
| `Initialized` | `false → true` once | second `initialize` ⇒ `AlreadyInitialized` | rejected | `true` is terminal | `Initialized == true` ⇒ `Admin` is set |
| `Admin` | any valid `Address` | malformed address ⇒ host error | rotation via `PendingAdmin` only | — | `Admin` is always set after `initialize` |
| `PendingAdmin` | `proposed != Admin`, aged `>= ADMIN_ROTATION_MIN_DELAY_LEDGERS` | same address ⇒ `InvalidState`; not aged ⇒ `InvalidState` | single-slot; second proposal ⇒ `InvalidState` | `proposed_at_ledger == 0` ⇒ not aged | at most one pending proposal |
| `Paused` | `false`/`true` | `pause` while `true` ⇒ `ContractPaused`; `unpause` while `false` ⇒ `ContractNotPaused` | rejected | — | `Emergency == true` ⇒ `Paused == true` |
| `Emergency` | `false`/`true` | `activate_emergency_pause` while `true` ⇒ `EmergencyActive`; `resolve_emergency` while `false` ⇒ `EmergencyNotActive` | rejected | — | `Emergency == true` blocks `unpause` |
| `NextContractId` | `1 ..= u32::MAX` | `u32::MAX` + 1 ⇒ `Overflow` | — | `0` never allocated | monotonic; no gaps from failed creates |
| `Contract(id)` | `id >= 1`, valid struct | `id == 0` or absent ⇒ `ContractNotFound` | write-once per id | `id == u32::MAX` is the last allocatable | `funded_amount <= total_deposited`; `released_amount + refunded_amount <= funded_amount` |
| Milestone vector | `1 <= len <= MAX_MILESTONES`, each `amount > 0` | `len == 0` ⇒ `InvalidMilestones`; `len > MAX` ⇒ `TooManyMilestones`; `amount <= 0` ⇒ `InvalidAmount` | — | `len == MAX_MILESTONES` accepted; `MAX + 1` rejected | `released` and `refunded` mutually exclusive |
| `ProtocolFeeBps` | `0 ..= 10_000` | `> 10_000` ⇒ `InvalidFeeBps` | idempotent (same value accepted) | `10_000` accepted; `10_001` rejected | fee applied to release amount only |
| `GovernedParameters` | `bps in [0, 10_000]`, `max >= 0` | `bps > 10_000` ⇒ `InvalidFeeBps`; `max < 0` ⇒ `InvalidAmount` | idempotent | `max == 0` means "no cap" | `ReadinessChecklist.governed_params_set` flips once |
| `AccumulatedProtocolFees` | `>= 0` | withdraw `> balance` ⇒ `InsufficientFees` | — | withdraw `== balance` ⇒ counter `0` | counter never negative |
| `SettlementToken` | valid `Address` | second bind ⇒ `SettlementTokenAlreadyBound` | rejected | — | absent ⇒ money paths fail `SettlementTokenNotConfigured` |
| `Finalization(id)` | participant, status `Completed`/`Disputed` | non-participant ⇒ `Unauthorized`; wrong status ⇒ `InvalidState` | second finalize ⇒ `AlreadyFinalized` | — | once written, all mutations fail `AlreadyFinalized` |
| `ReadinessChecklist` | each flag `false → true` | — | idempotent (no-op) | — | flags are monotonic |
| `ReputationIssued(id)` | `false → true` once | second issue ⇒ `ReputationAlreadyIssued` | rejected | — | `true` ⇒ `Reputation` updated exactly once |
| `PendingReputationCredits` | `>= 0` | decrement at `0` ⇒ `NoPendingCredit` | — | `0` is the floor | counter never negative |
| `Reputation` | `completed >= 0`, `total >= 0`, `last in [0, 10_000]` | `last > 10_000` ⇒ `InvalidRating` | — | `completed == 0` ⇒ `get_average_rating` returns `None` | counters monotonic |
| `ReputationComment(id)` | `1 ..= 200` UTF-8 bytes | `""` ⇒ `EmptyComment`; `> 200` ⇒ `CommentTooLong` | write-once per id | `200` accepted; `201` rejected | comment length capped |
| `MilestoneApprovals(id, idx)` | `id` exists, `idx < len`, role not yet approved | `id` absent ⇒ `ContractNotFound`; `idx >= len` ⇒ `InvalidMilestoneIndex`; same role twice ⇒ `AlreadyApproved` | rejected per role | `idx == len - 1` accepted; `idx == len` rejected | evicted ⇒ treated as not approved |
| `PendingClientMigration(id)` | `proposed != current`, status not terminal | same address ⇒ `InvalidState`; terminal status ⇒ `InvalidState` | single-slot; second ⇒ `InvalidState` | — | evicted ⇒ treated as no migration |

### 10.1 Cross-cutting invariants

1. **Fail-closed on absence.**  A missing or evicted key never grants
   permission and never satisfies a guard.  Every guard that depends on a
   storage key treats `None` as the rejecting branch.
2. **No partial writes.**  Every mutating entrypoint validates all inputs
   and all guards *before* the first storage write.  A rejected call
   therefore leaves storage byte-for-byte unchanged.
3. **Monotonic counters.**  `NextContractId`, `AccumulatedProtocolFees`,
   `PendingReputationCredits`, and `Reputation.*` are monotonic in the
   direction documented above; no path decrements them below their floor.
4. **Write-once keys.**  `Initialized`, `SettlementToken`,
   `Finalization(id)`, `ReputationIssued(id)`, and
   `ReputationComment(id)` are write-once; a second write is rejected
   with the documented error.
5. **Single-slot keys.**  `PendingAdmin` and
   `PendingClientMigration(id)` hold at most one proposal at a time; a
   second proposal is rejected until the first is accepted or cancelled.
6. **TTL does not weaken validation.**  An entry that is present but
   near expiry is validated exactly like a fresh entry; an entry that has
   been evicted is treated as absent (fail-closed).  TTL extension never
   changes the accepted input range.
7. **Observability.**  Every rejection raises a typed `ContractError`
   (see `types.rs`) that is stable across releases.  Errors never include
   addresses, amounts, or comment contents, so logs and metrics can be
   emitted without exposing sensitive data.
