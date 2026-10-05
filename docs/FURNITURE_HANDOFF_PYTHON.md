# Retaining inventory intent through Python furnishing execution

Owning beads: `df-dfhack-bridge-plane-c-pic.4` and
`df-dfhack-bridge-plane-c-pic.5`. Both retain their broader acceptance scope.

## Immutable handoff core

`scripts/furniture_handoff.py` retains one complete normalized
`dfmcp.furniture-request/1`, all exact selected items and native type numbers,
and an operations/1.4 capture reference. It derives the existing exact-item
`dfmcp.furniture-plan/1` rather than accepting a second independently editable
plan. The new `dfmcp.furniture-handoff-python/1` representation is bounded at
32 KiB and uses its own SHA-256 domain. It is not the Rust binary handoff format.

A complete assignment must satisfy every slot, exclusion, material/subtype,
same-level distance, unique-item and dependency constraint. The selected native
item type/key mapping must be consistent. A shortage cannot become a handoff.
The retained source covers the endpoint, software, inventory generation, exact
capture digest/size, game tick and job/building/item ID horizons.

Later native BEFORE observations must use the original item, kind, native type,
material and subtype. Position may change only inside the original level and
distance constraint. Source software/endpoint/fortress must still agree and
clocks and effect ID horizons cannot predate allocation. The operations plugin's
generation is not compared with a furniture-plugin generation: the batch must
independently fence the latter. Construction post-state is not an item-selection
precondition and must not be passed to the BEFORE validator.

## Authority and evidence

Serialized source references are not independently authenticated native evidence.
The effect-owning initializer must acquire the complete inventory through the
existing authenticated pager, verify release, derive this handoff, and retain it
inside original batch custody. Reading or decoding a handoff does not establish
that network acquisition took place. Neither the handoff nor its digest reserves
items, establishes reachability/eligibility, restores a connection permit,
authorizes reallocation or proves construction completion.

This first increment implements and executes the pure handoff core only. Native
request initialization and durable batch enforcement are separate integration
work; no command is advertised as accepting this artifact yet.

## Focused validation

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tests -p test_furniture_handoff.py -v
```

All 18 test methods pass, including global material scarcity, canonical ordering,
complete-plan derivation, 32-slot round trips, closed-schema/byte/nesting refusal,
source and selection substitution, conservative source-clock and ID-horizon
checks, and retained distance/material/subtype constraints. These are Python
semantic checks, not TCP, native DFHack, Rust/MCP, full qualification or admission.
