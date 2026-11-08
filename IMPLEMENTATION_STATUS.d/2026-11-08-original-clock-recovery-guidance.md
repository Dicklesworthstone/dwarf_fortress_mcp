## Original clock recovery guidance

Beads: `df-tree-clock-8q4` and `dfmcp-reality-wiring-proof-4bf.14.6`.

Tree Agent Turns previously recommended clock Wait for every pending/unresolved
interval. That included a retained Intent/Prepared interval after review loss,
terminal SourceLost records, and a blocked clock with no active interval.
Reconciliation cannot recreate the process-local review and does not query
terminal records, so those recommendations could never advance the workflow.

The presentation now distinguishes authorized local retirement, source/custody
inspection, and useful original-key reconciliation. Only a live Prepared review
can produce a clock confirmation. The independently authorized safety-stop offer
is retained only for a potentially dispatched active interval; terminal records
do not advertise a no-op stop. Both recovery families and indeterminate exposure
remain visible, without a pause or goal-completion claim.

Workshop's matching unattempted-run branch now uses the actual `intent_recorded`
wire label instead of the never-emitted `intent` label. Regression fixtures use
the real `RunState` labels, and recovery without retirement authority points to
inspection for both Intent and Prepared states.

Five new Tree presentation tests cover lost reviews, terminal source loss,
blocked owners without an active interval, in-flight reconciliation/safety stop,
and invalid/stale confirmation availability. The existing Workshop recovery test
now exercises real journal labels. These are presentation regressions; their
execution and current-source qualification are recorded by the integrating run.
No native gameplay, whole-workspace qualification or production admission is
claimed by this source change.
