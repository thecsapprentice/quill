// Shouting and repeated characters
//
// Flags posts written mostly in capitals, or with long runs of the same
// character ("!!!!!!!", "sooooo"), for an administrator to look at. The
// title and the text are judged separately.

// Share of letters in capitals (0 to 1) that counts as shouting.
let caps = 0.7;
// Share of characters in runs of three or more that's too many.
let repeats = 0.3;
// Text shorter than this many letters isn't judged ("OK!" is fine).
let at_least = 20;

for part in [post.title, post.body] {
    let letters = text::replace(`[^\p{L}]`, part, "");
    if letters.len() < at_least {
        continue;
    }
    if text::caps_ratio(part) >= caps {
        return flag("Mostly capitals", "spam");
    }
    if text::repeat_ratio(part) >= repeats {
        return flag("Lots of repeated characters", "spam");
    }
}
