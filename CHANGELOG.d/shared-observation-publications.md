# Immutable live observation publications

Operations/1.3, paged operations/1.4 and spatial/1.6 and 1.8 now share each published
world graph, native observation and identity map between readers and journal
candidates. An unchanged native read reuses the current publication. Changed reads
still produce independent successors, and journal-backed sessions publish the new
anchor only after syncing its complete record. Historical snapshot reads can
transfer the replayed graph directly to their caller.
