from __future__ import annotations

from collections.abc import Iterator
from typing import Any


class Row:
    def __init__(self, values: dict[str, Any]) -> None:
        object.__setattr__(self, "_values", dict(values))

    def __getattr__(self, name: str) -> Any:
        values = object.__getattribute__(self, "_values")
        try:
            return values[name]
        except KeyError as exc:
            raise AttributeError(name) from exc

    def __getitem__(self, name: str) -> Any:
        return self._values[name]

    def __iter__(self) -> Iterator[str]:
        return iter(self._values)

    def __repr__(self) -> str:
        values = ", ".join(f"{key}={value!r}" for key, value in self._values.items())
        return f"Row({values})"

    def to_dict(self) -> dict[str, Any]:
        return dict(self._values)
