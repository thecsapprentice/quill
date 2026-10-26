# Refuse prompts that use any word or phrase on this list. Matching sees
# through spacing ("s p a m"), stretched letters ("spaaam"), accents and
# lookalike letters, so the list only needs the plain spelling.
BANNED = [
    "spam",
    "free crypto",
    "paypal",
]

def check(post):
    found = text.find_words(BANNED, post.title + " " + post.body)
    if found:
        return reject("This prompt uses words that aren't allowed here.")
