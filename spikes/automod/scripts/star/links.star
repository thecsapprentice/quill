# Links are only allowed to these sites.
ALLOWED_SITES = ["quill.example", "archiveofourown.org"]

# Phone numbers: a digit, six or more digits, spaces, dots, dashes or
# brackets, then a digit. r"..." keeps the backslashes as written.
PHONE = r"\+?\(?\d[\d\s().-]{6,}\d"

def check(post):
    for link in text.links(post.body):
        if link.host not in ALLOWED_SITES:
            return reject("Links to outside sites aren't allowed in prompts.")

    if text.matches(PHONE, post.body):
        return reject("Please don't share phone numbers in prompts.")
