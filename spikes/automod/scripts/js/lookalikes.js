// Words that mix alphabets, like "pаypal" with a Cyrillic "а", are a common
// way to slip past word lists. Let it through, but ask an admin to look.
const odd = text.mixed_script_words(post.title + " " + post.body);
if (odd.length > 0) {
  return flag(`Mixed alphabets in "${odd[0]}"`, "spam");
}
