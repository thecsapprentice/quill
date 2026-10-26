local banned = {
    "spam",
    "free crypto",

local found = text.find_words(banned, post.title .. " " .. post.body)
if #found > 0 then
    return reject("This prompt uses words that aren't allowed here.")
end
