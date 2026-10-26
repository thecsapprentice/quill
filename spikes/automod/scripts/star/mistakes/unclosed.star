BANNED = [
    "spam",
    "free crypto",

def check(post):
    found = text.find_words(BANNED, post.title + " " + post.body)
    if found:
        return reject("This prompt uses words that aren't allowed here.")
