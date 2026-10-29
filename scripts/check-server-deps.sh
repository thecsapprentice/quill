#!/usr/bin/env bash
# Enforces the core security invariant: the server can never decrypt a
# writing session, because no code able to do so is linked into it.
#
# Fails if any MLS, HPKE, client-side crypto or client crate appears anywhere
# in the dependency tree of the crates the server links.
set -euo pipefail

cd "$(dirname "$0")/.."

# Crates the server binary links.
SERVER_CRATES=(quill-server quill-proto quill-mls-envelope quill-board-ui quill-certs)

# Banned crate names (exact match, or prefix when ending in '*').
BANNED=(
  'openmls*'        # MLS implementation and its providers
  'mls-rs*'         # alternative MLS implementation
  'hpke*'           # HPKE, used to decrypt MLS Welcome messages
  'libcrux*'        # crypto backend of openmls_libcrux_crypto
  'x25519-dalek'    # key agreement: only clients do this
  'age'             # backup encryption: only clients do this
  'quill-core'      # client library: holds all session keys
  'quill'           # CLI client
  'quill-ui'        # the writing client's views (sessions are decrypted there)
  'quill-gui'       # desktop client
)

status=0
for crate in "${SERVER_CRATES[@]}"; do
  deps=$(cargo tree --locked -p "$crate" -e normal,build --prefix none --format '{p}' \
    | awk '{print $1}' | sort -u)
  for pattern in "${BANNED[@]}"; do
    for dep in $deps; do
      # shellcheck disable=SC2053  # intentional glob match
      if [[ $dep == $pattern ]]; then
        echo "BANNED: '$dep' is in the dependency tree of $crate (rule '$pattern')" >&2
        cargo tree --locked -p "$crate" -e normal,build -i "$dep" >&2 || true
        status=1
      fi
    done
  done
done

if [[ $status -eq 0 ]]; then
  echo "OK: no banned crates in the server's dependency tree (${SERVER_CRATES[*]})"
fi
exit $status
