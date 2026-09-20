# Order progress: Rust read and monitor core

The isolated `dfmcp_adapter::order_progress` module decodes the complete native
progress/1.11 record documented in ORDER_PROGRESS.md and supplies a fixed read-only
RPC client plus Query-authorized foreground monitor sessions. This is development
source, not a production runner, admission, or completed-goods verification.
Creation/1.10 and its journal/evidence are not modified.

OrderProgress retains canonical bytes and validates complete presence/scalar/text
and finite-template evidence. Unsupported configurations keep their raw type,
frequency and status values without gaining a template. The folder/site lineage
matches the existing live domain; a separate reader incarnation does not prove
identity continuity with historical creation receipts.

ProgressClient binds only Handshake and ReadOrderProgress. It rejects wrong
protocol/nonce/software identity, malformed protobuf and nonincreasing samples.
Frames are at most 4 KiB, native observations 1 KiB, and notifications eight with
64 KiB per notification and 256 KiB combined. Numeric loopback TCP connect and
bootstrap share an absolute deadline; subsequent calls take explicit 1..60,000 ms
allowances. Failed wire/evidence calls fence the stream. No reconnect, background
work, arbitrary method dispatch or mutation call is implemented.

ProgressSession requires current Query authority on every operation. It checks
session/fortress identity and tracks the highest observed tick, sample sequence
and allocation horizon. Failed refresh clears selection and fences the source but
keeps earlier watch evidence for authorized diagnosis. Grants are checked at the
new observed tick, never resurrected by an older caller anchor.

At most eight immutable-key ProgressWatch records are retained, in ASCII order.
Each records a complete baseline, previous and latest sample, future game-time
deadline, cadence and required zero samples. Same-tick/too-fast samples and the
baseline earn no stability credit. Missing orders, changed templates/total/job
type or increased remaining counters retire the monitor without claiming success.
Expiration at the deadline wins over zero stability. CounterZeroStable means only
the sampled counter predicate, never continuous history or produced goods.

Terminal polls replay historical evidence without native calls. Exact-key replay
does not renew deadlines; cancellation only retires local observation work. Byte
and entity bounds apply before native acquisition and retained-record copying.
Watches are process-local and are lost on close/restart. No journal persistence,
downtime continuity, native cancellation, or uncertain-creation reconciliation is
provided by this module.

Twenty Rust regression groups are registered: ten codec/temporal, five fixed RPC
and five session groups. They have NOT been compiled or executed because Rust,
Cargo and rustfmt are unavailable. Source review and native test doubles do not
establish Rust, TCP, real DFHack SDK, MCP or live-game qualification.
