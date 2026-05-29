from crustdb import CrustDB, Int, Model, String


class User(Model):
    id = Int(id=True)
    username = String(unique=True)
    age = Int(range=(0, 120))


db = CrustDB("app.crustdb")
db.register(User)

user = db.User.find(id=1)
if user is None:
    user = db.User.insert(id=1, username="alice", age=25)

print(user.username)
