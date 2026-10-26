// Builds a 64 MB string.
let s = "x";
for i in 0..26 { s += s; }
reject(s.len().to_string())
