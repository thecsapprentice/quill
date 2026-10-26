// Words that mix alphabets, like "pаypal" with a Cyrillic "а", are a common
// way to slip past word lists. Let it through, but ask an admin to look.
let odd = text::mixed_script_words(post.title + " " + post.body);
if odd.len() > 0 {
    return flag("Mixed alphabets in \"" + odd[0] + "\"", "spam");
}
