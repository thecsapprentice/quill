// Refuse prompts that use any word or phrase on this list. Matching sees
// through spacing ("s p a m"), stretched letters ("spaaam"), accents and
// lookalike letters, so the list only needs the plain spelling.
const banned = [
  "spam",
  "free crypto",
  "paypal",
];

const found = text.find_words(banned, post.title + " " + post.body);
if (found.length > 0) {
  return reject("This prompt uses words that aren't allowed here.");
}
