#!/usr/bin/env python3
"""Independent temporal-watch models. This does NOT execute the Rust implementation."""
from __future__ import annotations
import hashlib
import itertools
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def model(events, *, required=2, cadence=1, deadline=8):
    """Baseline is tick zero, never counted. Events contain tick/truth/status."""
    last_counted = 0
    streak = []
    for number, (tick, truth, kind) in enumerate(events, 2):
        if kind == 'segment':
            return 'continuity_lost', number, []
        if tick > deadline:
            return 'expired', number, []
        if kind != 'normal':
            return kind, number, []
        if not truth:
            streak.clear()
        elif tick >= last_counted + cadence:
            streak.append(number)
            last_counted = tick
            if len(streak) == required:
                return 'satisfied_observation', number, streak
        if tick == deadline:
            return 'expired', number, []
    return 'pending', len(events) + 1, streak


def run_length_oracle(bits, required):
    # Earliest all-true window, independent of the streaming streak counter.
    for end in range(required, len(bits) + 1):
        if all(bits[end - required:end]):
            return 'satisfied_observation', end + 1, list(range(end - required + 2, end + 2))
    return 'expired', len(bits) + 1, []


def main():
    checks = 0
    for size in range(1, 9):
        for bits in itertools.product((False, True), repeat=size):
            for required in range(1, min(size, 4) + 1):
                events = [(tick, truth, 'normal') for tick, truth in enumerate(bits, 1)]
                assert model(events, required=required, deadline=size) == run_length_oracle(bits, required)
                checks += 1
    controls = [
        ([(0, True, 'normal'), (1, True, 'normal'), (1, True, 'normal')], {}, ('pending', 4, [3])),
        ([(2, True, 'normal'), (3, False, 'normal'), (4, True, 'normal'), (6, True, 'normal')],
         {'cadence': 2}, ('satisfied_observation', 5, [4, 5])),
        ([(1, True, 'normal'), (2, True, 'normal')], {'deadline': 2}, ('satisfied_observation', 3, [2, 3])),
        ([(1, True, 'normal'), (2, False, 'normal')], {'deadline': 2}, ('expired', 3, [])),
        ([(3, True, 'normal')], {'deadline': 2}, ('expired', 2, [])),
        ([(1, True, 'normal'), (2, True, 'segment'), (3, True, 'normal')], {}, ('continuity_lost', 3, [])),
        ([(1, True, 'missing_outcome_unknown')], {'required': 1}, ('missing_outcome_unknown', 2, [])),
        ([(1, True, 'configuration_changed')], {'required': 1}, ('configuration_changed', 2, [])),
        ([(1, True, 'counter_increased')], {'required': 1}, ('counter_increased', 2, [])),
        ([(1, True, 'normal'), (2, True, 'missing_outcome_unknown')], {'required': 1}, ('satisfied_observation', 2, [2])),
    ]
    for events, options, expected in controls:
        assert model(events, **options) == expected
    # Closed predicate arithmetic: unvalidated evidence never satisfies; active
    # at zero remaining is not an active-production observation.
    predicates = 0
    for flags, remaining, threshold in itertools.product(range(4), range(6), range(6)):
        validated = bool(flags & 1)
        active = bool(flags & 2)
        actual = (validated, validated and active and remaining > 0,
                  validated and remaining <= threshold)
        expected = (flags in (1, 3), flags == 3 and remaining != 0,
                    flags in (1, 3) and remaining in range(threshold + 1))
        assert actual == expected
        predicates += 1
    files = ['crates/dfmcp-adapter/src/work_order_progress/watches.rs',
             'crates/dfmcp-adapter/src/work_order_progress/watches_tests.rs',
             'scripts/check_progress_watch_reference.py']
    for name in files[:2]:
        text = (ROOT / name).read_text()
        assert '\0' not in text
        assert not any(token in text for token in ('unsafe {', 'todo!(', 'unimplemented!(', '.unwrap(', '.expect('))
    report = {'schema': 'dfmcp.progress-watch-reference/1', 'status': 'passed_reference_only',
              'truth_schedule_cases': checks, 'deadline_cadence_discontinuity_controls': len(controls),
              'predicate_cases': predicates, 'rust_groups_registered': 11,
              'rust_compiled': False, 'rust_tests_executed': False,
              'native_or_filesystem_or_mcp_executed': False,
              'scope': 'Independent Python temporal and predicate models, not Rust execution',
              'source_sha256': {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in files}}
    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
