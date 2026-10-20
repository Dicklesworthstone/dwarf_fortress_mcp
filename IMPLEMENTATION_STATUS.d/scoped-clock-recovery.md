# Independent scoped-clock recovery

The recovery-only CLI is implemented and executed against actual private clock
files and joined loopback peers. It uses unchanged scope/intent/receipt codecs
and query transport while leaving start-owner Batch checks intact. Recovery no
longer requires opening furniture custody or loading map/placement credentials.

23 tests pass with six assertion-rejected mutants. Test fixtures contain synthetic
native values, not a live fortress. No Rust/MCP, real SDK/plugin, full-workspace,
continuous pause, construction completion or admission claim is established.
Beads df-dfhack-bridge-plane-c-pic.4/.5 remain in progress.


The explicit original-key cancel path is also integrated without adding clock-start
or placement permission. Combined evidence: 34 real-file/TCP/CLI tests pass and ten
weakened variants fail assertion checks. Cancellation is one-shot on the original
incarnation and never auto-polls; a stopping or source-lost record stays unresolved.
Query's original read-only transport and existing execution owners are unchanged.
