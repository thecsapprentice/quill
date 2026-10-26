const banned = [
  "spam",
  "free crypto",

const found = text.find_words(banned, post.title + " " + post.body);
if (found.length > 0) {
  return reject("This prompt uses words that aren't allowed here.");
}
