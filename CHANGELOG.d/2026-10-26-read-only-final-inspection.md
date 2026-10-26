## Inspect all original construction without clock permission

- Add `observe-final` to the existing progress CLI using isolated route read credentials.
- Reuse the complete final-construction reader; no run token, confirmation, clock
  directory, clock contact or game-time advance is needed.
- Preserve original state, bounded output, current authority and custody through
  one output attempt. Reject clock/paging overrides before opening the batch.
- Twenty-nine routing/CLI tests and four assertion-rejected mutants pass; legacy
  next-step review/query fixture packets are byte-identical to the prior source.
- Infrastructure/codecs are explicit test ports; no native/live qualification claim.
