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
