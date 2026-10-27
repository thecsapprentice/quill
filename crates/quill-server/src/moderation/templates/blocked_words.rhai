// Blocked words and phrases
//
// Refuses posts that use any word or phrase in the "blocked_words" common
// data entry: one per line, lines starting with # are notes. Matching sees
// through spacing ("s p a m"), stretched letters ("spaaam"), accents and
// lookalike letters, so the list only needs the plain spelling.

// What the writer is told. They can change their post and try again.
let message = "This uses words that aren't allowed here. Please change it and try again.";

let found = text::find_words(common::lines("blocked_words"), post.title + "\n" + post.body);
if found.len() > 0 {
    return reject(message);
}
