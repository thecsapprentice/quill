-- Luau has no io library, and os is cut down to time functions.
local f = io.open("Cargo.toml")
return reject(f:read("*a"))
