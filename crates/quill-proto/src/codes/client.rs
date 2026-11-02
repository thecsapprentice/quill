//! Codes a client decides itself (50–99 of each area): the server didn't
//! answer, or this computer refused or couldn't do something.

registry! {
    // ---- Q-10xx: connection and the server -----------------------------
    /// The server couldn't be reached: offline, or the address is wrong.
    UNREACHABLE = 1050,
    /// The server's reply couldn't be read.
    MALFORMED_REPLY = 1051,
    /// The server's reply was larger than a client accepts.
    REPLY_TOO_LARGE = 1052,
    /// The server answered with a status this client doesn't expect.
    UNEXPECTED_REPLY = 1053,
    /// That isn't a server address, or it isn't https.
    BAD_ADDRESS = 1054,

    // ---- Q-99xx: not yet given a code of its own -------------------------
    /// Something went wrong that has no code of its own yet. Advanced mode
    /// shows the detail; report it with that.
    UNCLASSIFIED = 9950,
}
