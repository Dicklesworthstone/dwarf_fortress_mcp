## Reviewed minimum-corridor solver

Beads: dfmcp-wp-pln01-spatial-blueprint-layout-pwd;
df-dfhack-bridge-plane-c-pic.3/.4/.5. Broad work remains open.

The pure bounded minimum-new-tile solver from the pending corridor patch is now
source-present. Review found and fixed numeric alias acceptance at its root
argument: exact integer coordinates are required before membership checks.

Eight new warnings-denied Python tests passed. Independent exhaustive subset
oracles check all 6,561 small three-state worlds plus 400 seeded larger maps.
Other cases cover free-floor cycles, every cancellation checkpoint, all original
roots, exact allowances, nine-component structural support and invalid bounds.
The posted runtime and test Git blob IDs match the tested local bytes.

This increment does not enable the fallback in existing callers. Reviewing the
pending integration reproduced a saved-layout replay regression: changed earlier
candidate feasibility can invalidate an existing successful search artifact.
The next increment must preserve old search semantics explicitly before enabling
that integration. No native, Rust, live-game or full-repository qualification is
claimed; the pure solver does not change mutation authority or admission.
