# Excavation session source (unqualified)

The existing Rust excavation coordinator now has a typed foreground session and
concrete private-file/RPC backend supporting local plan review, one-shot start,
receipt query, authorized cancellation and offline/recovery handoff. This is not
native designation, global clock fencing, game checkpointing or production
admission. Inventory projections are not canonical world anchors.

Fourteen Rust session/coordinator regression groups are registered but UNCOMPILED
AND UNEXECUTED: Cargo/rustc/rustfmt are unavailable. Existing native wire, journal
formats, dependencies and production runners are unchanged. No live-game,
power-loss or full repository qualification is claimed. The standalone Python
blueprint monitor and mining recovery journal remain separate.
