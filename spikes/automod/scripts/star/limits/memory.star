# Builds a 64 MB string.
def check(post):
    s = "x"
    for i in range(26):
        s = s + s
    return reject(str(len(s)))
