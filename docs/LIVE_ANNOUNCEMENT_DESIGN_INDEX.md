# Live announcement generation index

- Machine contract: `architecture/live_announcement_read_v1.json`
- Design and acceptance model: `docs/LIVE_ANNOUNCEMENT_READ_GENERATION.md`
- Agent epistemics: `docs/ANNOUNCEMENT_WINDOW_AGENT_SEMANTICS.md`
- Threat model: `docs/LIVE_ANNOUNCEMENT_SECURITY_MODEL.md`
- State machine: `docs/LIVE_ANNOUNCEMENT_PROTOCOL_STATE_MACHINE.md`
- Implementation checklist: `docs/LIVE_ANNOUNCEMENT_ACCEPTANCE_CHECKLIST.md`
- Safe-Rust retained-window assembler: `crates/dfmcp-adapter/src/live_announcements.rs`
- Static checker: `scripts/check_live_announcement_contract.py`
- Negative checker tests: `scripts/test_live_announcement_contract.py`
- Source qualification: `scripts/qualify_live_announcement_generation.sh`

All files describe a prospective protocol-1.1 generation. They do not modify the evidence or authority of protocol 1.0.