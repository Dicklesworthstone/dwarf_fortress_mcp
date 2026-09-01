# Live announcement stream

The next live-read generation adds Dwarf Fortress reports and announcements without widening the
mutation boundary. The stream is an additive observation domain, not a command channel.

## Why announcements come next

Citizen state describes what the fortress is. Announcements describe what has just happened. They
provide high information value at low bridge cost: combat, cancellations, arrivals, deaths,
weather, mandates, production failures, and other game-significant events already pass through the
retained report log.

The implementation must not pretend that this retained log is an infinite history. Dwarf Fortress
may discard old reports. Every batch therefore carries both a cursor and the retained-window bounds.

## Cursor contract

A request names:

```text
announcement_after_id: -1 or a nonnegative report ID
max_announcements:     1..=512
```

A reply names:

```text
oldest_available_id
latest_available_id
requested_after_id
gap_before_window
complete_through_latest
next_after_id
```

`complete_through_latest` proves only that the response reached the latest report retained at the
observation instant. `gap_before_window` means the caller's prior cursor predates the retained
window, so historical continuity is unknown even if the retained suffix was returned completely.

Records are strictly ordered by ascending report ID. Duplicate, negative, out-of-window, or
noncanonical IDs fail closed. Text is valid UTF-8 and bounded to 2,048 bytes per record. A batch is
bounded to 512 records and 2 MiB of canonical bytes.

## Atomicity

Announcement fields are returned by the existing `ReadObservation` RPC. One DFHack RPC executes
under DFHack's internal suspension, so a one-page citizen observation and its announcement suffix
share one observation instant.

When citizen pagination requires multiple calls, V1.1 retains the existing paused-world rule. Every
page must reproduce the same announcement suffix and summary fields. Any drift invalidates the
assembly; no partial capsule or anchor is published.

## Coverage semantics

The announcement domain is one of:

- `complete_suffix`: every retained report after the requested cursor was returned;
- `partial_suffix`: more retained reports remain and `next_after_id` is the continuation cursor;
- `gap_before_retained_window`: the retained suffix may be complete, but older history was lost.

None of these states proves that no announcement existed before `oldest_available_id`. The canonical
world projection may prove absence only inside the explicitly covered suffix.

## Version and admission

This is bridge protocol `1.1` and bridge implementation `0.2.0`. The already admitted `1.0` tuple
remains an immutable historical admission for its exact source and plugin bytes. It does not admit
this source generation.

Protocol 1.1 requires a fresh native build receipt and a fresh disposable-fort acceptance campaign
covering cursor continuation, retained-window gaps, deterministic ordering, bounded text,
multipage paused-world stability, restart fencing, and cold-agent event orientation before any 1.1
tuple may enter the compatibility registry.

## Authority

The announcement stream adds no capability and no mutation method. The native method manifest
remains:

```text
Handshake
ReadObservation
```

`RunCommand`, Lua execution, keyboard forwarding, pause, dig, construction, labor, military,
checkpoint, restore, and arbitrary command forwarding remain absent.
