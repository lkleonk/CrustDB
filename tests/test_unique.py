import pytest

from crustdb import CrustDB, Int, Model, String, UniqueConstraintError


class User(Model):
    id = Int(id=True)
    username = String(unique=True)
    age = Int(range=(0, 120))


def test_duplicate_primary_key_raises(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)

    with pytest.raises(UniqueConstraintError, match="id"):
        db.User.insert(id=1, username="bob", age=30)


def test_duplicate_unique_field_raises(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)

    with pytest.raises(UniqueConstraintError, match="username"):
        db.User.insert(id=2, username="alice", age=30)
