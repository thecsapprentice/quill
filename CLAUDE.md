# Working on Quill

## Planning and tracking work

- **Write tasks up as GitHub issues, not as documents.** Future tasks, and the
  steps of each implementation phase or sub-phase, are opened as issues on the
  repo (one feature issue per sub-phase or task, titled like
  `Phase 3b: <name>`, labelled `enhancement`, with a Scope checklist and a
  "Done when"). Don't put task lists in a new document or only in
  `docs/roadmap.md`; the roadmap stays the high-level record and links to the
  issues.
- **Link every PR to its issue.** When you open a PR, reference the issue it
  implements in the description (`Closes #N` when it finishes the issue,
  `Refs #N` when it's partial). If a PR covers several issues, list each one.
  Tick off the issue's checklist items as they land.
- If work turns up that isn't covered by an issue, open one before or with the
  PR rather than leaving it in a note.
