# Bind a change record to actual file bytes

Use the project's existing record schema. This example has three SHA-256 fields
and explicit unexecuted/unqualified flags; it is not a new harbor-cad API or an
approval. Adapt field names only when the local schema differs.

First finish the new requirements file. Set `SOURCE_REQUIREMENTS`,
`HISTORICAL_RECEIPT`, `NEW_REQUIREMENTS` and `CHANGE_RECORD` to explicit paths;
the last path must not exist. Run this in Bash with installed `sha256sum`, `cut`
and `jq`. Never paste a digest from a manifest, tool display or model memory.
Hash through stdin so unusual filenames cannot escape into the digest string.

```sh
set -euo pipefail
set -o noclobber
source_requirements_sha256=$(sha256sum <"$SOURCE_REQUIREMENTS" | cut -d ' ' -f1)
historical_receipt_sha256=$(sha256sum <"$HISTORICAL_RECEIPT" | cut -d ' ' -f1)
requirements_sha256=$(sha256sum <"$NEW_REQUIREMENTS" | cut -d ' ' -f1)
# ponytail: direct new-file write; use the project's atomic publisher for concurrent readers.
jq -n \
  --arg source_requirements_sha256 "$source_requirements_sha256" \
  --arg historical_receipt_sha256 "$historical_receipt_sha256" \
  --arg requirements_sha256 "$requirements_sha256" \
  '{$source_requirements_sha256, $historical_receipt_sha256, $requirements_sha256,
    executed:false, qualification:"unqualified", approval_required:true}' >"$CHANGE_RECORD"
jq -e \
  --arg source_requirements_sha256 "$source_requirements_sha256" \
  --arg historical_receipt_sha256 "$historical_receipt_sha256" \
  --arg requirements_sha256 "$requirements_sha256" \
  '. == {$source_requirements_sha256, $historical_receipt_sha256, $requirements_sha256,
    executed:false, qualification:"unqualified", approval_required:true}' "$CHANGE_RECORD" >/dev/null
```

The read-back check verifies the emitted record, not merely printed hashes. Also
run the existing project contract/preservation checks: matching digests do not
prove approved content, native execution, unchanged geometry or physics. Inputs
must remain stable; this pattern is not an adversarial snapshot mechanism.
Retain a failure rather than silently repairing a historical or scored artifact.
