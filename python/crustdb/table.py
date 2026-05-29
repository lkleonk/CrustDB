from __future__ import annotations

from typing import TYPE_CHECKING, Any, Type

from .model import Model
from .row import Row

if TYPE_CHECKING:
    from .database import CrustDB


class Table:
    def __init__(self, db: CrustDB, model_cls: Type[Model]) -> None:
        self.db = db
        self.model_cls = model_cls
        self.model_name = model_cls.__model_name__
        self.fields = model_cls.__fields__

    def insert(self, **kwargs: Any) -> Row:
        row = self.db._engine.insert(self.model_name, kwargs)
        return Row(row)

    def find(self, **kwargs: Any) -> Row | None:
        row = self.db._engine.find(self.model_name, kwargs)
        if row is None:
            return None
        return Row(row)

    def delete(self, **kwargs: Any) -> bool:
        return self.db._engine.delete(self.model_name, kwargs)

    def update(self, *, where: dict[str, Any], values: dict[str, Any]) -> Row | None:
        row = self.db._engine.update(self.model_name, where, values)
        if row is None:
            return None
        return Row(row)
