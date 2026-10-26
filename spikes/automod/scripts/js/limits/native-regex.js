// JavaScript's own regular expressions backtrack: this pattern takes
// exponential time. Does the time limit still reach inside the regex?
const s = "a".repeat(32) + "!";
if (/^(a+)+$/.test(s)) return reject("matched");
return reject("finished");
