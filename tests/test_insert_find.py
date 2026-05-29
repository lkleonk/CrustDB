from crustdb import CrustDB, Int, Model, String


class User(Model):
    id = Int(id=True)
    username = String(unique=True)
    age = Int(range=(0, 120))


def test_register_exposes_table(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)

    assert hasattr(db, "User")


def test_insert_stores_and_returns_row(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)

    row = db.User.insert(id=1, username="alice", age=25)

    assert row.id == 1
    assert row.username == "alice"
    assert row.age == 25
    assert row["username"] == "alice"
    assert row.to_dict() == {"id": 1, "username": "alice", "age": 25}


def test_find_by_primary_key_returns_row(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)
    db.User.insert(id=1, username="alice", age=25)

    row = db.User.find(id=1)

    assert row is not None
    assert row.username == "alice"


def test_find_returns_none_when_missing(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)

    assert db.User.find(id=1) is None
