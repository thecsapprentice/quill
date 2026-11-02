#!/usr/bin/env bash
# Mechanical half of the Quill -> Plait rename (docs/rebrand-plait.md).
#
# Run from a clean checkout on its own branch, one phase at a time, and
# commit after each so every step stays reviewable:
#
#   scripts/rebrand-to-plait.sh crates    # directories, packages, Rust paths
#   scripts/rebrand-to-plait.sh brand     # every other "quill" (env, files, prose)
#   scripts/rebrand-to-plait.sh audit     # list what still says quill, and why
#
# The guide's manual steps (test vectors, the reserved-handles migration,
# the vocabulary pass) come after these.
set -euo pipefail
cd "$(dirname "$0")/.."

# Never rewritten:
#  - spikes/ and docs/design/: dated records of past work.
#  - migrations 0001-0029: already applied; new names go in a new migration.
#  - test-vectors/*.json: regenerated from code (PLAIT_WRITE_VECTORS=1), never
#    edited, because some vectors hash or encrypt the old labels.
#  - words.rs: its test uses "quill" as a word that isn't in the BIP-39 list.
#  - assets/brand and this script, which talk about the rename.
EXCLUDE=(
  ':!spikes/**'
  ':!docs/design/**'
  ':!crates/*/migrations/**'
  ':!test-vectors/*.json'
  ':!crates/*/src/words.rs'
  ':!assets/brand/**'
  ':!scripts/rebrand-to-plait.sh'
  ':!docs/rebrand-plait.md'
)

files_matching() {
  git grep -lIE "$1" -- . "${EXCLUDE[@]}" || true
}

phase_crates() {
  # crates/quill-x -> crates/plait-x, and crates/quill (the CLI) -> crates/plait.
  for dir in crates/quill-* crates/quill; do
    [[ -d $dir ]] || continue
    git mv "$dir" "${dir/quill/plait}"
  done
  # Package names and Rust paths. The bare CLI crate is matched only in the
  # forms Cargo and the tests use, so prose is left for the brand phase.
  local pat='quill[-_](board[-_]ui|certs|core|devtools|e2e|gui|mls[-_]envelope|proto|server|ui|seed)\b|crates/quill\b|-p quill\b|CARGO_BIN_EXE_quill\b|^name = "quill"$'
  files_matching "$pat" | while read -r f; do
    sed -E -i \
      -e 's/quill([-_])(board[-_]ui|certs|core|devtools|e2e|gui|mls[-_]envelope|proto|server|ui|seed)\b/plait\1\2/g' \
      -e 's#crates/quill\b#crates/plait#g' \
      -e 's/-p quill\b/-p plait/g' \
      -e 's/CARGO_BIN_EXE_quill\b/CARGO_BIN_EXE_plait/g' \
      -e 's/^name = "quill"$/name = "plait"/' \
      "$f"
  done
  # Workspace dependency key for the CLI crate, if it has one.
  sed -E -i 's/^quill = \{ path = "crates\/plait"/plait = { path = "crates\/plait"/' Cargo.toml
}

phase_brand() {
  # Everything else: env vars, file names, format labels, cookies, prose.
  files_matching 'quill|Quill|QUILL' | while read -r f; do
    sed -E -i -e 's/QUILL/PLAIT/g' -e 's/Quill/Plait/g' -e 's/quill/plait/g' "$f"
  done
}

phase_audit() {
  echo "Still saying quill (each should be in the guide's keep list):"
  git grep -nIi quill -- . ':!spikes/**' ':!docs/design/**' ':!assets/brand/**' \
    ':!scripts/rebrand-to-plait.sh' ':!docs/rebrand-plait.md' || true
}

case "${1:-}" in
  crates) phase_crates ;;
  brand) phase_brand ;;
  audit) phase_audit ;;
  *) echo "usage: $0 crates|brand|audit" >&2; exit 2 ;;
esac
