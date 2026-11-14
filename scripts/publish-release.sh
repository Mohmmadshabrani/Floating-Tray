#!/usr/bin/env bash
set -euo pipefail

tag=${1:?Provide a release tag}
dist=${2:?Provide the package directory}
setup=Floating-Tray-Setup-x64.exe
portable=Floating-Tray-Windows-x64.zip
sums=SHA256SUMS.txt

for name in "$setup" "$portable"; do
    test -s "$dist/$name"
done

if ! gh release view "$tag" >/dev/null 2>&1; then
    gh release create "$tag" --verify-tag --generate-notes
fi

existing=$(gh release view "$tag" --json assets --jq '.assets[] | select(.state == "uploaded") | .name')
published=$(mktemp -d)
trap 'rm -rf "$published"' EXIT

for name in "$setup" "$portable"; do
    if grep -Fxq "$name" <<<"$existing"; then
        gh release download "$tag" --pattern "$name" --dir "$published"
    else
        gh release upload "$tag" "$dist/$name"
        cp "$dist/$name" "$published/$name"
    fi
done

if grep -Fxq "$sums" <<<"$existing"; then
    gh release download "$tag" --pattern "$sums" --dir "$published"
    (cd "$published" && sha256sum --check --strict "$sums")
else
    (cd "$published" && sha256sum "$setup" "$portable" > "$sums")
    gh release upload "$tag" "$published/$sums"
fi
