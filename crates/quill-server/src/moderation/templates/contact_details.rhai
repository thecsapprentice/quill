// Phone numbers and email addresses
//
// Refuses posts that share a phone number or an email address, so writers
// meet through the server's own requests instead.

let message = "Please don't share phone numbers or email addresses here. Use a session request instead.";

let all = post.title + "\n" + post.body;

// A digit, then six or more digits, spaces, dots, dashes or brackets, then
// a digit: "+1 (555) 010-7788", "07700 900123".
if text::matches(`\+?\(?\d[\d\s().-]{6,}\d`, all) {
    return reject(message);
}

// Something@something.something, also written "name at site dot com".
if text::matches(`[\w.+-]+@[\w-]+\.[\w.-]+`, all)
    || text::matches(`\b\w+\s*[\[(]?\s*at\s*[\])]?\s*\w+\s*[\[(]?\s*dot\s*[\])]?\s*(com|net|org)\b`, all)
{
    return reject(message);
}
