# Compare historical citizen welfare

The spatial/citizen development server supports `historical_citizen_welfare`
through the existing `fortress.query` tool. It compares two exact retained
spatial/1.8 records and reports source-qualified citizen indicators at those
endpoints. It works in journal-backed live sessions and archive-only reopening,
without native capture, journal writes, watch advancement or baseline creation.

This is an unadmitted development feature. It does not establish physical health,
needs, raw stress changes, continuous trends or causes. See
[implementation status](../IMPLEMENTATION_STATUS.md) for the exact evidence scope.

## Select exact endpoint records

Use the existing `history` query to list retained records. Copy both record
numbers and their exact `record_digest` values. The `from` record must be no
later than `to`. Endpoints must retain the same accepted world, site, source
generation, software versions and observation epoch. Clock regressions and
observation resets are refused.

Send the following as the `query` argument to `fortress.query`, replacing the
illustrative digests with actual returned values:

```json
{
  "schema": "dfmcp.query/1",
  "query": {
    "kind": "historical_citizen_welfare",
    "from": {
      "record": 1,
      "record_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    },
    "to": {
      "record": 2,
      "record_digest": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    },
    "include_unchanged": true,
    "limit": 4
  }
}
```

An optional envelope `expected_anchor` names the current session authority
anchor, not either archived endpoint. Historical evidence never restores expired
Query authority. A fenced live source can still serve valid local archive
evidence without contacting that source.

## Read the complete summary and individual evidence

`summary.before` and `summary.after` describe the complete strict citizen roster
at each capture: citizen count, seven stress-category counts, unestablished
stress, categories 0–2 and job availability. Totals are computed before citizen
selection, unchanged filtering or pagination. A small page therefore retains
the whole-roster summary.

Same-generation summaries report category movement, crossings into/out of
categories 0–2, observed changes, wholly unchanged rows and rows with
unestablished indicators. Changed and unestablished counts can overlap: one
citizen may have a known availability change and unknown stress. Observed status
flags are `alive`, `sane`, `active`, `visible`, `adult`, `child` and `baby`.

| Indicator | Interpretation |
| --- | --- |
| `toward_higher_stress_category` | Movement toward numeric 0, the highest-stress end of the native category ordering. |
| `toward_lower_stress_category` | Movement toward numeric 6, the lowest-stress end of that ordering. |
| `entered_elevated_category` | The same citizen generation moved from categories 3–6 into 0–2. |
| `left_elevated_category` | The same citizen generation moved from categories 0–2 into 3–6. |
| `available_preserving_social` | Both native availability flags were true. |
| `available_only_interrupting_social` | Availability required permission to interrupt social activity. |
| `unavailable` | Both native availability flags were false; the cause is unknown. |
| `unestablished` | Required facts lack consistent source-qualified evidence. |

Category **3 may be DFHack's no-soul fallback**. Soul presence and raw stress were
not measured. Category transitions must not be described as measured stress
changes or health improvement. Availability is not labor permission, a native
job-eligibility decision or a reservation.

Rows preserve both endpoint citizen generations and distinguish the same
generation, a replaced generation, newly observed citizens and citizens no
longer observed. Reused native IDs are never compared as the original citizen.
Leaving the strict roster does not prove death, migration, insanity or a cause.

Each row contains executable `inspect_before` and `inspect_after` envelopes.
Submit either directly to `fortress.query` to inspect that exact retained record
and citizen generation. Missing endpoints have no inspection envelope and never
redirect to current state.

To follow one original citizen, add a `citizen` object containing its canonical
decimal-string `entity_id` and numeric `generation` from the **from record**.
Absent original citizens or mismatched generations are refused. The focus keeps
reporting disappearance and later ID reuse without substituting a replacement;
whole-roster summary totals remain complete.

## Bounds, filters and continuation

`limit` defaults to 4 and accepts 1–128. Responses include only complete rows
that fit the output allowance. If one row cannot fit, the query returns a budget
error instead of truncated JSON. `max_work` defaults to 4,000,000 and can be
lowered; it covers complete endpoint scans, comparison and page work.

When `continuation` is non-null, repeat the same endpoints, focus,
`include_unchanged` and `max_work` with that token. Page size may change. Tokens
bind the session, current authority anchor, archive identity and head, endpoint
sources, selection and work allowance. They cannot cross a reopen, appended
archive head or changed endpoint/selection. Reopening requires a new first page.

`include_unchanged=false` removes only same-generation rows whose compared
indicators are all known and unchanged. Missing fields, replaced generations
and roster transitions remain. Unknown evidence cannot become a healthy or
unchanged finding because of filtering.

## Limits of two observations

Agent Turn continuity is partial and coverage is selected endpoints only. Both
source digests, exact record witnesses and elapsed endpoint game ticks are
returned, but intervening events are unknown. Same-tick endpoints do not prove a
continuous interval. No result establishes current freshness, cause, physical
health, needs, raw stress change or that the fortress is all clear. Strict-roster
completeness does not extend to other population groups.

The closed request schema is
[`mcp_historical_citizen_welfare_v1.json`](../schemas/mcp_historical_citizen_welfare_v1.json).
