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
    /// A form on the web board was sent without one of its choices picked.
    FORM_NO_CHOICE = 1007,
    /// A delete on the web board was sent without ticking the box that
    /// confirms it.
    FORM_NOT_CONFIRMED = 1008,
    /// The marker for the next page of a list isn't one the server gave out.
    BAD_CURSOR = 1009,

    // ---- Q-11xx: sign-in and logins ------------------------------------
    /// No login token, or one the server no longer accepts: sign in again.
    SIGNED_OUT = 1101,
    /// That login name and password don't match.
    WRONG_LOGIN = 1102,
    /// This server isn't taking new accounts right now.
    REGISTRATION_CLOSED = 1103,
    /// That login name is already taken.
    LOGIN_NAME_TAKEN = 1104,

    // ---- Q-12xx: server rules ------------------------------------------
    /// The server rules have changed; read and accept them to keep writing.
    RULES_NOT_ACCEPTED = 1201,
    /// The rules changed after they were read; read them again to accept.
    RULES_CHANGED = 1202,
    /// The rules changed while the account was being made; read them again.
    RULES_CHANGED_AT_SIGNUP = 1203,
    /// This server asks new accounts to accept its rules first.
    RULES_REQUIRED_AT_SIGNUP = 1204,

    // ---- Q-14xx: personas and handles ----------------------------------
    /// That persona isn't one of this login's.
    PERSONA_UNKNOWN = 1401,
    /// This login has as many personas as the server allows.
    PERSONA_LIMIT = 1402,
    /// That handle is taken or otherwise not available.
    HANDLE_TAKEN = 1403,
    /// That handle is kept for the server's staff.
    HANDLE_STAFF_RESERVED = 1404,
    /// No handles were entered to reserve.
    RESERVED_NONE_GIVEN = 1405,
    /// That reserved handle comes from the server's reserved-handles file,
    /// so it can only be removed there.
    RESERVED_FROM_FILE = 1406,
    /// Only administrators have a staff name or speak as staff.
    STAFF_ADMIN_ONLY = 1407,
    /// Another administrator already uses that staff name.
    STAFF_NAME_TAKEN = 1408,
    /// Set a staff name in your settings before speaking as staff.
    STAFF_NAME_UNSET = 1409,

    // ---- Q-15xx: identity keys, pins and KeyPackages -------------------
    /// A key, certificate or other key data wasn't written as base64url.
    KEYS_NOT_BASE64 = 1501,
    /// An identity key, device certificate or revocation didn't check out.
    KEYS_REFUSED = 1502,
    /// An identity key's version must be 1 or more, and not too large.
    KEYS_VERSION_OUT_OF_RANGE = 1503,
    /// A replacement identity key needs a higher version than the current one.
    KEYS_VERSION_NOT_HIGHER = 1504,
    /// That identity key is already used by another persona.
    KEYS_IDENTITY_IN_USE = 1505,
    /// The persona has no identity key yet; it needs one first.
    KEYS_NO_IDENTITY = 1506,
    /// That persona has no keys, so it can't be invited to sessions.
    KEYS_PERSONA_HAS_NONE = 1507,
    /// A KeyPackage couldn't be read, or isn't one the server takes.
    KEYPACKAGE_REFUSED = 1508,
    /// A device already has as many KeyPackages waiting as it may.
    KEYPACKAGE_POOL_FULL = 1509,

    // ---- Q-16xx: devices and linking -----------------------------------
    /// That device key is already registered.
    DEVICE_KEY_REGISTERED = 1601,
    /// That device key was revoked, and revocation is permanent: make a new
    /// device key.
    DEVICE_REVOKED_FOR_GOOD = 1602,
    /// That device is already certified.
    DEVICE_ALREADY_CERTIFIED = 1603,
    /// The revocation is for a different device.
    DEVICE_REVOCATION_MISMATCH = 1604,
    /// That device is already revoked.
    DEVICE_ALREADY_REVOKED = 1605,
    /// That device is revoked, so it can't be used.
    DEVICE_REVOKED = 1606,
    /// That device needs certifying with the persona's current identity key.
    DEVICE_NEEDS_CERTIFYING = 1607,
    /// Only a personal device group can be started this way.
    DEVICE_GROUP_NOT_PERSONAL = 1608,
    /// A device group's id must be 16 to 64 bytes.
    DEVICE_GROUP_BAD_ID = 1609,
    /// This login already has a personal device group.
    DEVICE_GROUP_EXISTS = 1610,

    // ---- Q-20xx: prompts -----------------------------------------------
    /// An administrator removed this prompt, so it can't be changed.
    PROMPT_REMOVED = 2001,

    // ---- Q-21xx: tags and tag types ------------------------------------
    /// There's no tag type by that name.
    TAG_TYPE_UNKNOWN = 2101,
    /// A tag type by that name already exists.
    TAG_TYPE_EXISTS = 2102,
    /// A curated-only tag type asks for more tags than it has to pick from.
    TAG_TYPE_MIN_UNREACHABLE = 2103,
    /// Together the tag types ask for more tags than a prompt can have.
    TAG_TYPES_MIN_TOO_HIGH = 2104,
    /// There's no tag by that name.
    TAG_UNKNOWN = 2105,
    /// That tag isn't on this server, and writers can't add it: its type is
    /// curated, or the server doesn't take new tags.
    TAG_NEW_NOT_ALLOWED = 2106,
    /// That tag is already a curated tag.
    TAG_ALREADY_CURATED = 2107,
    /// Only new tags from writers can be awaiting approval.
    TAG_NOT_AWAITING = 2108,
    /// A tag can't be an alias of itself.
    TAG_SELF_ALIAS = 2109,
    /// A tag search is too long.
    TAG_SEARCH_TOO_LONG = 2110,
    /// A prompt has too few or too many tags of some type.
    TAGS_WRONG_COUNT = 2111,

    // ---- Q-22xx: reports, notices and automated moderation -------------
    /// A moderation script refused what was written; the message says why.
    CONTENT_REJECTED = 2201,
    /// Reports can be listed as open or all, nothing else.
    REPORT_BAD_STATE = 2202,
    /// That report has already been resolved.
    REPORT_RESOLVED = 2203,
    /// The reported comment was deleted by its author; discard the report.
    REPORT_COMMENT_GONE = 2204,
    /// A common data entry's name must be lowercase letters, digits and
    /// underscores, up to 64 characters.
    AUTOMOD_DATA_BAD_NAME = 2205,
    /// A common data entry is larger than the server allows.
    AUTOMOD_DATA_TOO_LARGE = 2206,
    /// A common data entry's description is longer than 500 characters.
    AUTOMOD_DATA_DESCRIPTION_TOO_LONG = 2207,
    /// The common data entry was saved by someone else meanwhile; reload it.
    AUTOMOD_DATA_STALE = 2208,
    /// All common data together would be larger than the server allows.
    AUTOMOD_DATA_FULL = 2209,
    /// That common data entry is used by a moderation script.
    AUTOMOD_DATA_IN_USE = 2210,
    /// A moderation script's name must be 1 to 100 characters.
    AUTOMOD_SCRIPT_BAD_NAME = 2211,
    /// A moderation script is larger than the server allows.
    AUTOMOD_SCRIPT_TOO_LARGE = 2212,
    /// A moderation script doesn't compile; the message says where.
    AUTOMOD_SCRIPT_INVALID = 2213,
    /// A moderation script reads common data that doesn't exist yet.
    AUTOMOD_SCRIPT_MISSING_DATA = 2214,
    /// A moderation script failed on a sample post.
    AUTOMOD_SCRIPT_FAILS_SAMPLE = 2215,
    /// Common data a moderation script reads was deleted while it was saved.
    AUTOMOD_SCRIPT_DATA_DELETED = 2216,
    /// A moderation script by that name already exists.
    AUTOMOD_SCRIPT_NAME_TAKEN = 2217,
    /// A moderation chain has more scripts than it can run.
    AUTOMOD_CHAIN_TOO_LONG = 2218,
    /// A moderation chain names the same script twice.
    AUTOMOD_CHAIN_REPEATS = 2219,
    /// A moderation chain names a script that doesn't exist.
    AUTOMOD_CHAIN_UNKNOWN_SCRIPT = 2220,

    // ---- Q-23xx: announcements and comments ----------------------------
    /// Comments are closed on this announcement.
    COMMENTS_CLOSED = 2301,
    /// A comment speaks as a persona or as staff, not both.
    COMMENT_TWO_SPEAKERS = 2302,
    /// Comments from the administrators can't be removed, only deleted.
    COMMENT_STAFF_NOT_REMOVABLE = 2303,
    /// That comment has already been removed.
    COMMENT_ALREADY_REMOVED = 2304,

    // ---- Q-24xx: administration and server settings --------------------
    /// Only the server's administrators can do that.
    ADMIN_ONLY = 2401,
    /// The server's limits must be whole numbers.
    SETTINGS_LIMITS_NOT_NUMBERS = 2402,

    // ---- Q-30xx: requests ----------------------------------------------
    /// Some part of a request wasn't written as base64url.
    REQUEST_NOT_BASE64 = 3001,
    /// A request's opening message (its pitch) couldn't be read.
    REQUEST_BAD_PITCH = 3002,
    /// A request's expiry is outside the days the server allows.
    REQUEST_BAD_EXPIRY = 3003,
    /// That's your own prompt: you can't ask to join it.
    REQUEST_OWN_PROMPT = 3004,
    /// You already have a request waiting on this prompt.
    REQUEST_ALREADY_WAITING = 3005,
    /// The request was accepted and is a session now.
    REQUEST_ALREADY_SESSION = 3006,
    /// The request was declined.
    REQUEST_DECLINED = 3007,
    /// The request expired.
    REQUEST_EXPIRED = 3008,
    /// Requests can be listed as pending or held, nothing else.
    REQUEST_BAD_STATE = 3009,

    // ---- Q-31xx: sessions and groups -----------------------------------
    /// A session id or other group data wasn't written as base64url.
    GROUP_NOT_BASE64 = 3101,
    /// What was sent as a welcome to a group isn't one.
    NOT_A_WELCOME = 3102,
    /// That group id is already taken.
    GROUP_ID_TAKEN = 3103,
    /// The session has ended.
    SESSION_ENDED = 3104,
    /// Welcomes can only be left for your own persona's devices in the
    /// session, or for a device asking to rejoin.
    SESSION_WELCOME_NOT_ALLOWED = 3105,
    /// A device's request to rejoin a session didn't check out.
    SESSION_REJOIN_REFUSED = 3106,

    // ---- Q-32xx: messages and channels ---------------------------------
    /// A message couldn't be read as one the server takes.
    MESSAGE_UNREADABLE = 3201,
    /// A message was sent to a different group than it was written for.
    MESSAGE_WRONG_GROUP = 3202,
    /// The group moved on while the message was written; catch up and send
    /// again.
    MESSAGE_STALE_EPOCH = 3203,
    /// A message has more attachments than the server allows.
    MESSAGE_TOO_MANY_ATTACHMENTS = 3204,

    // ---- Q-33xx: attachments and storage -------------------------------
    /// This server doesn't take attachments.
    ATTACHMENTS_OFF = 3301,
    /// Your attachment storage is full; it frees up as messages are read.
    ATTACHMENT_STORAGE_FULL = 3302,
    /// An attachment wasn't padded to one of the allowed sizes.
    ATTACHMENT_NOT_PADDED = 3303,
    /// A message names attachments that aren't the sender's to attach there.
    ATTACHMENT_NOT_YOURS = 3304,

    // ---- Q-34xx: backup and recovery -----------------------------------
    /// A backup is larger than this server keeps.
    BACKUP_TOO_LARGE = 3401,
    /// A part of a backup was empty.
    BACKUP_EMPTY_PART = 3402,
    /// Some parts of the backup never arrived.
    BACKUP_INCOMPLETE = 3403,

    // ---- Q-35xx: history and sync --------------------------------------
    /// A device marked a place as read that isn't in the session's log.
    SYNC_ACK_OUT_OF_RANGE = 3501,
    /// A device can only mark what it has read for its own persona's
    /// devices.
    SYNC_ACK_NOT_YOURS = 3502,
    /// The group's older messages were deleted, so this device can't follow
    /// it from where it is.
    SYNC_PRUNED = 3503,

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
