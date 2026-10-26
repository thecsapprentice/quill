// A pattern that hangs backtracking engines; Rust's regex stays linear.
let s = "";
for i in 0..5000 { s += "a"; }
s += "!";
if text::matches(`^(a+)+$`, s) { return reject("matched"); }
