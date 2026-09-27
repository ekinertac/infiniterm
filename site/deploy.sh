#!/bin/sh
# Publishes the site: builds it (reference pages regenerated from the app's
# tables first, see package.json) and force-pushes dist/ as the only commit
# on the gh-pages branch of the PUBLIC ekinertac/infiniterm-releases, which
# GitHub Pages serves at infiniterm.app (public/CNAME, DNS on Cloudflare).
#
# One orphan commit each time, so the public branch carries no history of
# this private repo, only the built files. The releases themselves (DMGs,
# zips, latest.json the updater reads) are GitHub Release assets on that
# repo, not branch files, so this never touches them.
set -eu
cd "$(dirname "$0")"
repo=git@github.com:ekinertac/infiniterm-releases.git
npm run build
[ -f dist/CNAME ] || { echo "deploy: dist/CNAME missing; Pages would drop the custom domain" >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cp -R dist/. "$tmp"
# Pages would otherwise run Jekyll and drop _astro/, which starts with _.
touch "$tmp/.nojekyll"
cd "$tmp"
git init -q -b gh-pages
git add -A
git -c user.name="$(git -C "$OLDPWD" config user.name)" -c user.email="$(git -C "$OLDPWD" config user.email)" \
    commit -q -m "Site build from $(git -C "$OLDPWD" rev-parse --short HEAD)"
git push -q -f "$repo" gh-pages
echo "deploy: pushed gh-pages; https://infiniterm.app"
