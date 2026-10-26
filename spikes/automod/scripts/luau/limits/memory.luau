-- Builds a 64 MB string.
local s = "x"
for i = 1, 26 do s = s .. s end
return reject(tostring(#s))
