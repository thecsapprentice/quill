# Starlark has no `while`, and range() takes 32-bit numbers, so nested
# huge ranges are the nearest thing to an endless loop.
def check(post):
    n = 0
    for i in range(2000000000):
        for j in range(2000000000):
            n += 1
