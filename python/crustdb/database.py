from __future__ import annotations

from pathlib import Path
from typing import Type

from .engine import create_engine
from .model import Model
from .table import Table


class CrustDB:
    def __init__(self, path: str | Path, *, engine: str = "native") -> None:
        self.path = Path(path)
        self.path.mkdir(parents=True, exist_ok=True)
        self._models: dict[str, Type[Model]] = {}
        self._tables: dict[str, Table] = {}
        self._engine = create_engine(self.path, engine=engine)

    def register(self, model_cls: Type[Model]) -> None:
        model_name = model_cls.__model_name__
        self._models[model_name] = model_cls
        self._engine.register_schema(model_name, self._schema_for(model_cls))

        table = Table(db=self, model_cls=model_cls)
        self._tables[model_name] = table
        setattr(self, model_name, table)

    def table_path(self, model_name: str) -> Path:
        return self.path / f"{model_name}.json"

    @property
    def schema_path(self) -> Path:
        return self.path / "schema.json"

    def _schema_for(self, model_cls: Type[Model]) -> dict:
        return {
            "fields": {
                field_name: field.to_schema()
                for field_name, field in model_cls.__fields__.items()
            }
        }
