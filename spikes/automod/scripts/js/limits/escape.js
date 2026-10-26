// QuickJS's std and os modules aren't loaded, and there's no module loader.
const fs = require("fs");
return reject(fs.readFileSync("Cargo.toml", "utf8"));
