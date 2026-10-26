# A pattern that hangs backtracking engines; Rust's regex stays linear.
def check(post):
    s = "a" * 5000 + "!"
    if text.matches(r"^(a+)+$", s):
        return reject("matched")
