const found = text.find_words(["spam"], post.titel + " " + post.body);
if (found.length > 0) {
  return reject("This prompt uses words that aren't allowed here.");
}
