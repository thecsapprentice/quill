def check(post):
    found = text.find_words(["spam"], post.titel + " " + post.body)
    if found:
        return reject("This prompt uses words that aren't allowed here.")
