//! Centralized storage key definitions and constructors for escrow milestones.

/// # State Invariants

/// This module owns the canonical construction of escrow storage keys. The

/// invariants it protects are:

/// 1. **Determinism**: a given logical key always maps to the exact same

///    concrete storage key tuple, regardless of call order or context.

/// 2. **No collisions**: distinct logical keys (different contract ids,

///    different milestone indices, or different feature namespaces) must

///    produce distinct storage keys.

/// 3. **Namespace separation**: milestone vectors and milestone

///    approvals must never share a storage key, even when the contract id and

///    index coincide.

///

/// ## Failure modes

/// The constructors are total and infallible: they do not panic, do not

/// allocate arbitrary memory, and do not depend on external state. This

/// means a retry or a concurrent call cannot observe a different key for the

/// same logical identity. The only way a key can change is through a

/// deleberate source change, which would be a breaking migration.

///

/// ## Authorization

/// This module is purely deterministic and does not authorize anything.

/// Authorization is enforced by the callers that use these keys. The

/// invariant here is that the key for a given contract is derived only from

/// the contract id and the feature namespace, never from caller-supplied

/// authorization data.

///

/// ## Concurrency

/// Since the constructors are pure functions of their arguments, two

/// concurrent invocations with the same arguments produce equal keys.

/// Soroban contract execution is single-threaded per invocation, but this

/// guarantee holds even if the runtime changes.

///

/// ## Boundary cases

/// `contract_id` and `milestone_index` are `uint32`, so the full range

/// including `0` and `uint32::MAX` is valid and produces distinct keys.

/// The constructors do not impose additional range restrictions; callers

/// validate logical bounds (e.g. milestone index within the vector).

///

/// ## Regression guard

/// The tests in `namespaced_keys_test.rs` assert determinism,

/// non-collision, and namespace separation. Any change to the key shape

/// must update those tests and include a migration plan.

 use soroban_sdk::{Env, Symbol};

use crate::types::DataKey;

/// Returns the persistent storage key tuple for a contract's milestones vector:

/// `(DataKey::Contract(contract_id), Symbol::new(env, "milestones"))`.

///

/// ## Invariants

/// - The first element is always `DataKey::Contract(contract_id)`.

/// - The second element is always the `milestones` symbol.

/// - Two calls with the same `contract_id` produce equal keys.

/// - Two calls with different `contract_id` produce distinct keys.

///

/// ## Panics

/// This function does not panic.

///

/// ## Example

/// ```

/// # use crate::keys::milestone_key;

/// # use soroban_sdk:Env;

/// let env = Env::default();

/// let key = milestone_key(&env, 42);

/// ```

pub fn milestone_key(env: &Env, contract_id: u32) -> (DataKey, Symbol) {

    (DataKey::Contract(contract_id), milestone_symbol(env))

}

/// Returns the `Symbol` key for milestones: `"milestones"`.

///

/// ## Invariants

/// - The returned symbol is always the literal `"milestones"`.

/// - The returned symbol is identical across all calls and environments.

/// - The symbol is not derived from any external input, so it cannot be

///   influenced by a caller.

///

/// ## Panics

/// This function does not panic.

///

/// ## Example

/// ```

/// # use crate::keys::milestone_symbol;

/// # use soroban_sdk::Env;

/// let env = Env::default();

/// let sym = milestone_symbol(&env);

/// ```

pub fn milestone_symbol(env: &Env) -> Symbol {

    Symbol::new(env, "milestones")

}

/// Returns the temporary storage key for milestone release approvals:

/// `DataKey::MilestoneApprovals(contract_id, milestone_index)`.

///

/// ## Invariants

/// - The returned key is always `DataKey::MilestoneApprovals`, never another

///   variant. This separates approvals from milestone vectors and from

///   release flags.

/// - Two calls with the same `(contract_id, milestone_index)` produce equal

///   keys.

/// - Two calls with different `contract_id` or different `milestone_index`

///   produce distinct keys.

/// - The key does not depend on authorization or on the caller.

///

/// ## Panics

/// This function does not panic.

///

/// ## Example

/// ```

/// # use crate::keys::milestone_approval_key;

/// let key = milestone_approval_key(10, 2);

/// ```

pub fn milestone_approval_key(contract_id: u32, milestone_index: u32) -> DataKey {

    DataKey::MilestoneApprovals(contract_id, milestone_index)

}
