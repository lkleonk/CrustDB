import pytest

from crustdb import CrustDB, Int, Model, String, ValidationError


class User(Model):
    id = Int(id=True)
    username = String(unique=True)
    age = Int(range=(0, 120))


def test_missing_required_field_raises(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)

    with pytest.raises(ValidationError, match="Missing required field: username"):
        db.User.insert(id=1, age=25)


def test_wrong_type_raises(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)

    with pytest.raises(ValidationError, match="age must be Int"):
        db.User.insert(id=1, username="alice", age="25")


def test_bool_is_not_accepted_as_int(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)

    with pytest.raises(ValidationError, match="id must be Int"):
        db.User.insert(id=True, username="alice", age=25)


def test_none_is_not_accepted_for_required_field(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)

    with pytest.raises(ValidationError, match="Missing required field: username"):
        db.User.insert(id=1, username=None, age=25)


def test_out_of_range_integer_raises(tmp_path):
    db = CrustDB(tmp_path / "app.crustdb")
    db.register(User)

    with pytest.raises(ValidationError, match="age must be between 0 and 120"):
        db.User.insert(id=1, username="alice", age=121)


def test_default_value_is_applied(tmp_path):
    class Post(Model):
        id = Int(id=True)
        title = String()
        views = Int(default=0)

    db = CrustDB(tmp_path / "app.crustdb")
    db.register(Post)

    row = db.Post.insert(id=1, title="Hello")

    assert row.views == 0
