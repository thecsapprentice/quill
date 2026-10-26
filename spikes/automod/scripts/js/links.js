// Links are only allowed to these sites.
const allowedSites = ["quill.example", "archiveofourown.org"];

for (const link of text.links(post.body)) {
  if (!allowedSites.includes(link.host)) {
    return reject("Links to outside sites aren't allowed in prompts.");
  }
}

// Phone numbers: a digit, six or more digits, spaces, dots, dashes or
// brackets, then a digit. JavaScript has regular expressions of its own
// (/.../.test), but text.matches uses the server's linear-time engine.
if (text.matches(String.raw`\+?\(?\d[\d\s().-]{6,}\d`, post.body)) {
  return reject("Please don't share phone numbers in prompts.");
}
