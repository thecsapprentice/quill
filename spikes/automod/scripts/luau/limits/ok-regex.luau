-- A pattern that hangs backtracking engines; Rust's regex stays linear.
local s = string.rep("a", 5000) .. "!"
if text.matches([[^(a+)+$]], s) then return reject("matched") end
