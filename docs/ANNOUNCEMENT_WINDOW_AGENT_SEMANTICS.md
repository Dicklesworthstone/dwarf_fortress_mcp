# Announcement-window agent semantics

An empty announcement page is useful only when its interval is explicit. Agent-facing output for this generation must include:

```text
after_report_id
oldest_retained_report_id
latest_retained_report_id
window_latest_report_id
next_after_report_id
history_truncated
complete
can_prove_absence_in_frozen_interval
```

The system may say “no retained announcement exists in this frozen interval” only when `complete` is true and `history_truncated` is false. Otherwise the correct statement is that the retained window is partial and earlier events may have been lost.

A later page may never silently retarget its high-water mark to include newly appended reports. Those reports belong to the next observation turn. This gives agents a stable replayable event batch, a monotonic continuation cursor, and an honest reason to refresh without conflating refresh with retry.