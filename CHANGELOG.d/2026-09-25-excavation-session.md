# Foreground excavation-run sessions

Connect the existing 1.18 native source, durable coordinator and private storage
through fixed offline/recover/control sessions. Local sealed reviews are consumed
before one-shot dispatch; native receipt discovery, cancellation and forced
recovery handoff preserve unresolved work without repeating unpause. Revalidate
exact retained inventory under the same private-file lock used for dispatch.

Reserve complete response and both custody reads before work, retain attempted
identity after uncertain final reads, and apply current authority at a monotonic
historical tick floor. No native wire, dependency or production admission change.
Fourteen added Rust regression groups remain uncompiled/unexecuted. See
`docs/EXCAVATION_RUN_SESSION.md` for the precise source and evidence limits.
