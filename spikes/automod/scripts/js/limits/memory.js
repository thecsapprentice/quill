// Builds a 64 MB string. Doubling with `s + s` isn't enough here: QuickJS
// joins strings lazily (ropes), so nothing large is ever allocated.
const s = "x".repeat(1 << 26);
return reject(String(s.length));
