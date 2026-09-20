### Typed run/1.13 evidence and fixed Rust RPC adapter

Add sealed bounded-run observations/plans, strict phase/receipt validation and a
fixed six-method native client. Enforce explicit source-domain Query/ControlClock
authority, complete run horizons, loopback-only transport, one absolute deadline,
bounded framing and failed-connection fencing without hidden effect retries.
Preserve uncertainty, historical pause semantics and observed overshoot.

Thirteen Rust regression groups are registered but uncompiled/unexecuted because
Rust/Cargo/rustfmt are unavailable. An independent Python-generated canonical
fixture is retained; no SDK, live-game, MCP, full-qualification or admission claim.
