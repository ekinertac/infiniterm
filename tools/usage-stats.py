#!/usr/bin/env python3
"""How many Macs run infiniterm, from GitHub's own download counters.

The app has no telemetry and the site says so. What exists anyway: every
installed copy fetches `latest.json` from the newest release at launch and
every six hours (infiniterm-ui/src/updater.rs), and GitHub counts each
download of each release asset. So the newest release's `latest.json`
count, divided by the days it has been the newest and by the fetches one
Mac makes a day, estimates the Macs running it. The zip counts updates
installed, the DMG counts manual first installs (Homebrew fetches the DMG
too).

It is an estimate and prints as one: a Mac relaunched often fetches more,
one asleep fetches less, and the counters are cumulative with no dates, so
a single release's rate is all there is. The launch plan counts success in
it (docs/marketing-plan.md).

    tools/usage-stats.py              reads the live counters through `gh`
    USAGE_STATS_JSON=file tools/...   reads a saved `gh api` response (tests)

tools/test-usage-stats.sh checks the arithmetic against a fixture.
"""
import json
import os
import subprocess
import sys
from datetime import datetime, timezone

REPO = "ekinertac/infiniterm"
# At launch plus every six hours: four a day for a Mac that stays up, more
# for one relaunched. Four keeps the estimate on the low side.
FETCHES_PER_MAC_PER_DAY = 4


def releases():
    path = os.environ.get("USAGE_STATS_JSON")
    if path:
        with open(path) as f:
            return json.load(f)
    out = subprocess.run(
        ["gh", "api", f"repos/{REPO}/releases?per_page=30"],
        capture_output=True, text=True, check=True,
    ).stdout
    return json.loads(out)


def now():
    fixed = os.environ.get("USAGE_STATS_NOW")
    if fixed:
        return datetime.fromisoformat(fixed.replace("Z", "+00:00"))
    return datetime.now(timezone.utc)


def counts(release):
    by_kind = {"latest.json": 0, "zip": 0, "dmg": 0}
    for asset in release.get("assets", []):
        name = asset["name"]
        kind = "latest.json" if name == "latest.json" else name.rsplit(".", 1)[-1]
        if kind in by_kind:
            by_kind[kind] += asset["download_count"]
    return by_kind


def main():
    rels = [r for r in releases() if not r.get("draft")]
    if not rels:
        print("no releases", file=sys.stderr)
        return 1
    rels.sort(key=lambda r: r["published_at"], reverse=True)
    print(f"{'release':<16} {'published':<11} {'latest.json':>11} {'zip':>5} {'dmg':>5}")
    for r in rels:
        c = counts(r)
        print(f"{r['tag_name']:<16} {r['published_at'][:10]:<11} "
              f"{c['latest.json']:>11} {c['zip']:>5} {c['dmg']:>5}")
    newest = rels[0]
    published = datetime.fromisoformat(newest["published_at"].replace("Z", "+00:00"))
    days = max((now() - published).total_seconds() / 86400, 1 / 24)
    fetches = counts(newest)["latest.json"]
    per_day = fetches / days
    macs = per_day / FETCHES_PER_MAC_PER_DAY
    print()
    print(f"{newest['tag_name']}: {fetches} update checks in {days:.1f} days, "
          f"{per_day:.0f} a day, about {macs:.0f} Macs running it (estimate)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
