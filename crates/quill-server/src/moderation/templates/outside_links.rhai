// Links to outside sites
//
// Refuses posts that link anywhere except the sites in the "allowed_sites"
// common data entry: one site per line, like "archiveofourown.org". A site
// on the list allows its subdomains too ("www.", "en.").

let message = "Links to outside sites aren't allowed here.";

// Change to true to let the post through and flag it for an administrator
// instead of refusing it.
let flag_instead = false;

let allowed = common::lines("allowed_sites");
for link in text::links(post.body) {
    let ok = false;
    for site in allowed {
        if link.host == site || link.host.ends_with("." + site) {
            ok = true;
        }
    }
    if !ok {
        if flag_instead {
            return flag("Links to " + link.host, "spam");
        }
        return reject(message);
    }
}
