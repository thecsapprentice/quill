# Error codes

Generated from `quill_proto::codes`; don't edit by hand (`QUILL_WRITE_CODES=1 cargo test -p quill-proto codes`).

Every problem Quill can show has a code, `Q-AACC`: `AA` is the area and `CC` the case. In each area, 01–49 are refusals by the server and 50–99 are decided by the client. The desktop app shows codes in advanced mode; the CLI always prints them. A code never changes meaning, and a retired code is never reused.

## Q-10xx: Connection and the server

| Code | Name | Meaning |
| --- | --- | --- |
| Q-1001 | `NOT_FOUND` | What was asked for isn't there, or isn't shown to this login. |
| Q-1002 | `TOO_LARGE` | The request was larger than the server takes. |
| Q-1003 | `BAD_BODY` | The request wasn't JSON, or its body wasn't what the call takes. |
| Q-1004 | `TOO_MANY` | Too many tries in a short time; wait and try again. |
| Q-1005 | `SERVER_FAULT` | Something went wrong on the server. Its log says what. |
| Q-1006 | `WRONG_MEDIA_TYPE` | The request's media type isn't one the call takes. |
| Q-1007 | `FORM_NO_CHOICE` | A form on the web board was sent without one of its choices picked. |
| Q-1008 | `FORM_NOT_CONFIRMED` | A delete on the web board was sent without ticking the box that confirms it. |
| Q-1009 | `BAD_CURSOR` | The marker for the next page of a list isn't one the server gave out. |
| Q-1050 | `UNREACHABLE` | The server couldn't be reached: offline, or the address is wrong. |
| Q-1051 | `MALFORMED_REPLY` | The server's reply couldn't be read. |
| Q-1052 | `REPLY_TOO_LARGE` | The server's reply was larger than a client accepts. |
| Q-1053 | `UNEXPECTED_REPLY` | The server answered with a status this client doesn't expect. |
| Q-1054 | `BAD_ADDRESS` | That isn't a server address, or it isn't https. |

## Q-11xx: Sign-in and logins

| Code | Name | Meaning |
| --- | --- | --- |
| Q-1101 | `SIGNED_OUT` | No login token, or one the server no longer accepts: sign in again. |
| Q-1102 | `WRONG_LOGIN` | That login name and password don't match. |
| Q-1103 | `REGISTRATION_CLOSED` | This server isn't taking new accounts right now. |
| Q-1104 | `LOGIN_NAME_TAKEN` | That login name is already taken. |

## Q-12xx: Server rules

| Code | Name | Meaning |
| --- | --- | --- |
| Q-1201 | `RULES_NOT_ACCEPTED` | The server rules have changed; read and accept them to keep writing. |
| Q-1202 | `RULES_CHANGED` | The rules changed after they were read; read them again to accept. |
| Q-1203 | `RULES_CHANGED_AT_SIGNUP` | The rules changed while the account was being made; read them again. |
| Q-1204 | `RULES_REQUIRED_AT_SIGNUP` | This server asks new accounts to accept its rules first. |

## Q-14xx: Personas and handles

| Code | Name | Meaning |
| --- | --- | --- |
| Q-1401 | `PERSONA_UNKNOWN` | That persona isn't one of this login's. |
| Q-1402 | `PERSONA_LIMIT` | This login has as many personas as the server allows. |
| Q-1403 | `HANDLE_TAKEN` | That handle is taken or otherwise not available. |
| Q-1404 | `HANDLE_STAFF_RESERVED` | That handle is kept for the server's staff. |
| Q-1405 | `RESERVED_NONE_GIVEN` | No handles were entered to reserve. |
| Q-1406 | `RESERVED_FROM_FILE` | That reserved handle comes from the server's reserved-handles file, so it can only be removed there. |
| Q-1407 | `STAFF_ADMIN_ONLY` | Only administrators have a staff name or speak as staff. |
| Q-1408 | `STAFF_NAME_TAKEN` | Another administrator already uses that staff name. |
| Q-1409 | `STAFF_NAME_UNSET` | Set a staff name in your settings before speaking as staff. |

## Q-15xx: Identity keys, pins and KeyPackages

| Code | Name | Meaning |
| --- | --- | --- |
| Q-1501 | `KEYS_NOT_BASE64` | A key, certificate or other key data wasn't written as base64url. |
| Q-1502 | `KEYS_REFUSED` | An identity key, device certificate or revocation didn't check out. |
| Q-1503 | `KEYS_VERSION_OUT_OF_RANGE` | An identity key's version must be 1 or more, and not too large. |
| Q-1504 | `KEYS_VERSION_NOT_HIGHER` | A replacement identity key needs a higher version than the current one. |
| Q-1505 | `KEYS_IDENTITY_IN_USE` | That identity key is already used by another persona. |
| Q-1506 | `KEYS_NO_IDENTITY` | The persona has no identity key yet; it needs one first. |
| Q-1507 | `KEYS_PERSONA_HAS_NONE` | That persona has no keys, so it can't be invited to sessions. |
| Q-1508 | `KEYPACKAGE_REFUSED` | A KeyPackage couldn't be read, or isn't one the server takes. |
| Q-1509 | `KEYPACKAGE_POOL_FULL` | A device already has as many KeyPackages waiting as it may. |

## Q-16xx: Devices and linking

| Code | Name | Meaning |
| --- | --- | --- |
| Q-1601 | `DEVICE_KEY_REGISTERED` | That device key is already registered. |
| Q-1602 | `DEVICE_REVOKED_FOR_GOOD` | That device key was revoked, and revocation is permanent: make a new device key. |
| Q-1603 | `DEVICE_ALREADY_CERTIFIED` | That device is already certified. |
| Q-1604 | `DEVICE_REVOCATION_MISMATCH` | The revocation is for a different device. |
| Q-1605 | `DEVICE_ALREADY_REVOKED` | That device is already revoked. |
| Q-1606 | `DEVICE_REVOKED` | That device is revoked, so it can't be used. |
| Q-1607 | `DEVICE_NEEDS_CERTIFYING` | That device needs certifying with the persona's current identity key. |
| Q-1608 | `DEVICE_GROUP_NOT_PERSONAL` | Only a personal device group can be started this way. |
| Q-1609 | `DEVICE_GROUP_BAD_ID` | A device group's id must be 16 to 64 bytes. |
| Q-1610 | `DEVICE_GROUP_EXISTS` | This login already has a personal device group. |

## Q-20xx: Prompts

| Code | Name | Meaning |
| --- | --- | --- |
| Q-2001 | `PROMPT_REMOVED` | An administrator removed this prompt, so it can't be changed. |

## Q-21xx: Tags and tag types

| Code | Name | Meaning |
| --- | --- | --- |
| Q-2101 | `TAG_TYPE_UNKNOWN` | There's no tag type by that name. |
| Q-2102 | `TAG_TYPE_EXISTS` | A tag type by that name already exists. |
| Q-2103 | `TAG_TYPE_MIN_UNREACHABLE` | A curated-only tag type asks for more tags than it has to pick from. |
| Q-2104 | `TAG_TYPES_MIN_TOO_HIGH` | Together the tag types ask for more tags than a prompt can have. |
| Q-2105 | `TAG_UNKNOWN` | There's no tag by that name. |
| Q-2106 | `TAG_NEW_NOT_ALLOWED` | That tag isn't on this server, and writers can't add it: its type is curated, or the server doesn't take new tags. |
| Q-2107 | `TAG_ALREADY_CURATED` | That tag is already a curated tag. |
| Q-2108 | `TAG_NOT_AWAITING` | Only new tags from writers can be awaiting approval. |
| Q-2109 | `TAG_SELF_ALIAS` | A tag can't be an alias of itself. |
| Q-2110 | `TAG_SEARCH_TOO_LONG` | A tag search is too long. |
| Q-2111 | `TAGS_WRONG_COUNT` | A prompt has too few or too many tags of some type. |

## Q-22xx: Reports, notices and automated moderation

| Code | Name | Meaning |
| --- | --- | --- |
| Q-2201 | `CONTENT_REJECTED` | A moderation script refused what was written; the message says why. |
| Q-2202 | `REPORT_BAD_STATE` | Reports can be listed as open or all, nothing else. |
| Q-2203 | `REPORT_RESOLVED` | That report has already been resolved. |
| Q-2204 | `REPORT_COMMENT_GONE` | The reported comment was deleted by its author; discard the report. |
| Q-2205 | `AUTOMOD_DATA_BAD_NAME` | A common data entry's name must be lowercase letters, digits and underscores, up to 64 characters. |
| Q-2206 | `AUTOMOD_DATA_TOO_LARGE` | A common data entry is larger than the server allows. |
| Q-2207 | `AUTOMOD_DATA_DESCRIPTION_TOO_LONG` | A common data entry's description is longer than 500 characters. |
| Q-2208 | `AUTOMOD_DATA_STALE` | The common data entry was saved by someone else meanwhile; reload it. |
| Q-2209 | `AUTOMOD_DATA_FULL` | All common data together would be larger than the server allows. |
| Q-2210 | `AUTOMOD_DATA_IN_USE` | That common data entry is used by a moderation script. |
| Q-2211 | `AUTOMOD_SCRIPT_BAD_NAME` | A moderation script's name must be 1 to 100 characters. |
| Q-2212 | `AUTOMOD_SCRIPT_TOO_LARGE` | A moderation script is larger than the server allows. |
| Q-2213 | `AUTOMOD_SCRIPT_INVALID` | A moderation script doesn't compile; the message says where. |
| Q-2214 | `AUTOMOD_SCRIPT_MISSING_DATA` | A moderation script reads common data that doesn't exist yet. |
| Q-2215 | `AUTOMOD_SCRIPT_FAILS_SAMPLE` | A moderation script failed on a sample post. |
| Q-2216 | `AUTOMOD_SCRIPT_DATA_DELETED` | Common data a moderation script reads was deleted while it was saved. |
| Q-2217 | `AUTOMOD_SCRIPT_NAME_TAKEN` | A moderation script by that name already exists. |
| Q-2218 | `AUTOMOD_CHAIN_TOO_LONG` | A moderation chain has more scripts than it can run. |
| Q-2219 | `AUTOMOD_CHAIN_REPEATS` | A moderation chain names the same script twice. |
| Q-2220 | `AUTOMOD_CHAIN_UNKNOWN_SCRIPT` | A moderation chain names a script that doesn't exist. |

## Q-23xx: Announcements and comments

| Code | Name | Meaning |
| --- | --- | --- |
| Q-2301 | `COMMENTS_CLOSED` | Comments are closed on this announcement. |
| Q-2302 | `COMMENT_TWO_SPEAKERS` | A comment speaks as a persona or as staff, not both. |
| Q-2303 | `COMMENT_STAFF_NOT_REMOVABLE` | Comments from the administrators can't be removed, only deleted. |
| Q-2304 | `COMMENT_ALREADY_REMOVED` | That comment has already been removed. |

## Q-24xx: Administration and server settings

| Code | Name | Meaning |
| --- | --- | --- |
| Q-2401 | `ADMIN_ONLY` | Only the server's administrators can do that. |
| Q-2402 | `SETTINGS_LIMITS_NOT_NUMBERS` | The server's limits must be whole numbers. |

## Q-30xx: Requests

| Code | Name | Meaning |
| --- | --- | --- |
| Q-3001 | `REQUEST_NOT_BASE64` | Some part of a request wasn't written as base64url. |
| Q-3002 | `REQUEST_BAD_PITCH` | A request's opening message (its pitch) couldn't be read. |
| Q-3003 | `REQUEST_BAD_EXPIRY` | A request's expiry is outside the days the server allows. |
| Q-3004 | `REQUEST_OWN_PROMPT` | That's your own prompt: you can't ask to join it. |
| Q-3005 | `REQUEST_ALREADY_WAITING` | You already have a request waiting on this prompt. |
| Q-3006 | `REQUEST_ALREADY_SESSION` | The request was accepted and is a session now. |
| Q-3007 | `REQUEST_DECLINED` | The request was declined. |
| Q-3008 | `REQUEST_EXPIRED` | The request expired. |
| Q-3009 | `REQUEST_BAD_STATE` | Requests can be listed as pending or held, nothing else. |

## Q-31xx: Sessions and groups

| Code | Name | Meaning |
| --- | --- | --- |
| Q-3101 | `GROUP_NOT_BASE64` | A session id or other group data wasn't written as base64url. |
| Q-3102 | `NOT_A_WELCOME` | What was sent as a welcome to a group isn't one. |
| Q-3103 | `GROUP_ID_TAKEN` | That group id is already taken. |
| Q-3104 | `SESSION_ENDED` | The session has ended. |
| Q-3105 | `SESSION_WELCOME_NOT_ALLOWED` | Welcomes can only be left for your own persona's devices in the session, or for a device asking to rejoin. |
| Q-3106 | `SESSION_REJOIN_REFUSED` | A device's request to rejoin a session didn't check out. |

## Q-32xx: Messages and channels

| Code | Name | Meaning |
| --- | --- | --- |
| Q-3201 | `MESSAGE_UNREADABLE` | A message couldn't be read as one the server takes. |
| Q-3202 | `MESSAGE_WRONG_GROUP` | A message was sent to a different group than it was written for. |
| Q-3203 | `MESSAGE_STALE_EPOCH` | The group moved on while the message was written; catch up and send again. |
| Q-3204 | `MESSAGE_TOO_MANY_ATTACHMENTS` | A message has more attachments than the server allows. |

## Q-33xx: Attachments and storage

| Code | Name | Meaning |
| --- | --- | --- |
| Q-3301 | `ATTACHMENTS_OFF` | This server doesn't take attachments. |
| Q-3302 | `ATTACHMENT_STORAGE_FULL` | Your attachment storage is full; it frees up as messages are read. |
| Q-3303 | `ATTACHMENT_NOT_PADDED` | An attachment wasn't padded to one of the allowed sizes. |
| Q-3304 | `ATTACHMENT_NOT_YOURS` | A message names attachments that aren't the sender's to attach there. |

## Q-34xx: Backup and recovery

| Code | Name | Meaning |
| --- | --- | --- |
| Q-3401 | `BACKUP_TOO_LARGE` | A backup is larger than this server keeps. |
| Q-3402 | `BACKUP_EMPTY_PART` | A part of a backup was empty. |
| Q-3403 | `BACKUP_INCOMPLETE` | Some parts of the backup never arrived. |

## Q-35xx: History and sync

| Code | Name | Meaning |
| --- | --- | --- |
| Q-3501 | `SYNC_ACK_OUT_OF_RANGE` | A device marked a place as read that isn't in the session's log. |
| Q-3502 | `SYNC_ACK_NOT_YOURS` | A device can only mark what it has read for its own persona's devices. |
| Q-3503 | `SYNC_PRUNED` | The group's older messages were deleted, so this device can't follow it from where it is. |

## Q-90xx: Input checks shared by every area

| Code | Name | Meaning |
| --- | --- | --- |
| Q-9001 | `BAD_LOGIN_NAME` | A login name must be 3 to 30 characters of a to z, 0 to 9 and _. |
| Q-9002 | `BAD_PASSWORD` | A password is too short or too long. |
| Q-9003 | `TEXT_TOO_LONG` | Text is too long, or holds control characters. |
| Q-9004 | `REQUIRED_MISSING` | A required field is empty or too long. |
| Q-9005 | `BAD_TAG_NAME` | A tag needs Latin letters or digits. |
| Q-9006 | `BAD_TAG_FORM` | A tag must be written as type:name. |
| Q-9007 | `BAD_TAG_TYPE_NAME` | A tag type's name must be letters, digits and hyphens. |
| Q-9008 | `TOO_MANY_TAGS` | A prompt has more tags than it can carry. |
| Q-9009 | `BAD_COLOUR` | A colour must be written as #rrggbb. |
| Q-9010 | `BAD_RESERVED_HANDLE` | A reserved handle entry isn't a valid handle. |
| Q-9011 | `BAD_CHANNEL` | A channel name must be a short slug. |
| Q-9012 | `BAD_STAFF_NAME` | A staff name is empty, too long, or reserved. |
| Q-9013 | `BAD_TAG_LIMITS` | A tag type's minimum and maximum don't fit together. |

## Q-99xx: Not yet given a code of its own

| Code | Name | Meaning |
| --- | --- | --- |
| Q-9950 | `UNCLASSIFIED` | Something went wrong that has no code of its own yet. Advanced mode shows the detail; report it with that. |
