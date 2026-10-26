# There is no open(), and load() has no loader to read files with.
load("Cargo.toml", "anything")

def check(post):
    return reject("loaded a file")
