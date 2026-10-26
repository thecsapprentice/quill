-- Sandboxed globals are read-only, so one script can't rewire another's
-- `reject` or the text API.
text.find_words = function() return {} end
return reject("rewired text.find_words")
