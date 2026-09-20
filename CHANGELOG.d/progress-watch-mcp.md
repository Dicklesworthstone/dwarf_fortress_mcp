### Durable progress watches through the existing MCP query surface

Wire optional operator WATCHES custody into progress/1.12 bootstrap before native
connection. Expose watch_register, watch_list, watch_status and watch_cancel inside
the existing history query string. Require exact archive/definition/frontier identity,
bounded predicates and current authority. Return all derived outcomes with exact
sample witnesses; publish a bounded discoverable definition index in Agent Turns.
Offline mode remains Query-only and cannot mutate intent; session closure does not
cancel watches. Restart discontinuities retire unfinished predicates explicitly.

Eight new Rust groups remain uncompiled/unexecuted, for 32 groups across this work.
Independent Python checks pass 17 positive and 55 negative request envelopes; the
maximal complete response model is 98,823 bytes against 147,456 reserved. No Rust,
MCP, filesystem, live-game or admission qualification is claimed. Existing native
wire, creation journals, dependencies and production runners are unchanged.
See docs/PROGRESS_WATCH_MCP.md and architecture/progress_watch_requests_v1.json.
