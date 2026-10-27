# Citizen welfare indicators

The spatial/1.8 runtime exposes a bounded citizen-welfare report through the existing
`fortress.query` tool. It summarizes the complete strict-citizen roster from the last coherent
capture and gives actionable inspection requests for citizens with elevated observed stress.
It does not acquire another native observation, alter labor assignments, or claim that health
and needs have been measured. Development/admission status is unchanged.

## Use the report

The convenience mode selects observed stress categories 0 through 2 and citizens whose stress
cannot be established:

```json
{"session_id":"<spatial session>","mode":"welfare"}
```

The structured form supports stable pagination and selection:

```json
{
  "session_id":"<spatial session>",
  "query":{
    "schema":"dfmcp.query/1",
    "query":{
      "kind":"citizen_welfare",
      "max_stress_category":2,
      "include_unestablished":true,
      "limit":8
    }
  }
}
```

Set `max_stress_category` to 6 to inspect every observed category. The default is 2; lower
numbers indicate greater stress. `include_unestablished` defaults to true. `limit` is 1–128,
default 8, and `max_work` is 1–4,000,000, default 4,000,000. Requests are closed: unknown fields,
invalid types and out-of-range categories fail before analysis.

Rows sort by stress category ascending, then canonical entity ID; unknown categories follow
known ones. The summary always counts the entire captured strict roster, regardless of filtering
or pagination. It includes the seven-entry `stress_categories` histogram indexed by category,
the unestablished count, the elevated-stress count, current availability counts, and unresolved
job-worker links. Empty selected rows do not establish good wellbeing.

Each row includes a generation/revision-qualified citizen handle, source-qualified name and
position, stress category, observed status flags, availability interpretation, and an explicit
list of unestablished fields. Current job assignments have a count and at most four canonical
examples. `no_assignment_proven` is true only when no retained current job belongs to that citizen
and every current job's strict-citizen worker membership is resolved. This does not prove that
the citizen is idle, healthy or authorized to perform another job.

## Source and interpretation

The report uses policy `dfmcp.citizen-welfare/1` and only existing `spatial/1.8` facts. Each
contributing field must have its exact expected native field path, capture digest, game tick,
type and consistent known-value representation. Missing, omitted, redacted, unsupported,
stale, differently sourced and contradictory fields remain unestablished. An inconsistent
availability pair is explicitly unknown.

DFHack's `Units::getStressCategory` uses the following ordering. These ranges explain the
native category; the report does not expose or reconstruct an exact raw stress value.

| Category | Native raw stress range | Default selection |
|---|---|---|
| 0 | At least 50,000 | Included |
| 1 | At least 25,000 and below 50,000 | Included |
| 2 | At least 10,000 and below 25,000 | Included |
| 3 | At least −10,000 and below 10,000; also the no-soul fallback | Excluded when observed |
| 4 | At least −25,000 and below −10,000 | Excluded when observed |
| 5 | At least −50,000 and below −25,000 | Excluded when observed |
| 6 | Below −50,000 | Excluded when observed |

The implementation and fallback were checked against
[DFHack `Units.cpp` at commit `438faf3b87ff3a308c0a8501e91228a10cdaeec4`](https://github.com/DFHack/dfhack/blob/438faf3b87ff3a308c0a8501e91228a10cdaeec4/library/modules/Units.cpp).
Category 3 alone cannot establish that a soul was present or a stress value was measured.

The strict roster comes from the existing `Units::getCitizens(..., true, false)` acquisition.
It excludes residents, visitors, dead and insane units. Complete strict-roster capture therefore
does not mean complete population or health coverage. The response labels needs, physical health,
hunger, thirst and injuries as **unknown**, and raw stress, soul presence and other population
coverage as **not observed**. `all_clear_proven` is always false.

Availability comes from the two captured DFHack `isJobAvailable` policies. The report distinguishes
availability while preserving social activity, availability only when interrupting social activity,
unavailability, and unknown. These are observations at capture, not labor permissions, skill
qualification, causes of stress, or a recommendation to interrupt activity.

## Attention, drilldowns and history

The spatial situation policy `/2` adds `living_citizen_elevated_stress` as a high-priority
inspection signal. It requires an observed living citizen and a source-qualified category in
0–2. The existing detailed and compact briefings, full rule explanation and endpoint comparisons
include this signal without another capture. The two-group detailed-attention bound is unchanged.
See [Spatial operational situation](SPATIAL_SITUATION.md).

Each welfare row contains a ready `inspect_query` bound to the full snapshot anchor and citizen
generation. Submit that envelope as the `query` argument to `fortress.query`. After the live
observation advances, an old live drilldown fails instead of inspecting a different version.

`citizen_welfare` is also accepted inside `historical_query` and in archive-only sessions. Historical
rows rewrite their inspections into `historical_query` envelopes bound to the same exact record
number and digest. Historical results carry no current live attention. Archive reads make no native
call and do not modify the archive.

Continuations bind the complete anchor, session, source/model digest, category/unknown selection
and work allowance. Page width can change; these bindings cannot. A changed observation or selection
requires a new first page. The complete summary and metadata must fit, and pages contain only whole
rows. If even one required row cannot fit, the query returns a budget error instead of a
zero-progress continuation or silently omitted evidence. Whole-projection Query authority,
cancellation and cooperative time/work bounds are enforced before and during analysis.

## Validation scope

The welfare unit tests decode independently assembled native spatial/1.8 bytes through the real
adapter and execute the query. They cover all seven stress categories, selection, pagination and
staleness, authority/resource refusal, complete-row output bounds, exact provenance and unknown
facts, contradictory worker membership, bounded assignment examples, availability consistency
and pinned historical drilldowns. Additional situation tests cover stress priority, unknown
inputs and endpoint comparisons; runtime tests cover the live convenience mode and exact-record
historical/archive dispatch. This is read-side validation, not a live-fortress or native admission
certificate.
