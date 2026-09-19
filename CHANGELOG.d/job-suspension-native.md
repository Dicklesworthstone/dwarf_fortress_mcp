# Native job intervention, isolated development protocol 1.9

Add fixed ReadJob/PrepareSuspension/CommitSuspension/QuerySuspension methods plus
Handshake. Only paused, idle, supported jobs at completed workshops/furnaces can
change their suspension flag. Exact observed witnesses, TTL, map incarnation and
local intervention sequence are revalidated before the sole setter. Unknown
outcomes never redispatch; complete terminal evidence is independently hashable.

Execute 397 assertions on GCC and Clang with UBSan against the actual native
handler/engine and explicit DFHack/protobuf doubles. Reject three removed-gate
mutants. Commit native-emitted observation/effect vectors. This is development
source and mock-boundary execution, not a real plugin build, Rust or live-game
qualification. No read-only protocol, production runner or registry is widened.
See docs/JOB_SUSPENSION_CONTROL.md for scope and durability requirements.
