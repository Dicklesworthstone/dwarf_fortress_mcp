### Bounded lossless journal payload codec

- Add the dependency-free `dfmcp_world::journal_delta` byte codec for unchanged
  spans and shifted suffixes, with deterministic raw fallback unless at least
  128 bytes are saved. Bounds: 16 MiB expanded payload and 32,768 commands.
- Validate every command and input/output range before allocating reconstructed
  output. The enclosing journal must still bind the predecessor and verify
  checksums, native source identity and canonical anchors.
- Register eight Rust test groups, including independent fixed vectors, 2,048
  deterministic edit cases, truncation, fragmentation and maximum-size input.
  Rust/Cargo/rustfmt are unavailable; none of the Rust tests was executed here.
- The checked-in independent Python reference passed 15,625 checks, including
  3,800 compressed roundtrips and 348 raw fallbacks. This is reference evidence,
  not Rust, filesystem, MCP or live-game execution.
- This codec commit alone does not change any archive writer or file format;
  integration must retain exact replay, current authority, publication ordering,
  old-profile framing and fail-closed handling by older readers.
