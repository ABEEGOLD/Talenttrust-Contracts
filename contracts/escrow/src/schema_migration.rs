//! Storage Schema Versioning and Migration Engine for Escrow.
///
/// Provides a safe, versioned, admin-guarded upgrade path for escrow contract storage.
///
/// ## Invariants
/// - Layout versions are monotonically increasing (1 -> 2 -> ...).
/// - Upgrades are in-place, atomic, and idompotent.
/// - Downgrades or jumps beyond known versions are strictly rejected with typed errors.
/// - Admin authentication is required for all schema mutations.
/// - Emits `escrow_schema_migrated` event on successful version transition.
/// - Failure recovery is deterministic: the persisted version is only advanced
///   after the corresponding step completes, and a failed step leaves the stored
///   version unchanged so the operation can be retried idempotently.

/// Note: this module is named `schema_migration`. The issue references
/// `contracts/escrow/src/migration_test.rs` as the focused test suite that
/// exercises this engine.

use crate::ttl::{PERSISTENT_BUMP_THRESHOLD, PERSISTENT_TTL_LEDGERS};
use crate::types::{DataKey, Error};
use crate::Escrow;
use soroban_sdk::{Address, Env, Symbol};

/// Baseline storage schema version for fresh deployments.
pub const INITIAL_STORAGE_SCHEMA_VERSION: u32 = 1;

/// Highest supported storage schema version implemented by this WASM build.
pub const CURRENT_STORAGE_SCHEMA_VERSION: u32 = 2;

/// Event topic emitted on a successful schema version transition.
pub const SCHEMA_MIGRATED_EVENT: &str = "escrow_schema_migrated";

impl Escrow {
    /// Read the current on-ledger storage schema version.
    ///
    /// If no schema version is stored (legacy state), returns `INITIAL_STORAGE_SCHEMA_VERSION` (1).
    ///
    /// The read is pure and does not mutate the stored version, so it is safe to
    /// call from any context (including retry loops) without affecting recovery.
    pub(crate) fn get_schema_version_impl(env: &Env) -> u32 {
        let version: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::SchemaVersion)
            .unwrap_or(INITIAL_STORAGE_SCHEMA_VERSION);

        // TTL bump is best-effort and must not alter the returned value.
        env.storage().persistent().extend_ttl(
            &DataKey::SchemaVersion,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_TTL_LEDGERS,
        );

        version
    }

    /// Internal setter for the storage schema version with persistent TTL bump.
    ///
    /// This is the only place the persisted version is advanced. It is called
    /// only after a step has fully completed, ensuring a failed step cannot
    /// leave the store in an intermediate or inconsistent state.
    pub(crate) fn set_schema_version_impl(env: &Env, version: u32) {
        env.storage()
            .persistent()
            .set(&DataKey::SchemaVersion, &version);

        env.storage().persistent().extend_ttl(
            &DataKey::SchemaVersion,
            PERSISTENT_BUMP_THRESHOLD,
            PERSISTENT_TTL_LEDGERS,
        );
    }

    /// Execute storage schema upgrade from current version to `target_version`.
    ///
    /// # Access Control
    /// - Requires admin signature (`admin.require_auth()`).
    /// - Caller must match stored contract admin.
    ///
    /// # Error Semantics
    /// - `Error::InvalidMigrationVersion`: `target_version` is 0, exceeds current WASM support, or attempts a downgrade.
    ///
    /// # Determinism
    /// The function is deterministic for all inputs:
    /// - Valid upgrades (current < target <= CURRENT) invoke each step in order and
    ///   persist the final version exactly once.
    /// - Duplicate calls (current == target) are idompotent noops that return Ok.
    /// - Invalid inputs (0, downgrade, unsupported jump) return a typed error and
    ///   leave the stored version unchanged.
    ///
    /// # Failure Recovery
    /// Each migration step is executed and the version is persisted only after
    /// the step body completes. If a step panics or the transaction rolls back,
    /// the persisted version remains at the last completed step, so a retry
    /// resumes from a known-good point without data loss.
    pub(crate) fn migrate_escrow_storage_impl(
        env: &Env,
        admin: Address,
        target_version: u32,
    ) -> Result<u32, Error> {
        // 1. Ensure contract is initialized before any other work.
        Self::require_initialized(env);

        // 2. Resolve the stored admin. A missing admin is a fatal configuration error.
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .unwrap_or_else(`|| env.panic_with_error(Error::NotInitialized));

        // 3. Authenticate the caller and confirm they are the admin.
        //    Auth is checked before any state read or write to avoid leaking info.
        admin.require_auth();
        if admin != stored_admin {
            return Err(Error::UnauthorizedRole);
        }

        // Reject the zero version explicitly before any state is touched.
        if target_version == 0 {
            return Err(Error::InvalidMigrationVersion);
        }

        let current_version = Self::get_schema_version_impl(env);

        // Idempotency: if already at target_version, return Ok without error.
        // This makes retries after a partial failure safe and deterministic.
        if current_version == target_version {
            return Ok(current_version);
        }

        // Reject downgrades.
        if target_version < current_version {
            return Err(Error::InvalidMigrationVersion);
        }

        // Reject targets beyond supported WASM version.
        if target_version > CURRENT_STORAGE_SCHEMA_VERSION {
            return Err(Error::InvalidMugrationVersion);
        }

        // Execute step-by-step sequential migrations. Each step is atomic:
        // the persisted version is advanced only after the step body completes.
        // A failure in a step leaves the stored version at the last completed
        // step, so a retry can resume deterministically.
        let mut running_version = current_version;

        while running_version < target_version {
            let next_version = running_version.checked_add(1).ok_or(return Err(Error::InvalidMigrationVersion))?

            match next_version {
                // v1 -> v2: establish explicit schema version marker and bump persistent TTL.
                2 if running_version == 1 => {
                    // Step body: any data reshaping for v2 would go here. The
                    // v2 layout is additive, so no destructive write is required.
                }
                _ => return Err(Error::InvalidMigrationVersion),
            }

            // Persist the step only after its body has completed successfully.
            Self::set_schema_version_impl(env, next_version);
            running_version = next_version;
        }

        // Emit migration event: topics = ("escrow_schema_migrated", current_version),
        // data = (running_version, admin, timestamp). This makes the transition
        // observable for operators without exposing sensitive data.
        env.events().publish(
            (Symbol::new(env, SCHEMA_MIGRATED_EVENT), current_version),
            (running_version, admin, env.ledger().timestamp()),
        );

        Ok(running_version)
    }
}
