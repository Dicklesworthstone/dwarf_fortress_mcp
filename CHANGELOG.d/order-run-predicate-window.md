# Show when a sampled order condition can stop its interval

Order-run reviews now give the earliest native tick that could supply all
required samples and whether it falls before the tick limit. This helps callers
distinguish an interval that can stop on its predicate from one that can only
reach its time limit or stop for another reason.

The Python operator command `review` obtains this estimate with a read-only order observation. Stored journal inspection exposes the same fields.
