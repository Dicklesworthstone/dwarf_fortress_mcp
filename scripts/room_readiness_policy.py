"""Closed durable goal generations; old room-local proofs never gain access scope.

No native effects or authority. Beads: df-dfhack-bridge-plane-c-pic.3/.4/.5.
"""
from __future__ import annotations

from dataclasses import dataclass

POLICY = 'dfmcp.original-room-terrain-and-furnishings/1'
CIRCULATION_POLICY = 'dfmcp.original-room-terrain-furnishings-and-circulation/2'
ACCESS_POLICY = 'dfmcp.original-room-terrain-furnishings-and-project-access/3'
INNER_MAGICS = (b'DFMRDG01', b'DFMRDG02', b'DFMRDG03')
OUTER_MAGICS = (b'DFMRDC01', b'DFMRDC02', b'DFMRDC03')


@dataclass(frozen=True)
class ReadinessPolicy:
    circulation: bool = False
    project_access: bool = False

    def __post_init__(self) -> None:
        if (type(self.circulation) is not bool or type(self.project_access) is not bool
                or (self.project_access and not self.circulation)):
            raise ValueError('project access requires an exact enabled circulation policy')

    @property
    def version(self) -> int:
        self.__post_init__()
        return 3 if self.project_access else 2 if self.circulation else 1

    @property
    def name(self) -> str:
        return (POLICY, CIRCULATION_POLICY, ACCESS_POLICY)[self.version - 1]

    def magic(self, *, original: bool = False) -> bytes:
        if type(original) is not bool:
            raise ValueError('exact goal envelope kind required')
        return (OUTER_MAGICS if original else INNER_MAGICS)[self.version - 1]

    @classmethod
    def decode(cls, magic: bytes, *, original: bool = False) -> ReadinessPolicy:
        if type(original) is not bool or type(magic) is not bytes:
            raise ValueError('exact goal envelope kind and bytes required')
        choices = OUTER_MAGICS if original else INNER_MAGICS
        if magic not in choices:
            raise ValueError('wrong room readiness goal generation')
        version = choices.index(magic) + 1
        return cls(version >= 2, version == 3)

    def access_point(self, body: dict) -> tuple[int, int, int] | None:
        """Bind only the regenerated original intent, never a runtime-selected root.

        The enclosing Goal must revalidate its entire RoomPlan before this call.
        Legacy goals deliberately retain their old room-local interpretation.
        """
        self.__post_init__()
        if not self.project_access:
            return None
        if type(body) is not dict or type(body.get('intent')) is not dict:
            raise ValueError('project access requires the complete original room intent')
        intent = body['intent']
        access = intent.get('access')
        if intent.get('connect_areas') is not True or type(access) is not dict:
            raise ValueError('project access requires a connected original entrance-backed plan')
        point = access.get('point')
        if (type(point) is not list or len(point) != 3
                or any(type(n) is not int or not 1 <= n <= 32766 for n in point)):
            raise ValueError('invalid original project access point')
        return tuple(point)
