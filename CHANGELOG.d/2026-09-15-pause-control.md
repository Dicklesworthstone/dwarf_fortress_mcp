# Pause-control/1.7 development effect boundary

- Added a new explicitly unadmitted control profile with fixed `Handshake`, `PreparePause`, `CommitPause`, and `QueryPause` RPC methods.
- Added stable idempotency-key and sealed-plan-digest binding for pause/resume prepare and commit.
- Commit records the attempt before applying `World::SetPauseState`, observes the resulting pause state, and retains a replay-safe receipt.
- Ambiguous commit transport failure is represented as `EffectIndeterminate`; reconciliation must query the retained bridge record before retry.
- World load/unload advances the control generation and clears retained effect records to avoid carrying idempotency state across world identity changes.
- Added a closed safe-Rust control client that binds only control/1.7 plugin/type/method identities and loopback transport.
- Added `dfmcp-live-control-dev-server`, preserving the eleven-tool waist while granting only reversible `ControlClock` authority and refusing every non-pause effect family.
- Added operator-only development gating and dedicated token/endpoint configuration; production admission state and unrelated `DFMCP_*` configuration are refused.
- Added `docs/LIVE_CONTROL.md` and updated `IMPLEMENTATION_STATUS.md` to distinguish source presence from qualification/admission.

Evidence status: source-present only in this editing session. Rust compilation, real DFHack/protobuf build, disposable-fort effect campaigns, durable effect-journal restart recovery, registry admission, deployment-floor advancement, server qualification, and production-runner admission are not claimed.
