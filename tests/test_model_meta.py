from crustdb import Int, Model, String


class User(Model):
    id = Int(id=True)
    username = String(unique=True)


def test_model_meta_collects_fields():
    assert User.__model_name__ == "User"
    assert set(User.__fields__) == {"id", "username"}
    assert User.__fields__["id"].name == "id"
    assert User.__fields__["id"].id is True
    assert User.__fields__["username"].unique is True
