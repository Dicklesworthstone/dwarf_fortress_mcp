# Confirmed original-prefix designation

Beads: `df-dfhack-bridge-plane-c-pic.4` and `.5` (partial progress).

`dig_blueprint_prefix.Prefix` binds an exact consecutive set of 1..8 NEW native
rectangles from the existing floor-blueprint batch. Its confirmation covers the
original batch, layout, complete inventory digest, keys, target geometry, hidden
context policy and the existing disposable-fortress checkpoint policy.

The confirmation explicitly selects fresh per-step native observation and
revalidation of that fixed target prefix. It is not a collection of pre-observed
native witnesses, an arbitrary future action grant, or proof that the remaining
terrain is safe. A changed frontier requires a newly reviewed confirmation.
No count is silently clipped, no room/rectangle is omitted, and no original key
is changed or reused. Original room-backed inventory remains bound as well.

The successor check allows exactly one additional original record. All earlier
receipts, original room intent, policy and source floors remain unchanged or
monotone as appropriate. Only `designated` permits another step. `unknown` and
`refused` stop progression, and stopped or completed batches cannot start again.
These checks supplement the existing private-file/receipt replay; untrusted
caller-authored inventory is not authority.

The pure policy is implemented and has 22 passing Python tests. Its effect-shell
and command integration are a separate increment. No existing command, native
protocol, one-shot connection rule, journal format or admission gate is changed
by this policy module. There is no live-game or Rust qualification claim.

```sh
python3 -m unittest discover -s tests -p 'test_dig_blueprint_prefix.py' -v
```
