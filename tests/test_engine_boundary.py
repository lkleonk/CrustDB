import pytest

from crustdb import CrustDB, Int, Model, String, ValidationError


class User(Model):
    id = Int(id=True)
    username = String(unique=True)
    age = Int(range=(0, 120))


def test_database_owns_engine_boundary(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)

    assert hasattr(db._engine, "register_schema")
    assert hasattr(db._engine, "insert")
    assert hasattr(db._engine, "find")


def test_table_delegates_validation_to_engine(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)

    with pytest.raises(ValidationError, match="age must be between 0 and 120"):
        db.User.insert(id=1, username="alice", age=121)
