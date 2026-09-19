# Durable job intervention and recovery

Connect the isolated protocol-1.9 RPC client to a durable production-authorized
coordinator. Persist complete observations, plans, source identity and native
effect evidence; sync DispatchStarted before the setter call and verified
outcomes before acknowledgement. Reopening, missing native records, lost replies,
changed source identity and indeterminate outcomes never re-enable dispatch.
Query-only recovery cannot invoke prepare/commit. Add durable pre-dispatch
cancellation and bounded unresolved-record recovery for transcript loss.

Use separately locked, custody-checked Unix journal files. Do not store credentials,
truncate incomplete tails, fabricate canonical entity IDs, consume untracked
limited grants, or widen the production runner/compatibility registry.

Register 16 additional Rust regression groups (34 total for this feature). Rust,
Cargo and rustfmt are unavailable; these tests were not compiled or executed.
The committed independent Python reference checker executed five native states,
1,608 rejected effect bit corruptions, 1,361 rejected journal byte corruptions,
1,358 rejected incomplete journal prefixes and six rejected rehashed invalid
state histories. This is format/reference evidence, not execution of Rust or a
filesystem crash campaign. Retain its output with reviewed-source SHA-256 hashes.
See docs/JOB_SUSPENSION_RUST.md for APIs, evidence and remaining live/MCP work.
