# `mana_payment/` — Mana cost resolution and payment execution

This module owns everything about turning "this costs `{2}{R}`" into concrete
state changes: deciding *which* mana sources to use, and then actually tapping
them and draining the pool.

It is split into two cleanly separated concerns:

| File | Concern | Key items |
| --- | --- | --- |
| `mod.rs` | **Resolution** — pure, side-effect-free algorithms that decide *which* sources satisfy a cost. No `GameState`; operates on `ManaSource` snapshots. | `PaymentResult` (Yes/No/Maybe three-valued logic), `ManaSource`, the `ManaPaymentResolver` trait, `SimpleManaResolver`, `GreedyManaResolver` |
| `payment_execution.rs` | **Execution** — inherent `impl GameState` methods that mutate the live game state to actually pay costs. | `tap_for_mana`, `tap_for_mana_and_update_hint`, `tap_for_mana_for_cost`, `pay_mana_cost_by_tapping`, `pay_ability_cost`, `reflected_mana_colors` |

## Why the split

`payment_execution.rs` was extracted verbatim from the ~11k-line
`game/actions/mod.rs` (a pure structural refactor — no behavior change, proven
by an empty before/after seeded-game-log diff). The payment-execution methods
belong next to the resolver they drive, not buried in the action dispatcher.

The resolution layer (`mod.rs`) is the *decision* — given a cost and a set of
available sources, which sources to tap (and a tap order). The execution layer
(`payment_execution.rs`) is the *effect* — given that decision, walk the
battlefield, tap the permanents, run tap-triggers, add to the pool, and spend
it. Keeping them in one module (but separate files) means the algorithm and its
sole consumer evolve together while each file stays focused and well under the
2000-line guideline.

## Determinism (read before touching anything here)

Mana payment is **determinism-sensitive**: the tap order and pool-draining must
be identical on the server (full state) and on a shadow client, or the network
simulation desyncs (which is *always fatal* — see `docs/NETWORK_ARCHITECTURE.md`).
The resolvers are deterministic functions of their inputs and the execution
methods iterate the battlefield in a stable order; preserve that. Never branch
mana-payment on hidden information or RNG.

## Relationship to Java Forge

Forge-Java spreads this across `ManaCostBeingPaid`, `ComputerUtilMana`, and the
cost-payment classes. This Rust version keeps the resolver strategies behind one
trait (`ManaPaymentResolver`) and the state mutation in `GameState` methods; the
file split here is a Rust-side structural choice with no direct Java analog.
