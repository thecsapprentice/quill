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
    /// That server is already in this app's list.
    SERVER_ALREADY_ADDED = 1055,
    /// A server's name in this app must be 1 to 60 characters.
    SERVER_NAME_BAD = 1056,

    // ---- Q-11xx: sign-in and logins ------------------------------------
    /// This computer has no login token for the server: sign in.
    NOT_SIGNED_IN = 1150,

    // ---- Q-13xx: this computer's store and keychain ----------------------
    /// The store file couldn't be read or written.
    STORE_IO = 1350,
    /// A store already exists where a new one was to be made.
    STORE_EXISTS = 1351,
    /// The store is open in another copy of Quill.
    STORE_IN_USE = 1352,
    /// That file isn't a Quill store.
    NOT_A_STORE = 1353,
    /// The store was made by a newer Quill: update this one.
    STORE_TOO_NEW = 1354,
    /// The store is damaged.
    STORE_DAMAGED = 1355,
    /// Wrong passphrase, or the keychain holds a different key.
    WRONG_PASSPHRASE = 1356,
    /// The store is unlocked another way (passphrase or keychain).
    WRONG_UNLOCK = 1357,
    /// The store's key isn't in this computer's keychain.
    KEY_NOT_IN_KEYCHAIN = 1358,
    /// The keychain refused or failed.
    KEYCHAIN_FAILED = 1359,
    /// The store's database failed.
    STORE_DATABASE = 1360,
    /// This computer has no keychain for Quill's key: choose a passphrase.
    NO_KEYCHAIN = 1361,
    /// The stores couldn't be moved to that folder.
    STORES_NOT_MOVED = 1362,
    /// A store's passphrase must be at least 12 characters.
    PASSPHRASE_TOO_SHORT = 1363,
    /// The store is busy with something else; try again in a moment.
    STORE_BUSY = 1364,

    // ---- Q-14xx: personas and handles --------------------------------------
    /// That persona is gone: deleted, perhaps from another device.
    PERSONA_GONE = 1450,

    // ---- Q-15xx: identity keys, pins and KeyPackages --------------------------
    /// A key check failed: something published doesn't verify.
    KEYS_CHECK_FAILED = 1550,
    /// Another writer's identity key changed since this computer pinned it.
    KEY_CHANGED = 1551,
    /// This persona has no keys on this computer.
    NO_KEYS_HERE = 1552,
    /// That writer has no identity key yet, so can't take part in sessions.
    NO_IDENTITY_KEY = 1553,
    /// A device certificate doesn't verify.
    BAD_CERTIFICATE = 1554,
    /// The encryption library (MLS) refused or failed.
    MLS_FAILED = 1555,
    /// A KeyPackage failed its checks.
    KEY_PACKAGE_REFUSED = 1556,
    /// The persona's identity key was made on another device: link this one.
    KEYS_ELSEWHERE = 1557,

    // ---- Q-16xx: devices and linking -----------------------------------------
    /// A check on the login's devices failed.
    DEVICES_CHECK_FAILED = 1650,
    /// The link expired before it was approved.
    LINK_EXPIRED = 1651,
    /// No device approved the link in time.
    LINK_NOT_ANSWERED = 1652,
    /// The new device's key doesn't match the code or words: don't link it.
    LINK_MISMATCH = 1653,
    /// This computer was removed from the login's devices.
    REMOVED_HERE = 1654,
    /// This computer was away too long to catch up with the login's devices.
    FELL_BEHIND = 1657,
    /// That isn't a link code or six words from the new computer.
    LINK_CODE_BAD = 1655,
    /// A device's name must be 1 to 40 characters on one line.
    DEVICE_NAME_BAD = 1656,

    // ---- Q-30xx: requests ---------------------------------------------------
    /// A request failed a check on this computer.
    REQUEST_CHECK_FAILED = 3050,
    /// The pitch is empty.
    PITCH_EMPTY = 3051,
    /// The pitch is longer than requests allow.
    PITCH_TOO_LONG = 3052,
    /// The prompt's author has no devices that can take a request now.
    AUTHOR_HAS_NO_DEVICES = 3053,
    /// A request must be read on this computer before it's accepted here.
    READ_IT_HERE_FIRST = 3054,
    /// The request was sent before this computer was linked.
    REQUEST_NOT_SENT_HERE = 3055,
    /// This computer sent the request, under another login.
    REQUEST_SENT_FROM_HERE = 3056,
    /// The request failed its checks (a forged group or pitch): never shown.
    REQUEST_REFUSED = 3057,

    // ---- Q-31xx, Q-33xx to Q-35xx: sessions, attachments, backup, history ------
    /// A session failed a check on this computer.
    SESSION_CHECK_FAILED = 3150,
    /// An attachment failed a check on this computer.
    ATTACHMENT_CHECK_FAILED = 3350,
    /// That attachment is on another server (federation comes later).
    ATTACHMENT_ELSEWHERE = 3351,
    /// A backup failed a check on this computer.
    BACKUP_CHECK_FAILED = 3450,
    /// A backup couldn't be decrypted or is damaged.
    BACKUP_DAMAGED = 3451,
    /// History failed a check on this computer.
    HISTORY_CHECK_FAILED = 3550,
    /// History sync between devices failed a check.
    HISTORY_SYNC_FAILED = 3551,

    // ---- Q-90xx: input checks shared by every area -------------------------
    /// The command was given something it can't use; its message says what.
    CLI_USAGE = 9050,

    // ---- Q-99xx: not yet given a code of its own -------------------------
    /// Something went wrong that has no code of its own yet. Advanced mode
    /// shows the detail; report it with that.
    UNCLASSIFIED = 9950,
}
