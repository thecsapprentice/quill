# Board layout mockups

Mockups for the responsive layout of the server's web board (`crates/quill-board-ui`). They were chosen over three rounds of review on 2026-10-27. Each screen is here in two forms:

- `html/`: the mockup itself, standalone HTML and CSS with no scripts. Open it in a browser. The layout and the exact colours, spacing and type sizes are in its `<style>` block and inline styles.
- `png/`: a screenshot at the mockup's size. Desktop is 1440×900 and mobile is 390×844.

These are drawings, not code. All names, counts and dates are made up, and the inline styles have to become classes in `board.css` because the board's Content Security Policy forbids inline styles. Links between the HTML files work, so you can click from the board to the filters, for example.

## Screens

### Desktop

| File | Screen | Notes |
|---|---|---|
| D1-board | Board: list and reading pane | Order switch from #45, saved searches, filter summary, "refreshed" and first-partner labels |
| D2-filters | Filter builder | Tag types on the left, one type's tags in the middle (search, A–Z, paging, Any/Has/Hide), the whole filter on the right |
| D3-post | Post a prompt | Wide form, preview, tag panel that scales to hundreds of tags, per-type min/max counters, first-partner checkbox |
| D4-overview | Admin overview | Each admin picks, orders and sizes their own panels |
| D5-tags | Admin tags at scale | Tag types in the admin sidebar; table with search, sort, paging, bulk actions; tag editor rail with similar tags |
| D6-my-prompts | My prompts | Refresh buttons, refresh allowance (two rules), posting limits |
| D7-tag-type | Admin tag type settings | Min/max per prompt, presets, warning when a lower limit affects existing prompts |
| D8-board-settings | Admin board settings | Posting limits, refresh rules, which board orders are offered |
| A3-reports | Admin reports | From round 1. The table and detail rail are the chosen pattern; its sidebar should become the D4/D5 admin sidebar under the top bar |

### Mobile

| File | Screen | Notes |
|---|---|---|
| N1-board | Board | Compact rows, one row of buttons (Filters, saved search, order), one-line filter summary, bottom tab bar |
| N2-filters | Filters | Basics and tag types, each showing what's set; drill into a type |
| N3-filter-type | Filters: one tag type | Search, selected chips, most used, A–Z, Has/Hide |
| N4-prompt | Prompt page | Clean reading layout; the dashed "Ask to write together" button is a placeholder for the future app hand-off |
| N5-post-tags | Post: tag picker | Required slots, per-type min/max counters |
| N6-sort | Order and saved searches | The four orders with their one-line explanations |
| N7-my-prompts | My prompts | Refresh buttons and allowance |
| M3b-post | Post form | From round 1; "Browse tags" opens N5 |
| M2b-notices | Notices | From round 1 |
| M2c-more | More (account and admin) | From round 1; the admin entry point on mobile |

## Decisions from the review

- **Kept:**
  - the list board with a reading pane (not cards);
  - the admin sidebar with a list and a detail rail;
  - a wide post form with a tag browser and a preview;
  - three-pane tag management;
  - the bottom tab bar and More screen on mobile;
  - the compact mobile rows, with tags as plain text rather than coloured chips;
  - dashed "required" tag slots.
- **Rejected:**
  - the masonry card grid (too cluttered);
  - a tall mobile header (takes space from the board);
  - a menu sheet that mixed a grid with a list.
- **Filters** collapse to a fixed-size summary, so complex filters never push the list down. Building a filter happens on its own page (desktop) or screen (mobile), never in a popup.
- **The admin overview** is per admin, not one layout for the whole server.
- **Refreshing** has two rules: each prompt once every N days, and each account K refreshes every W days. A used refresh comes back W days after it was used, even if the prompt is later closed or deleted.
