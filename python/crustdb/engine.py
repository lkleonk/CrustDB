from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from .exceptions import CrustDBError, UniqueConstraintError, ValidationError


def create_engine(path: str | Path, engine: str = "native"):
    if engine == "json":
        return JsonEngine(path)

    if engine != "native":
        raise CrustDBError(
            f"Unknown engine: {engine!r}. Expected 'native' or 'json'."
        )

    try:
        NativeEngineBinding = _load_native_engine_binding()
    except ImportError as exc:
        raise CrustDBError(
            "Native engine is not available. Build it with "
            "`uv run --with maturin maturin develop`, or explicitly use "
            "`CrustDB(path, engine='json')` for the development JSON engine."
        ) from exc

    return NativeEngine(path, NativeEngineBinding)


def _load_native_engine_binding():
    from ._native import Engine as NativeEngineBinding

    return NativeEngineBinding


class NativeEngine:
    def __init__(self, path: str | Path, binding: type) -> None:
        self._engine = binding(str(path))

    def register_schema(self, model_name: str, schema: dict[str, Any]) -> None:
        payload = {"name": model_name, "fields": schema["fields"]}
        self._call(self._engine.register_schema, json.dumps(payload))

    def insert(self, model_name: str, values: dict[str, Any]) -> dict[str, Any]:
        row_json = self._call(self._engine.insert, model_name, json.dumps(values))
        return json.loads(row_json)

    def find(self, model_name: str, filters: dict[str, Any]) -> dict[str, Any] | None:
        row_json = self._call(self._engine.find, model_name, json.dumps(filters))
        if row_json is None:
            return None
        return json.loads(row_json)

    def delete(self, model_name: str, filters: dict[str, Any]) -> bool:
        return self._call(self._engine.delete, model_name, json.dumps(filters))

    def update(
        self,
        model_name: str,
        filters: dict[str, Any],
        values: dict[str, Any],
    ) -> dict[str, Any] | None:
        row_json = self._call(
            self._engine.update,
            model_name,
            json.dumps(filters),
            json.dumps(values),
        )
        if row_json is None:
            return None
        return json.loads(row_json)

    def _call(self, func, *args):
        try:
            return func(*args)
        except ValueError as exc:
            message = str(exc)
            if message.startswith("Duplicate value for unique field:"):
                raise UniqueConstraintError(message) from exc
            raise ValidationError(message) from exc


class JsonEngine:
    def __init__(self, path: str | Path) -> None:
        self.path = Path(path)
        self.path.mkdir(parents=True, exist_ok=True)
        self._schemas: dict[str, dict[str, Any]] = {}
        self._rows: dict[str, list[dict[str, Any]]] = {}

    def register_schema(self, model_name: str, schema: dict[str, Any]) -> None:
        self._schemas[model_name] = schema
        self._rows[model_name] = self._load_rows(model_name)
        self._write_schema()

    def insert(self, model_name: str, values: dict[str, Any]) -> dict[str, Any]:
        row = self._prepare_row(model_name, values)
        self._validate_unique(model_name, row)
        self._rows.setdefault(model_name, []).append(row)
        self._save_rows(model_name)
        return dict(row)

    def find(self, model_name: str, filters: dict[str, Any]) -> dict[str, Any] | None:
        schema = self._schema(model_name)
        self._validate_filters(schema, filters, "find")

        for row in self._rows.get(model_name, []):
            if all(row.get(name) == value for name, value in filters.items()):
                return dict(row)
        return None

    def delete(self, model_name: str, filters: dict[str, Any]) -> bool:
        schema = self._schema(model_name)
        self._validate_filters(schema, filters, "find")

        for index, row in enumerate(self._rows.get(model_name, [])):
            if all(row.get(name) == value for name, value in filters.items()):
                del self._rows[model_name][index]
                self._save_rows(model_name)
                return True
        return False

    def update(
        self,
        model_name: str,
        filters: dict[str, Any],
        values: dict[str, Any],
    ) -> dict[str, Any] | None:
        if not values:
            raise ValidationError("update requires at least one value")

        schema = self._schema(model_name)
        self._validate_filters(schema, filters, "find")

        rows = self._rows.get(model_name, [])
        for index, row in enumerate(rows):
            if all(row.get(name) == value for name, value in filters.items()):
                updated = self._prepare_updated_row(model_name, row, values)
                self._validate_unique(model_name, updated, ignore_index=index)
                rows[index] = updated
                self._save_rows(model_name)
                return dict(updated)
        return None

    def _prepare_row(self, model_name: str, values: dict[str, Any]) -> dict[str, Any]:
        schema = self._schema(model_name)
        fields = schema["fields"]

        unknown = sorted(set(values) - set(fields))
        if unknown:
            raise ValidationError(f"Unknown field(s): {', '.join(unknown)}")

        row = {}
        for field_name, field in fields.items():
            if field_name in values:
                value = values[field_name]
            elif field["default"] is not None:
                value = field["default"]
            elif field["required"]:
                raise ValidationError(f"Missing required field: {field_name}")
            else:
                value = None

            self._validate_value(field_name, field, value)
            row[field_name] = value

        return row

    def _prepare_updated_row(
        self,
        model_name: str,
        existing: dict[str, Any],
        values: dict[str, Any],
    ) -> dict[str, Any]:
        schema = self._schema(model_name)
        fields = schema["fields"]

        unknown = sorted(set(values) - set(fields))
        if unknown:
            raise ValidationError(f"Unknown field(s): {', '.join(unknown)}")

        updated = dict(existing)
        updated.update(values)

        row = {}
        for field_name, field in fields.items():
            if field_name in updated:
                value = updated[field_name]
            elif field["default"] is not None:
                value = field["default"]
            elif field["required"]:
                raise ValidationError(f"Missing required field: {field_name}")
            else:
                value = None

            self._validate_value(field_name, field, value)
            row[field_name] = value

        return row

    def _validate_value(
        self,
        field_name: str,
        field: dict[str, Any],
        value: Any,
    ) -> None:
        if value is None:
            if field["required"]:
                raise ValidationError(f"Missing required field: {field_name}")
            return

        field_type = field["type"]
        if field_type == "Int":
            if isinstance(value, bool) or not isinstance(value, int):
                raise ValidationError(
                    f"{field_name} must be Int, got {type(value).__name__}"
                )

            if field["range"] is not None:
                min_value, max_value = field["range"]
                if value < min_value or value > max_value:
                    raise ValidationError(
                        f"{field_name} must be between {min_value} and {max_value}"
                    )
            return

        if field_type == "String":
            if not isinstance(value, str):
                raise ValidationError(
                    f"{field_name} must be String, got {type(value).__name__}"
                )
            return

        raise ValidationError(f"Unsupported field type: {field_type}")

    def _validate_unique(
        self,
        model_name: str,
        new_row: dict[str, Any],
        ignore_index: int | None = None,
    ) -> None:
        fields = self._schema(model_name)["fields"]
        unique_fields = [
            name
            for name, field in fields.items()
            if field["id"] or field["unique"]
        ]

        for index, existing in enumerate(self._rows.get(model_name, [])):
            if index == ignore_index:
                continue
            for field_name in unique_fields:
                if existing.get(field_name) == new_row.get(field_name):
                    raise UniqueConstraintError(
                        f"Duplicate value for unique field: {field_name}"
                    )

    def _validate_filters(
        self,
        schema: dict[str, Any],
        filters: dict[str, Any],
        operation: str,
    ) -> None:
        if not filters:
            raise ValidationError(f"{operation} requires at least one field filter")

        unknown = sorted(set(filters) - set(schema["fields"]))
        if unknown:
            raise ValidationError(f"Unknown field(s): {', '.join(unknown)}")

    def _schema(self, model_name: str) -> dict[str, Any]:
        try:
            return self._schemas[model_name]
        except KeyError as exc:
            raise ValidationError(f"Unknown model: {model_name}") from exc

    def _table_path(self, model_name: str) -> Path:
        return self.path / f"{model_name}.json"

    @property
    def _schema_path(self) -> Path:
        return self.path / "schema.json"

    def _load_rows(self, model_name: str) -> list[dict[str, Any]]:
        path = self._table_path(model_name)
        if not path.exists():
            return []

        data = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(data, list):
            raise ValidationError(f"Invalid table file for {model_name}")
        return data

    def _save_rows(self, model_name: str) -> None:
        path = self._table_path(model_name)
        path.write_text(
            json.dumps(self._rows.get(model_name, []), indent=2, sort_keys=True),
            encoding="utf-8",
        )

    def _write_schema(self) -> None:
        self._schema_path.write_text(
            json.dumps(self._schemas, indent=2, sort_keys=True),
            encoding="utf-8",
        )
