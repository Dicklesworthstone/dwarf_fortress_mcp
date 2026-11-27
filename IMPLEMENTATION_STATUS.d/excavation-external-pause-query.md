# Query a pending excavation pause

The excavation/1.18 encoder accepts the ExternalPause reason for a Stopping run. The Rust and Python decoders already accept this combination. A native RPC fixture exercises repeated queries while the modal pause masks a running raw clock, followed by confirmation through the original run. The fixture was source-reviewed and awaits execution.

Owners: `df-dfhack-bridge-plane-c-pic.4` and `df-dfhack-bridge-plane-c-pic.5`.
