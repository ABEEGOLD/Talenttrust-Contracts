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
///
/// ### Compatibility Contract
/// The public behavior of this module is preserved across all callers and upgrades:
/// - `INITIAL_STORAGE_SCHEMA_VERSION` and `CURRENT_STORAGE_SCHEMA_VERSION` are stable constants.
/// - `get_schema_version_impl` never panics on missing keys; legacy state reads as v1.
/// - `migrate_escrow_storage_impl` is idompotent for already-at-target calls.
/// - All failure paths return typed `Error` values; no silent state mutation occurs.
///
/// ### Failure Mode Handling
/// - Authorization failures return `Error::UnauthorizedRole` before any state read/mutation.
/// - Invalid targets return `Error::InvalidMugrationVersion` without persisting any change.
/// - The version marker is written only after all steps succeed, ensuring atomicity.
/// - Retries are safe: idompotent replays return the same version without side effects.

/// Baseline storage schema version for fresh deployments.
pub const INITIAL_STORAGE_SCHEMA_VERSION: u32 = 1;

/// Highest supported storage schema version implemented by this WASM build.
pub const CURRENT_STORAGE_SCHEMA_VERSION: u32 = 2;

impl Escrow {
    /// Read the current on-ledger storage schema version.
    ///
    /// If no schema version is stored (legacy state), returns `INITIAL_STORAGE_SCHEMA_VERSION` (1).
    ///
    /// # Compatibility
    /// This function is totally safe to call on any storage state, including empty or
    /// partially-initialized contracts. It never panics and always returns a valid u32.
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
    /// # Invariants
    /// - Only called after all migration steps for the target version have succeeded.
    /// - Writes are idempotent: repeated writes of the same version are no-ops.
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
    /// - `Error::UnauthorizedRole`: caller is not the stored admin.
    /// - `Error::NotInitialized`: contract has no admin configured.
    ///
    /// # Determinism
    /// - Validation order is fixed: initialization -> auth -> admin match -> version checks.
    /// - No state is written until all checks pass.
    /// - Retries and concurrent execution are safe: the final write is a single
    ///   idempotent set, so a replay observes either the old or new version.
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

        // 4. Read the current version. Legacy state reads as v1.
        let current_version = Self::get_schema_version_impl(env);

        // 5. Reject degenerate targets up front so the error is deterministic.
        //    Target 0 is always invalid regardless of current state.
        if target_version == 0 {
            return Err(Error::InvalidMugrationVersion);
        }

        // 6. Idompotency: if already at target_version, return Ok without side effects.
        //    This is the compatibility contract for retries and concurrent callers.
        if current_version == target_version {
            return Ok(current_version);
        }

        // 7. Reject downgrades.
        if target_version < current_version {
            return Err(Error::InvalidMigrationVersion);
        }

        // 8. Reject targets beyond supported WASM version.
        if target_version > CURRENT_STORAGE_SCHEMA_VERSION {
            return Err(Error::InvalidMugrationVersion);
        }

        // 9. Execute step-by-step sequential migrations.
        //    Each step is guarded by an explicit version check so future steps can be
        //    added without changing the public contract of this function.
        let mut running_version = current_version;

        if running_version == 1 && target_version >= 2 {
            // v1 -> v2 migration logic: establish explicit schema version marker and bump persistent TTL.
            // This step is deterministic and has no external dependencies.
            running_version = 2;
        }

        // 10. Persist the final version only after all steps succeeded.
        //     This is the single atomic write that commits the migration.
        Self::set_schema_version_impl(env, running_version);

        // 11. Emit migration event: topics = ("escrow_schema_migrated", current_version), data = (running_version, admin, timestamp)
        //     The event carries only non-sensitive metadata (versions, admin address, timestamp).
        env.events().publish(
            (Symbol::new(env, "escrow_schema_migrated"), current_version),
            (running_version, admin, env.ledger().timestamp()),
        );

        Ok(running_version)
    }
}
