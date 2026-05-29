from __future__ import annotations

from copy import copy
from typing import Any

from .exceptions import ValidationError


class Field:
    db_type = "Field"
    python_type: type[Any] | tuple[type[Any], ...] = object

    def __init__(
        self,
        *,
        id: bool = False,
        unique: bool = False,
        required: bool = True,
        range: tuple[Any, Any] | None = None,
        default: Any = None,
    ) -> None:
        self.name: str | None = None
        self.id = id
        self.unique = unique
        self.required = required
        self.range = range
        self.default = default

    def clone_for_model(self, name: str) -> Field:
        field = copy(self)
        field.name = name
        return field

    def has_default(self) -> bool:
        return self.default is not None

    def get_default(self) -> Any:
        if callable(self.default):
            return self.default()
        return self.default

    def validate(self, value: Any) -> None:
        if self.python_type is not object and not isinstance(value, self.python_type):
            raise ValidationError(
                f"{self.name} must be {self.db_type}, got {type(value).__name__}"
            )

    def to_schema(self) -> dict[str, Any]:
        range_value = list(self.range) if self.range is not None else None
        return {
            "type": self.db_type,
            "id": self.id,
            "unique": self.unique,
            "required": self.required,
            "range": range_value,
            "default": self.default if not callable(self.default) else None,
        }


class Int(Field):
    db_type = "Int"
    python_type = int

    def validate(self, value: Any) -> None:
        if isinstance(value, bool) or not isinstance(value, int):
            raise ValidationError(
                f"{self.name} must be Int, got {type(value).__name__}"
            )

        if self.range is not None:
            min_value, max_value = self.range
            if value < min_value or value > max_value:
                raise ValidationError(
                    f"{self.name} must be between {min_value} and {max_value}"
                )


class String(Field):
    db_type = "String"
    python_type = str
