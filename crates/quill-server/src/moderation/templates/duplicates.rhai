// Posting the same prompt again
//
// Refuses a prompt that's almost the same as one the same account posted
// in the last 30 days, across all its personas. Editing a prompt is never
// counted as posting it again.

// How alike (0 to 1) two prompts must be to count as the same.
let same = 0.85;

if post.hook != "prompt_created" {
    return;
}
for mine in post.history.own_recent {
    if mine.similarity >= same {
        return reject("This is very like your prompt \"" + mine.title + "\". Edit that one instead of posting it again.");
    }
}
