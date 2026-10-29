// Mixed alphabets
//
// Words that mix writing systems, like "pаypal" with a Cyrillic "а", are a
// common way to slip past word lists. This lets the post through but asks
// an administrator to look. Posts written in another alphabet are fine;
// it's mixing inside one word that's flagged.

let odd = text::mixed_script_words(post.title + "\n" + post.body);
if odd.len() > 0 {
    return flag("Mixed alphabets in \"" + odd[0] + "\"", "spam");
}
