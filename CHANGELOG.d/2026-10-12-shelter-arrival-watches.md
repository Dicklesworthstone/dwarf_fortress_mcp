## Joint shelter arrival in existing foreground watches

- Add a bounded `shelter_set` condition over original spatial/1.8 citizen
  generations and shelter geometry, with complete source-qualified diagnostics.
- Reuse existing watch cadence, deadline, gap/ABA handling, cancellation,
  checkpoint publication and condition inspection; no parallel monitor or effect.
- Add nine Rust regressions (unexecuted here) and executable structural checks.
  No native protocol, compatibility registry or production runner is widened.
