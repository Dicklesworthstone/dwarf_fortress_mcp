# Durable whole-original-room terrain monitoring

Add `track_excavation.py start-rooms --plan-file/--request-file` and resume the
whole room goal through existing sample/inspect/cancel commands. Preserve every
original floor, required bedroom wall, furnishing constraint and excluded item
in the private journal. Require one shared advancing-tick stability streak;
never replace it with residual excavation or independent room successes.

Add a bounded room journal profile, source/clock invalidation, durable failed/
unfinished-read recovery, revocable shared live-read budgets, final publication
checks and single-attempt output. Keep floor/blueprint journal and response bytes
compatible. Thirty-two focused Python tests passed, including real CLI/files and
joined synthetic TCP, full retention, twenty legacy trace/response goldens and
three rejected mutations. Native game/Rust/full-repository admission unchanged.
