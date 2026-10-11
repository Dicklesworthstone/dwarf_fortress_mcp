"""Independent exhaustive model, not execution of the repository's Rust source."""
from itertools import product

# Each step advances one cursor and one tick unless it injects a discontinuity.
# Qualified clearance requires two clear samples spanning at least two ticks.
# An oracle retains the actual suffix instead of a counter/start accumulator.
events = ['clear', 'present', 'unknown', 'partial', 'gap', 'source', 'roster', 'epoch', 'paused']
count = 0
for history in product(events, repeat=5):
    samples = 0
    since = None
    tick = 0
    suffix = []
    for event in history:
        if event != 'paused':
            tick += 1
        complete = event != 'partial'
        threat = event if event in ('present', 'unknown') else 'clear'
        continuous = event not in ('gap', 'source', 'roster', 'epoch', 'paused')
        if not continuous:
            samples = 0
            since = None
        if threat == 'clear' and complete:
            if since is None:
                since = tick
            samples += 1
        else:
            samples = 0
            since = None
        ready = threat == 'clear' and complete and samples >= 2 and since is not None and tick - since >= 2
        if not continuous or threat != 'clear' or not complete:
            suffix = []
        if threat == 'clear' and complete:
            suffix.append(tick)
        expected = len(suffix) >= 2 and tick - suffix[0] >= 2
        assert ready == expected, (history, event, samples, since, suffix)
        if ready:
            assert complete and threat == 'clear' and len(suffix) >= 2
        count += 1
print(f'PASS: {count} modeled alert transitions checked against a clear-suffix oracle (not Rust execution)')
