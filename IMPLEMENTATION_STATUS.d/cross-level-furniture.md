# Cross-level route-constrained furniture

Source and executed Python component evidence: furniture on another floor can
be allocated under an explicit 0..127 endpoint-z limit and complete observed
stair route. Default same-floor request/allocation/review bytes remain unchanged.
Original limits survive retained reviews, moved-item checks and receipt-backed
placement-prefix validation. Terrain-free allocation refuses this opt-in.

All 26 actual component tests pass, with six weakened versions rejected by
assertions. The suite includes exact native value codecs but supplies synthetic
map/inventory/receipt data; it does not execute native acquisition, private batch
parent custody, Rust/MCP, DFHack or the whole workspace. No production capability,
wire encoding or admission changes. See docs/CROSS_LEVEL_FURNITURE.md and the
machine contract. Broad beads df-dfhack-bridge-plane-c-pic.3/.4/.5 remain open.
