let banned = [
    "spam",
    "free crypto",

let found = text::find_words(banned, post.title + " " + post.body);
if found.len() > 0 {
    return reject("This prompt uses words that aren't allowed here.");
}
