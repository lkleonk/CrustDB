import json

import pytest

import crustdb.engine as engine_module
from crustdb import CrustDB, CrustDBError, Int, Model, String


class User(Model):
    id = Int(id=True)
    username = String(unique=True)
    age = Int(range=(0, 120))


def test_rows_persist_after_reopen(tmp_path):
    db_path = tmp_path / "app.crustdb"

    db = CrustDB(db_path)
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)

    reopened = CrustDB(db_path)
    reopened.register(User)
    row = reopened.User.find(id=1)

    assert row is not None
    assert row.username == "alice"
    assert row.age == 25


def test_schema_is_written(tmp_path):
    db_path = tmp_path / "app.crustdb"

    db = CrustDB(db_path)
    db.register(User)

    assert db._engine.__class__.__name__ == "NativeEngine"
    assert (db_path / "schema.crust").exists()
    assert (db_path / "manifest.crust").exists()


def test_json_engine_writes_json_files_when_requested(tmp_path):
    db_path = tmp_path / "app.crustdb"

    db = CrustDB(db_path, engine="json")
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)

    schema = json.loads((db_path / "schema.json").read_text(encoding="utf-8"))
    assert schema["User"]["fields"]["id"]["type"] == "Int"
    assert schema["User"]["fields"]["username"]["unique"] is True

    rows = json.loads((db_path / "User.json").read_text(encoding="utf-8"))
    assert rows == [{"age": 25, "id": 1, "username": "alice"}]
    assert not (db_path / "manifest.crust").exists()


def test_missing_native_engine_does_not_fallback_to_json(tmp_path, monkeypatch):
    def raise_import_error():
        raise ImportError("missing native test double")

    monkeypatch.setattr(
        engine_module,
        "_load_native_engine_binding",
        raise_import_error,
    )

    with pytest.raises(CrustDBError, match="Native engine is not available"):
        CrustDB(tmp_path / "app.crustdb")

    assert not (tmp_path / "app.crustdb" / "schema.json").exists()


def test_native_engine_writes_phase_3_storage_files(tmp_path):
    db_path = tmp_path / "app.crustdb"

    db = CrustDB(db_path)
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)

    if db._engine.__class__.__name__ != "NativeEngine":
        pytest.skip("native engine is not built")

    assert (db_path / "manifest.crust").exists()
    assert (db_path / "schema.crust").exists()
    assert (db_path / "tables" / "User.tbl").exists()
    assert not (db_path / "User.json").exists()


def test_native_engine_writes_phase_6_index_files(tmp_path):
    db_path = tmp_path / "app.crustdb"

    db = CrustDB(db_path)
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)

    if db._engine.__class__.__name__ != "NativeEngine":
        pytest.skip("native engine is not built")

    assert (db_path / "indexes" / "manifest.crustix").exists()
    assert (db_path / "indexes" / "User" / "id.idx").exists()
    assert (db_path / "indexes" / "User" / "username.idx").exists()
    assert not (db_path / "indexes" / "User" / "age.idx").exists()

    reopened = CrustDB(db_path)
    reopened.register(User)
    row = reopened.User.find(username="alice")

    assert row is not None
    assert row.id == 1
