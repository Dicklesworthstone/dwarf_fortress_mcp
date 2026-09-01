# Live announcement acceptance checklist

- [x] Retained-window semantics distinguish current retained evidence from complete history.
- [x] The first page freezes an inclusive report-ID high-water mark.
- [x] Canonical identity is independent of transport page size.
- [x] History truncation is explicit and prevents absence proofs.
- [x] Record text, page count, total records, and canonical bytes are bounded.
- [x] Duplicate, reordered, cursor-inconsistent, and malformed records fail closed in the Rust assembler.
- [x] The protobuf request/reply surface is frozen prospectively.
- [ ] Native DFHack extraction authenticates before report inspection.
- [ ] Safe-Rust protobuf reply codec passes malformed-wire campaigns.
- [ ] Complete-window driver freezes and validates continuations.
- [ ] Announcement entities and coverage enter canonical world state.
- [ ] The frozen MCP waist exposes announcements through observe/query/explain.
- [ ] Disposable-fort live evidence passes the versioned acceptance campaign.
- [ ] A source-, binary-, platform-, and receipt-bound exact tuple is promoted.

Unchecked items are not implemented and must not be inferred from the completed semantic layers.