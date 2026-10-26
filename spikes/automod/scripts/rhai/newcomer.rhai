// Extra care for accounts less than three days old.
if post.author.account_age_days < 3 {
    if post.history.prompts_last_hour >= 3 {
        return reject("New accounts can post up to 3 prompts an hour. Please try again a little later.");
    }
    if post.history.most_similar_on_board > 0.9 {
        return flag("New account posted a near copy of another prompt", "spam");
    }
}
