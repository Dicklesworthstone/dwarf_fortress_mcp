# Excavation-conditioned clock control: native source and double-based execution

`docs/EXCAVATION_RUN.md` and `docs/EXCAVATION_RUN_NATIVE.md` specify the bounded
floor-condition engine and isolated six-method excavation-run/1.18 native plugin.
The native update owner advances toward sampled dry visible floor satisfaction
and stops on limits, unknown/wet targets, capture failure or cancellation. Source
changes never pause a replacement; failed pause verification retains ownership.
Clock safety remains independent of terrain acquisition. Query/terminal replay
never samples or repeats an unpause. Unload is vetoed until the owner drains.

GCC and Clang each execute 7030 engine plus 961 actual-handler assertions, with
warnings denied and nonrecovering UBSan. Five native encoding vectors match an
independent Python reconstruction. Eight compiled mutants fail. SDK/protobuf
interfaces are explicit doubles, not an actual DFHack or generated-protobuf build.

No Rust adapter, MCP route, durable external coordinator, global clock fence,
checkpoint, mining causality, structural safety or continuous-history proof is
supplied. No live fortress, physical power-loss or full qualification was run.
Existing native profiles, dependencies, compatibility registry and production
runner map are unchanged. No live tuple is admitted. Work items
`df-dfhack-bridge-plane-c-pic.4/.5` and `df-action-coordinator-exec-ero.4` stay open.
