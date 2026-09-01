# Live announcement read threat model

The announcement generation inherits the loopback bearer-token and exact-tuple model of the first live bridge, but its data-specific hazards differ:

- report text is attacker-influenced game data and must be treated as untrusted UTF-8 after conversion;
- report vectors can be large, pruned, reordered by malformed bridges, or extended during pagination;
- an empty page without retained-window bounds is epistemically ambiguous;
- a continuation that silently follows a moving high-water mark is not replayable;
- history loss must never be translated into evidence of absence;
- report text must not enter shell commands, logs, file names, or qualification metadata without explicit bounded encoding;
- the method must remain read-only and separate from keyboard, command, Lua, pause, designation, labor, and military routes.

The safe-Rust side therefore validates the entire page before mutating its assembler, freezes a high-water mark, rejects drift, bounds all allocations, canonicalizes only complete page sets, and carries truncation as coverage rather than a warning that agents might ignore.