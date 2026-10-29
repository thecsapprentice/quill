// New accounts
//
// Extra care for accounts less than a few days old: a limit on how fast
// they post, and a flag when they post a near copy of someone else's
// prompt.

// How old, in days, an account must be to skip these checks.
let new_for_days = 3;
// Prompts a new account may post in an hour.
let per_hour = 3;
// How alike (0 to 1) a prompt must be to another account's to be flagged.
let copy_similarity = 0.9;

if post.author.account_age_days < new_for_days {
    if post.kind == "prompt" && post.hook == "prompt_created" && post.history.prompts_last_hour >= per_hour {
        return reject("New accounts can post up to " + per_hour + " prompts an hour. Please try again a little later.");
    }
    for other in post.history.similar_on_board {
        if !other.same_account && other.similarity >= copy_similarity {
            return flag("A new account posted a near copy of \"" + other.title + "\"", "spam");
        }
    }
}
