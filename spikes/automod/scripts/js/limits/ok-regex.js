// The same pattern through text.matches, which stays linear.
const s = "a".repeat(5000) + "!";
if (text.matches(String.raw`^(a+)+$`, s)) return reject("matched");
