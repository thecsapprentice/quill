local found = text.find_words({ "spam" }, post.titel .. " " .. post.body)
if #found > 0 then
    return reject("This prompt uses words that aren't allowed here.")
end
