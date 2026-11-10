#!/usr/bin/env bash
# Prints `version=<version>` for $GITHUB_OUTPUT (docs/release.md). On main it is YYYY.M.N: N is
# this month's release count plus one, or the existing number when this commit was released
# already (a re-run). Any other branch gets test-<sha7>, which never becomes a tag or release.
set -euo pipefail

sha=${GITHUB_SHA:-$(git rev-parse HEAD)}
if [ "${GITHUB_REF:-}" != "refs/heads/main" ]; then
  echo "version=test-${sha:0:7}"
  exit 0
fi

tags=$(git ls-remote --tags origin 'refs/tags/v*')
# An annotated tag's commit is on its peeled `^{}` line; a lightweight tag points at it directly.
existing=$(awk -v sha="$sha" '$1 == sha { sub("refs/tags/v", "", $2); sub("\\^\\{\\}", "", $2); print $2; exit }' <<< "$tags")
if [ -n "$existing" ]; then
  echo "version=$existing"
  exit 0
fi

year=$(date -u +%Y)
month=$(date -u +%-m)
count=$(grep -cE "refs/tags/v$year\.$month\.[0-9]+$" <<< "$tags" || true)
echo "version=$year.$month.$((count + 1))"
