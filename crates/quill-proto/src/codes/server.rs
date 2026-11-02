//! Codes the server sets on its refusals (01–49 of each area), and the
//! input checks shared with clients (Q-90xx).

registry! {
    // ---- Q-10xx: connection and the server -----------------------------
    /// What was asked for isn't there, or isn't shown to this login.
    NOT_FOUND = 1001,
    /// The request was larger than the server takes.
    TOO_LARGE = 1002,
    /// The request wasn't JSON, or its body wasn't what the call takes.
    BAD_BODY = 1003,
    /// Too many tries in a short time; wait and try again.
    TOO_MANY = 1004,
    /// Something went wrong on the server. Its log says what.
    SERVER_FAULT = 1005,
    /// The request's media type isn't one the call takes.
    WRONG_MEDIA_TYPE = 1006,

    // ---- Q-11xx: sign-in and logins ------------------------------------
    /// No login token, or one the server no longer accepts: sign in again.
    SIGNED_OUT = 1101,
    /// That login name and password don't match.
    WRONG_LOGIN = 1102,

    // ---- Q-90xx: input checks shared by every area ---------------------
    /// A login name must be 3 to 30 characters of a to z, 0 to 9 and _.
    BAD_LOGIN_NAME = 9001,
    /// A password is too short or too long.
    BAD_PASSWORD = 9002,
    /// Text is too long, or holds control characters.
    TEXT_TOO_LONG = 9003,
    /// A required field is empty or too long.
    REQUIRED_MISSING = 9004,
    /// A tag needs Latin letters or digits.
    BAD_TAG_NAME = 9005,
    /// A tag must be written as type:name.
    BAD_TAG_FORM = 9006,
    /// A tag type's name must be letters, digits and hyphens.
    BAD_TAG_TYPE_NAME = 9007,
    /// A prompt has more tags than it can carry.
    TOO_MANY_TAGS = 9008,
    /// A colour must be written as #rrggbb.
    BAD_COLOUR = 9009,
    /// A reserved handle entry isn't a valid handle.
    BAD_RESERVED_HANDLE = 9010,
    /// A channel name must be a short slug.
    BAD_CHANNEL = 9011,
    /// A staff name is empty, too long, or reserved.
    BAD_STAFF_NAME = 9012,
    /// A tag type's minimum and maximum don't fit together.
    BAD_TAG_LIMITS = 9013,
}
