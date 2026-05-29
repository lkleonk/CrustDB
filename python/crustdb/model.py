from __future__ import annotations

from .fields import Field


class ModelMeta(type):
    def __new__(
        mcls,
        name: str,
        bases: tuple[type, ...],
        attrs: dict,
        **kwargs,
    ):
        fields = {}

        for base in bases:
            fields.update(getattr(base, "__fields__", {}))

        for attr_name, attr_value in list(attrs.items()):
            if isinstance(attr_value, Field):
                fields[attr_name] = attr_value.clone_for_model(attr_name)

        attrs["__fields__"] = fields
        attrs["__model_name__"] = name
        attrs["__frozen__"] = kwargs.get("frozen", False)

        return super().__new__(mcls, name, bases, attrs)


class Model(metaclass=ModelMeta):
    pass
