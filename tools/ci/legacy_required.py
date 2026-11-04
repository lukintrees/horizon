#!/usr/bin/env python3
"""Fail closed if any job in the selected CI01 matrix did not succeed."""
import json
import os
import sys
import urllib.request


def validate(needs, mode):
    expected = {"scope", "policy-tests"}
    if mode == "full":
        expected.update({"terrain-integration", "gl2-runtime", "windows-x86-abi", "modern"})
    elif mode != "docs":
        raise ValueError("missing/invalid CI mode")
    for name in expected:
        if needs.get(name, {}).get("result") != "success":
            raise ValueError("required job not successful: " + name)
    for name, value in needs.items():
        if name not in expected and value["result"] != "skipped":
            raise ValueError("unexpected matrix result: " + name)


def current_candidate(event, report, candidate, pr):
    """A moved base/head/merge result cannot reuse an earlier candidate result."""
    if (pr["base"]["sha"] != report["base"] or pr["head"]["sha"] != report["head"]
            or pr.get("merge_commit_sha") != candidate):
        raise ValueError("PR base/head/merge moved; rerun on the current candidate")
    if event["pull_request"]["number"] != pr["number"]:
        raise ValueError("unexpected PR metadata")


def main():
    needs = json.loads(os.environ["NEEDS_JSON"])
    report = json.loads(needs["scope"]["outputs"]["report"])
    validate(needs, report["mode"])
    if os.environ["GITHUB_EVENT_NAME"] == "pull_request":
        with open(os.environ["GITHUB_EVENT_PATH"]) as source:
            event = json.load(source)
        number = event["pull_request"]["number"]
        req = urllib.request.Request(
            f'https://api.github.com/repos/{os.environ["GITHUB_REPOSITORY"]}/pulls/{number}',
            headers={"Authorization": "Bearer " + os.environ["GH_TOKEN"],
                     "Accept": "application/vnd.github+json"},
        )
        with urllib.request.urlopen(req, timeout=30) as response:
            pr = json.load(response)
        current_candidate(event, report, os.environ["GITHUB_SHA"], pr)
    print(json.dumps({"status": "passed", "evidence": report}))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
