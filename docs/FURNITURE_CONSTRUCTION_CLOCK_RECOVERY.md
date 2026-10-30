# Recover an original furniture clock without placement custody

`dfmcp-build-placement-dev-server` can reopen an existing whole-batch
construction clock using its original completion monitor and clock journal,
even when the original placement journal and furniture parent file are gone.
The recovered owner can inspect local history, reconcile one original native
outcome, or request safety cancellation of that original run. It cannot place
furniture, sample construction, prepare a new interval or retry a commit.

This is an unadmitted development profile of the existing furniture/1.19,
operations/1.4 and run/1.13 composition. Start new work through the ordinary
[construction-clock workflow](FURNITURE_CONSTRUCTION_CLOCK_MCP.md); this guide
covers recovery of its already retained identity. The
[adapter contract](FURNITURE_CONSTRUCTION_CLOCK.md) explains original run keys
and uncertainty. Executed verification and its limits are recorded in
[implementation status](../IMPLEMENTATION_STATUS.md).

## Keep both independently acknowledged recovery references

Preserve these values from successful responses and authorized failure
handoffs:

| Evidence | Response field | Required contents |
| --- | --- | --- |
| Original construction monitor | `result.completion.recovery` | `monitor_id`, `head` |
| Original construction clock | `result.construction_clock.recovery` | `journal_id`, `head` |

Each reference binds an externally acknowledged head of its own original
history. An intact, semantically valid descendant may contain additional
records after that head. Reopening refuses a rollback before the acknowledged
head, a different monitor or journal, an unrelated fork, a substituted file,
an invalid record or a torn tail. It never truncates, repairs, creates or adopts
replacement custody.

The monitor must contain an accepted, source-bearing construction acquisition.
The ordinary activation workflow establishes this with its first accepted
completion sample. A fully source-validated retained readiness acquisition can
also preserve the original source; readiness never contributes positive goal
credit or becomes a completion sample. The **first accepted operations source**
is reconstructed during full replay and matched to the clock. A later sample
that terminally invalidated the monitor after a source change does not substitute
a new original source or strand the earlier clock history. The latest diagnostic
sample can therefore name a different source while the recovered clock remains
bound to the first.

Both the original monitor and clock must be available and exact when opening.
Loss of the monitor before opening is still a refusal; a clock journal alone
does not reconstruct the original furniture project. The original placement
and parent files are not required in this profile.

## Configure the existing files and select the recovery profile

Retain the original `DFMCP_ALLOW_UNADMITTED_BUILD_MCP_V1_19=1`, fortress identity,
scope and endpoint configuration. Keep all four configured path strings:

| Variable | Use during independent clock recovery |
| --- | --- |
| `DFMCP_BUILD_JOURNAL` | Must match the original placement path retained in the monitor; the file is never opened. |
| `DFMCP_BUILD_BATCH` | Must match the original furniture parent path retained in the monitor; the file is never opened. |
| `DFMCP_BUILD_COMPLETION` | Opens the existing original monitor read-only. |
| `DFMCP_BUILD_CLOCK_CONFIG.path` | Opens the existing original clock journal in the selected recovery mode. |

Do not replace the first two path strings with paths to new files. Their
retained identities remain part of the original project even when those files
have been removed. All configured paths remain normalized, absolute and
distinct. The two files that are opened must satisfy the original private-file
requirements: exact mode `0600` under owned real directories with exact mode
`0700`.

For local inspection without contacting the game:

```sh
export DFMCP_BUILD_MODE=clock-offline
export DFMCP_BUILD_CLOCK_MONITOR_RECOVERY='{"monitor_id":"<original monitor id>","head":"<acknowledged monitor head>"}'
export DFMCP_BUILD_CLOCK_CONFIG='{"path":"/private/furniture/clock","initialize":false,"source":{"endpoint":"127.0.0.1:5000","generation":41,"df_version":"<original DF version>","dfhack_version":"<original DFHack version>"},"recovery":{"journal_id":"<original clock journal id>","head":"<acknowledged clock head>"}}'
```

`DFMCP_BUILD_CLOCK_MONITOR_RECOVERY` accepts only `monitor_id` and `head`, both
nonzero SHA-256 values. It is required in `clock-offline` and `clock-recover`
and refused in every other profile. Existing clock configuration remains
closed and requires `initialize:false` with its own recovery reference.
Initialization is refused in both independent recovery modes.

Use the original run source endpoint, generation and software versions, not
the furniture or operations generation. The sources remain independently
pinned; sharing an endpoint does not establish a shared process incarnation.
No furniture or operations credential is needed for these recovery operations.
`clock-offline` needs no run credential and never creates a native connection.

For a native query of the original unresolved run, select:

```sh
export DFMCP_BUILD_MODE=clock-recover
export DFMCP_BUILD_CLOCK_TOKEN='<matching run credential, 32..256 bytes>'
```

To retain the additional authority needed for native safety cancellation, set
the independent opt-in **before opening**:

```sh
export DFMCP_BUILD_ALLOW_CLOCK=1
```

Without that opt-in, the retained clock owner is query-only. Adding the opt-in
later cannot promote that owner; close or explicitly release it and reopen the
same original files and pins with cancellation authority enabled. Removing the
opt-in takes effect at current native boundaries. It is independent of
`DFMCP_BUILD_ALLOW_PLACE`, and no placement opt-in is needed here.

The native plugin separately needs its run/1.13 opt-in and matching token as
described in [bounded simulation runs](BOUNDED_SIMULATION_RUN.md). Do not put
native-plugin environment variables in the isolated MCP server environment.
The default `required` checkpoint policy does not block these recovery-only
operations: none prepares or dispatches a new unpause interval.

## Open with zero future game ticks

```text
fortress.open_session(max_wall_millis=60000, max_game_ticks=0)
```

Omit `selection`, `furniture_plan` and `furniture_request`. A positive game-time
allowance is refused. The recovered owner retains Query authority and, only
when separately enabled at opening, ControlClock authority. It retains no
Plan, Observe or Construct authority and restores no review seal.

Opening fully replays the monitor and clock, verifies both acknowledged
histories, matches the original monitor definition and source, and checks the
two current files again after the last runtime callback. It acquires no native
observation, spatial lease or construction sample. The monitor stays read-only.
No original placement or parent path is opened, including during failure
handoffs or close.

## Inspect retained history

Use the exact current clock head returned by opening or the most recent
response, even for the first page:

```text
fortress.query(session_id,
  query='{"mode":"construction_clock","monitor_id":"<original monitor id>","head":"<current clock head>","offset":0,"limit":8}')
```

Offset is 0..63 and page size is 1..8. Use the returned continuation unchanged.
A changed head refuses a mixed-history page. Pending and unresolved state
remain visible independently of pagination. Any retained active run and
history row has `review_seal:null`.

Other local inspection operations are:

```text
fortress.query(session_id, query='{"mode":"completion"}')
fortress.query(session_id, query='{"mode":"batch"}')
fortress.query(session_id, query='{"mode":"allocation","view":"request"}')
fortress.query(session_id, query='{"mode":"schema"}')
fortress.doctor(session_id)
```

The batch view includes every original step from the monitor's immutable
origin. Allocation views expose the original sealed request or selected item
evidence; they do not allocate replacements. Completion inspection and doctor
verify the monitor's local history as well as clock custody. These operations
make no native calls and do not recover missing placement or parent files.

`history_verified` and `monitor_inventory_verified` describe the files actually
checked. They do not establish current construction, original placement-file
custody or current item availability. The response explicitly retains
`original_inventory_verified:false`, `original_placements_verified:false`,
`fresh_sample_acquired:false`, `sampled_construction_condition_proven:false`
and `current_pause_proven:false`. Historical sampled success remains historical.

## Reconcile or cancel only the original run

In `clock-recover`, one wait reconciles the retained original unresolved key:

```text
fortress.wait(session_id, scope="clock",
  monitor_id="<original monitor id>", max_wall_millis=5000)
```

It performs at most one native query. Already terminal history returns locally.
Missing native records and lost-source evidence remain unresolved; they cannot
be converted into proof of no effect or permission to dispatch again. The
operation cannot extend the interval, start another run, poll in a loop or
publish a construction sample.

For separately enabled safety cancellation:

```text
fortress.cancel(session_id, scope="clock", monitor_id="<original monitor id>")
```

A never-dispatched preparation can be retired locally. After possible dispatch,
the action asks the native source to cancel the original run. Only this safety
action may repeat; commit never retries. Both retained and current Query and
ControlClock authority are required in the named fortress domain and the
independently issued native source domain. Cancellation does not require future
game ticks, Plan or Observe authority, or an unexpired construction deadline.

If the monitor file disappears or becomes corrupt **after successful opening**,
the retained exact original clock can still reconcile or cancel. Its result
marks monitor history unverified. If the clock journal itself is damaged,
replaced or no longer exact, the owner refuses native contact. It does not send
a speculative pause based on unverified custody. Source loss also cannot prove
that the game is paused now.

The local-only `clock-offline` profile refuses both wait and clock cancellation.
Both profiles refuse new intent, preparation, commit, ordinary observation,
completion-start, completion cancellation and batch advancement. Restoring
original placement custody and resuming construction sampling requires the
ordinary workflow with its full checks.

Recovery Agent Turn recommendations stay within the retained profile: offline
or settled recovery recommends exact-head history inspection, while writable
recovery of a pending run recommends its original outcome query. Safety-cancel
arguments appear only when current and retained authority in both domains allow
that path. Closed sessions carry no further action recommendation. No recovery
response recommends fresh construction sampling or a new clock commit.

## Close or release without claiming settlement

Ordinary close requires intact clock history with no pending or unresolved run:

```text
fortress.cancel(session_id, scope="session")
```

If ownership must be released while an effect remains unresolved or custody is
damaged, explicitly request:

```text
fortress.cancel(session_id, scope="session", release_for_recovery=true)
```

Release drops local ownership without querying, cancelling, pausing, erasing or
settling the original effect. It returns unverified historical references when
disclosure is still authorized. If canonical or native Query authority has
expired or been removed, release can still succeed, but the reply withholds
cached monitor, batch and clock identities and histories. Save the original
recovery pins before losing that authority. Other requests cannot use a failure
response to disclose cached evidence after Query loss.

## Evidence and resource limits

Each operation reserves output and exact custody checks before doing work.
Monitor verification is budgeted from its actual retained byte length, including
the final check after the last runtime callback; it is not treated as a fixed
small clock journal. The shared request budget remains bounded by 1 GiB and one
shrinking wall deadline. An insufficient allowance refuses the operation rather
than skipping replay or repairing a file.

Tick floors include original placement receipts, retained monitor samples and
clock observations. They cannot move backward within the owner, including after
a rejected observation. A read that was never durably recorded does not acquire
restart durability merely because it raised that in-memory floor.

Recovery uses existing monitor and run journal formats and the existing
run/1.13 protocol. It adds no global controller fence, process identity proof,
verified checkpoint, restore provider, exact-tick guarantee or automatic
gameplay. A terminal native receipt proves its recorded original outcome;
current pause, current furniture usability, worker reachability and construction
causality still require evidence this recovery profile does not acquire.
