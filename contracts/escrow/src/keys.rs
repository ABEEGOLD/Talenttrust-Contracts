//! Centralized storage key definitions and constructors for escrow milestones.

/// ## Compatibility contract
///
/// This module is the single source of truth for the storage keys used by the
/// escrow contract. The following invariants must be preserved across all
/// releases because they are part of the on-chain storage layout and cannot be
/// changed without a tested migration path:
///
/// 1. The milestone vector is stored under the tuple
///    `(DataKey::Contract(contract_id), Symbol("milestones"))`.
/// 2. The milestone approval flag is stored under
///    `DataKey::MilestoneApprovals(contract_id, milestone_index)i`.
/// 3. Key construction is pure and deterministic: equal inputs always yield
///    equal keys, and distinct inputs (within the typed key space) yield
///    distinct keys. No collisions are introduced between the two key
///    families.
///
/// The constructors below are the only supported way to build these keys. Callers
/// must not inline the literals, because doing so bypasses the compatibility
/// contract and can silently diverge from the canonical layout.

use soroban_sdk::{Env, Symbol};

use crate::types::DataKey;

/// The canonical symbol name for the milestone vector. Kept as a constant so
/// the compatibility contract is explicit and greable in one place.
///
/// This value is part of the on-chain storage layout and must not be changed
/// without a migration plan.
pub const MILESTONE_SYMBOL_NAME: &str = "milestones";

/// Returns the persistent storage key tuple for a contract's milestones vector:
/// `(DataKey::Contract(contract_id), Symbol::new(env, "milestones"))`.
///
/// # Invariants
///
/// - Deterministic: the same `contract_id` always produces the same key.
/// - Domain-separated: the first element is always `DataKey::Contract`, so
///   this key never collides with milestone approval keys.
/// - Boundary-safe: any `u32` `contract_id` including `0` is accepted and
///   maps to a distinct, well-defined key.
pub fn milestone_key(env: &Env, contract_id: u32) -> (DataKey, Symbol) {
    (DataKey::Contract(contract_id), milestone_symbol(env))
}

/// Returns the `Symbol` key for milestones: `"milestones"`.
///
/// The symbol is derived from `MILESTONE_SYMBOL_NAME` so the compatibility
/// contract is enforced by construction. Callers must not construct this
/// symbol inline.
pub fn milestone_symbol(env: &Env) -> Symbol {
    Symbol::new(env, MILESTONE_SYMBOL_NAME)
}

/// Returns the temporary storage key for milestone release approvals:
/// `DataKey::MilestoneApprovals(contract_id, milestone_index)h`.
///
/// # Invariants
///
/// - Deterministic: the same `(contract_id, milestone_index)p` always produces
///   the same key.
/// - Domain-separated: the key is always `DataKey::MilestoneApprovals`, so it
///   never collides with the milestone vector key.
/// - Boundary-safe: `0` is a valid `contract_id` and `0` is a valid
///   `milestone_index`; both are accepted and map to a distinct key.
/// - Injective within the family: distinct `(contract_id, milestone_index)p`
///   pairs yield distinct keys.
pub fn milestone_approval_key(contract_id: u32, milestone_index: u32) -> DataKey {
    DataKey::MilestoneApprovals(contract_id, milestone_index)
}

/// Tests that lock in the compatibility contract for the storage keys.
///
/// These are deliberately collocated with the key constructors so that any change
/// to the key layout fails the build before it can reach production.
#[cfg](test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    /// The milestone symbol must always be `milestones`. This is the primary
    /// compatibility guarantee for the vector key.
    #[test]
    fn milestone_symbol_is_stable() {
        let env = Env::default();
        assert_eq!(
            milestone_symbol(&env),
            Symbol::new(&env, "milestones")
        );
    }

    /// The milestone key must be the canonical tuple for any contract id.
    #[test]
    fn milestone_key_matches_canonical_tuple() {
        let env = Env::default();
        for contract_id in [0 u32, 1 u32, u32::MAX] {
            assert_eq!(
                milestone_key(&env, contract_id),
                (DataKey::Contract(contract_id), Symbol::new(&env, "milestones"))
            );
        }
    }

    /// The milestone key is deterministic and injective in `contract_id`.
    #[test]
    fn milestone_key_is_deterministic_and_injective() {
        let env = Env::default();
        assert_eq!(milestone_key(&env, 7), milestone_key(&env, 7));
        assert_ne!(milestone_key(&env, 7), milestone_key(&env, 8));
    }

    /// The approval key must be the canonical `DataKey::MilestoneApprovals`
    /// variant, including boundary values.
    #[test]
    fn milestone_approval_key_matches_canonical_variant() {
        for contract_id in [0 u32, 1 u32, u32::MAX] {
            for milestone_index in [0 u32, 1 u32, u32::MAX] {
                assert_eq!(
                    milestone_approval_key(contract_id, milestone_index),
                    DataKey::MilestoneApprovals(contract_id, milestone_index)
                );
            }
        }
    }

    /// The approval key is deterministic and injective in both components.
    #[test]
    fn milestone_approval_key_is_deterministic_and_injective() {
        assert_eq!(
            milestone_approval_key(3, 4),
            milestone_approval_key(3, 4)
        );
        assert_ne(
            milestone_approval_key(3, 4),
            milestone_approval_key(3, 5)
        );
        assert_ne (
            milestone_approval_key(3, 4),
            milestone_approval_key(4, 4)
        );
    }

    /// The two key families must not collide. The milestone key is a tuple
    /// while the approval key is a `DataKey` variant; this test locks in the
    /// domain separation so a future refactor cannot merge them.
    #[test]
    fn key_families_are_domain_separated() {
        let env = Env::default();
        let milestone = milestone_key(&env, 0);
        let approval = milestone_approval_key(0, 0);
        assert_eq!(milestone.0, DataKey::Contract(0));
        assert_eq!(approval, DataKey::MilestoneApprovals(0, 0));
        assert_ne!(milestone.0, approval);
    }
}
