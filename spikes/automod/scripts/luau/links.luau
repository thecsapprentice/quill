-- Links are only allowed to these sites.
local allowed_sites = { "quill.example", "archiveofourown.org" }

for _, link in text.links(post.body) do
    if not table.find(allowed_sites, link.host) then
        return reject("Links to outside sites aren't allowed in prompts.")
    end
end

-- Phone numbers: a digit, six or more digits, spaces, dots, dashes or
-- brackets, then a digit. [[ ]] keeps the backslashes as written.
if text.matches([[\+?\(?\d[\d\s().-]{6,}\d]], post.body) then
    return reject("Please don't share phone numbers in prompts.")
end
