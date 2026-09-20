# Typed fortress-bound conditional running in Rust

`dfmcp_adapter::order_run` implements the existing native order-run/1.14 wire
without widening run/1.13 or adding dependencies. It reconstructs immutable
captures, finite predicate plans, keyed preparation tokens, and native receipts.
Folder/site bytes are retained and checked exactly; the numeric FortressId uses
the existing live lineage hash domain. An operator-selected fortress must match
the capture. A numeric hash alone never substitutes for those identity checks.

`OrderRunRpc` binds exactly the six native methods. Its actual loopback endpoint,
source generation and software tuple are available for durable coordination.
Sockets carry an absolute deadline that subsequent steps can only narrow. A
failed native exchange fences the connection. No reconnect, commit retry, raw
method selection, shell, Lua, or subprocess is exposed.

Query authority is checked before native I/O and at newly observed game ticks.
Preparation/commit require guarded ControlClock, Plan, the correct fortress and
a grant horizon covering every requested game tick. Limited-use grants are
refused rather than copied and replayed. These authority checks do not establish
a global clock lease or production compatibility admission.

Receipts validate the complete plan, sample identity, phase/reason/trigger,
canonical booleans, native counter bounds, stability count/cadence lower bound,
current predicate sample, token and checksum. Repeated terminal receipts must
be identical; retained trigger evidence cannot change. Counter/horizon regression
claims lack their preceding sample in this native wire and are not independently
promoted into a complete observed history. A predicate sample, verified pause,
current pause state and produced goods remain separate claims.

Thirteen Rust test groups are registered across codec, authority and scripted
RPC paths. They include byte corruption/truncation, false rehashed predicate
samples, replacement sources, expired/limited grants, phase regressions, bounded
frames, fragmented transport, maximal sizes and failed-commit fencing. **Rust,
Cargo and rustfmt are unavailable here: these tests are uncompiled/unexecuted.**

`python3 scripts/check_order_run_rust_vectors.py` independently reconstructs the
295-byte predicate fixture and verifies maximum capture/plan/receipt lengths of
573/604/1425 bytes plus the 736-byte keyed durable intent. This is a Python
canonical-reference check, not Rust, native SDK, live-game or MCP qualification.
The native profile, production runner map and admission remain unchanged.
