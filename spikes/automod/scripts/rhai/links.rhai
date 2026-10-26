// Links are only allowed to these sites.
let allowed_sites = ["quill.example", "archiveofourown.org"];

for link in text::links(post.body) {
    if !(link.host in allowed_sites) {
        return reject("Links to outside sites aren't allowed in prompts.");
    }
}

// Phone numbers: a digit, six or more digits, spaces, dots, dashes or
// brackets, then a digit. Backticks keep the backslashes as written.
if text::matches(`\+?\(?\d[\d\s().-]{6,}\d`, post.body) {
    return reject("Please don't share phone numbers in prompts.");
}
