# Live announcement read generation

This document defines the next read-only bridge generation after the first admitted citizen-roster slice. It is prospective until a clean source revision, native plugin binary, disposable-fort acceptance campaign, and exact compatibility entry establish it.

## Purpose

Announcements are the smallest high-value event domain that materially improves cold-agent orientation without introducing mutation authority. They expose attacks, deaths, mandates, arrivals, cancellations, and other fortress events that cannot be inferred from the complete citizen roster alone.

The bridge must not pretend that Dwarf Fortress retains complete history. Its report vector is a bounded retained window whose oldest element may advance. The protocol therefore models a retained-window witness, not an eternal event log.

## Cursor contract

A request names:

- `after_report_id`: the greatest report ID already incorporated by the caller, or `-1` for bootstrap;
- `through_report_id`: a frozen inclusive high-water mark, or `-1` on the first page to select the current retained maximum;
- `max_announcements`: a positive bounded page size;
- the negotiated protocol, client nonce, and bearer token.

The first accepted page selects and returns `window_latest_report_id`. Every continuation request must repeat that value as `through_report_id`. Reports appended later are outside the frozen window and cannot perturb pagination identity.

The reply names:

- `requested_after_report_id`;
- `oldest_retained_report_id` and `latest_retained_report_id` for all retained reports;
- `window_latest_report_id` for the frozen page set;
- `next_after_report_id`, equal to the last returned announcement ID or the requested cursor when no record is returned;
- `history_truncated`, which is true when the caller's non-bootstrap cursor predates the oldest retained report;
- `complete`, which is true exactly when no retained announcement in `(next_after_report_id, window_latest_report_id]` remains;
- records in strict ascending report-ID order.

A caller must discard the entire candidate window if any page changes the bridge generation, nonce, frozen high-water mark, retained-window bounds, or canonical record ordering. A truncated history is publishable only as explicit partial coverage; it can never prove absence before `oldest_retained_report_id`.

## Record shape

Each record carries only bounded read-only facts:

- report ID;
- announcement type as the raw stable integer exposed by the exact DF/DFHack tuple;
- UTF-8 text truncated only at a valid code point boundary;
- year and year tick;
- position plus an explicit `has_position` bit;
- repeat count;
- continuation, unconscious, and announcement flags.

Display color and duration are intentionally omitted from the first semantic generation because they do not improve control decisions enough to justify canonical-state width. They may be added only by a later versioned generation.

## Canonical identity

Canonical announcement bytes include:

1. a versioned domain separator;
2. exact bridge, DFHack, and Dwarf Fortress versions;
3. bridge generation;
4. fortress identity;
5. retained-window bounds and frozen high-water mark;
6. truncation and completeness posture;
7. every record field in strict report-ID order.

Transport page size is not semantic. The same frozen retained window returned in one page or many must produce byte-identical canonical bytes and one SHA-256 digest.

## Coverage semantics

`fortress.announcements.retained_window` is:

- `complete` only for the frozen retained interval `(after_report_id, window_latest_report_id]` when `history_truncated` is false;
- `partial` when the caller's cursor predates retained history;
- never evidence of complete fortress history.

The agent-facing packet must report the oldest retained ID, frozen latest ID, next cursor, and whether the result can prove absence inside the named interval. It must not translate an empty retained page into “nothing happened” without those boundaries.

## Authority and failure posture

The generation adds one RPC method, `ReadAnnouncements`, and no mutation methods. Authentication precedes report-vector inspection. Rejected calls disclose no report text, retained bounds, world identity, or world posture. The Rust client treats every reply as untrusted and validates duplicate fields, minimal varints, UTF-8, record bounds, cursor monotonicity, high-water stability, and strict ordering before publication.

## Acceptance campaign

A future exact tuple must establish at least:

1. missing, malformed, and wrong credentials disclose no report posture;
2. page sizes `1`, `2`, `7`, `64`, and the hard maximum reproduce one canonical window digest;
3. reports appended after the first page do not enter the frozen window;
4. restart changes bridge generation and invalidates continuation;
5. deliberate retained-history truncation is surfaced as partial coverage;
6. duplicate, reordered, oversized, invalid-UTF-8, and cursor-inconsistent replies fail closed in Rust;
7. no announcement text or bearer token appears in qualification command lines or secret scans;
8. the complete MCP Agent Turn exposes useful recent events while retaining an empty mutation-capability set.

Until that campaign passes, announcements remain an unadmitted prospective domain.