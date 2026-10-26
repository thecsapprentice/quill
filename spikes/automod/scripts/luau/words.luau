-- Refuse prompts that use any word or phrase on this list. Matching sees
-- through spacing ("s p a m"), stretched letters ("spaaam"), accents and
-- lookalike letters, so the list only needs the plain spelling.
local banned = {
    "spam",
    "free crypto",
    "paypal",
}

local found = text.find_words(banned, post.title .. " " .. post.body)
if #found > 0 then
    return reject("This prompt uses words that aren't allowed here.")
end
