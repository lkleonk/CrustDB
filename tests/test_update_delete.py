import pytest

from crustdb import CrustDB, Int, Model, String, UniqueConstraintError


class User(Model):
    id = Int(id=True)
    username = String(unique=True)
    age = Int(range=(0, 120))


def test_delete_returns_true_and_removes_row(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)

    deleted = db.User.delete(id=1)

    assert deleted is True
    assert db.User.find(id=1) is None


def test_delete_missing_row_returns_false(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)

    assert db.User.delete(id=1) is False


def test_deleted_unique_value_can_be_reused(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)
    db.User.delete(id=1)

    row = db.User.insert(id=2, username="alice", age=30)

    assert row.id == 2
    assert row.username == "alice"


def test_update_returns_updated_row(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)

    row = db.User.update(where={"id": 1}, values={"age": 26})

    assert row is not None
    assert row.id == 1
    assert row.username == "alice"
    assert row.age == 26


def test_update_persists_after_reopen(tmp_path):
    db_path = tmp_path / "app.crustdb"

    db = CrustDB(db_path)
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)
    db.User.update(where={"id": 1}, values={"age": 26})

    reopened = CrustDB(db_path)
    reopened.register(User)
    row = reopened.User.find(id=1)

    assert row is not None
    assert row.age == 26


def test_delete_persists_after_reopen(tmp_path):
    db_path = tmp_path / "app.crustdb"

    db = CrustDB(db_path)
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)
    db.User.delete(id=1)

    reopened = CrustDB(db_path)
    reopened.register(User)

    assert reopened.User.find(id=1) is None


def test_update_duplicate_unique_raises(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)
    db.User.insert(id=2, username="bob", age=30)

    with pytest.raises(UniqueConstraintError, match="username"):
        db.User.update(where={"id": 2}, values={"username": "alice"})
