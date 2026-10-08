#!/usr/bin/env bash
# GitLab: upload dist/* to the Generic Package Registry and create a release
# that links them. Uses only curl and the CI job token.
set -euo pipefail
: "${CI_COMMIT_TAG:?run on a tag pipeline}"
version=${CI_COMMIT_TAG#v}
pkg_url="${CI_API_V4_URL}/projects/${CI_PROJECT_ID}/packages/generic/ct/${version}"

cd dist
links=""
for f in ct-* SHA256SUMS; do
    echo ">> uploading $f"
    curl --fail-with-body --silent --show-error \
        --header "JOB-TOKEN: ${CI_JOB_TOKEN}" \
        --upload-file "$f" "${pkg_url}/${f}"
    echo
    links="${links:+${links},}{\"name\":\"${f}\",\"url\":\"${pkg_url}/${f}\",\"link_type\":\"package\"}"
done
cd ..

notes=$(ci/release-notes.sh "$CI_COMMIT_TAG" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')
payload=$(printf '{"name":"ct %s","tag_name":"%s","description":%s,"assets":{"links":[%s]}}' \
    "$CI_COMMIT_TAG" "$CI_COMMIT_TAG" "$notes" "$links")

echo ">> creating release $CI_COMMIT_TAG"
curl --fail-with-body --silent --show-error \
    --header "JOB-TOKEN: ${CI_JOB_TOKEN}" \
    --header "Content-Type: application/json" \
    --data "$payload" \
    "${CI_API_V4_URL}/projects/${CI_PROJECT_ID}/releases"
echo
