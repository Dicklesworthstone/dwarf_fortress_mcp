# Original project access in sampled room readiness

Beads: `df-dfhack-bridge-plane-c-pic.3/.4/.5` (WP-05/07/10).

A connected room layout can carry an original `intent.access.point`, but
room-local circulation alone does not prove a route from that point. In
particular, blocked or unknown corridor cells before the first room must not
be ignored when every room and furnishing is locally reachable.

`room_circulation.assess(..., connected=True, access_point=point)` evaluates
all entries and furnishing approaches from that exact point in the complete
original floor mask. It uses separate clear and potential cardinal graphs;
unknown or unit-occupied passages can establish only a potential route.
A blocked point cannot be silently replaced by an internal room entry. All
furnishing targets remain blocked in both graphs.

Evidence policy `dfmcp.observed-room-access-circulation/2` binds the point,
complete selection and observed partitions in the model and route digests.
The point is an independent root, not a ninth room or extra furnishing.
At maximum selection it returns 41 records: the root, eight entries and
32 furnishing approaches. Existing 512-tile and work/output limits remain.

Calls without the new point retain their previous policy, canonical bytes,
digests, work counts and room-local interpretation. An entrance model does
not prove membership in the existing fortress, native unit pathfinding,
continuous access, current safety or authority to mutate the game.

Test command:

```sh
python3 -m unittest discover -s tests -p 'test_room_access_circulation.py' -v
```
