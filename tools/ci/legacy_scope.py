#!/usr/bin/env python3
"""CI01 trusted-base scope gate. No PR imports, hooks, build or execution.

The caller must load this script AND policy from the trusted base. The separate
pull_request_target workflow publishes a head status without running PR code.
The read-only build workflow also runs this gate before dispatching jobs.
"""
from __future__ import annotations

import argparse
import base64
import fnmatch
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import urllib.request


class Rejected(ValueError):
    pass


def sha(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{40}", value):
        raise Rejected("missing/invalid commit SHA")
    return value


def classify(event, kind, policy):
    repo = policy["repository"]
    if event.get("repository", {}).get("full_name") != repo:
        raise Rejected("unexpected repository")
    if kind in ("pull_request", "pull_request_target"):
        pr = event["pull_request"]
        base, head = pr["base"], pr["head"]
        if base["repo"]["full_name"] != repo:
            raise Rejected("unexpected base repository")
        if not isinstance(pr["number"], int) or pr["number"] <= 0:
            raise Rejected("invalid PR number")
        same_repo = head.get("repo", {}).get("full_name") == repo
        if base["ref"] == policy["main_branch"] and same_repo and head["ref"] == policy["integration_branch"]:
            role = "integration"
        elif base["ref"] == policy["integration_branch"] and not (same_repo and head["ref"] == policy["integration_branch"]):
            role = "task"
        else:
            raise Rejected("not a registered legacy integration/task PR")
        return dict(role=role, base=sha(base["sha"]), head=sha(head["sha"]), number=pr["number"])
    if kind not in ("push", "workflow_dispatch") or event.get("ref") != "refs/heads/" + policy["integration_branch"]:
        raise Rejected("not a registered integration event")
    return dict(role="aggregate", base=None, head=None, number=None)


def inspect_paths(paths, role, policy):
    paths = sorted(set(paths))
    if not paths or any(not p or p.startswith("/") or ".." in p.split("/") for p in paths):
        raise Rejected("empty or invalid changed paths")
    forbidden = [p for p in paths if any(fnmatch.fnmatchcase(p, rule) for rule in policy["protected"])]
    if role == "task" and forbidden:
        raise Rejected("central changes require the integrator: " + ", ".join(forbidden))
    return "docs" if all(p.endswith(".md") for p in paths) else "full"


def metadata(body, policy):
    """Declarations are for coordination, never an authorization mechanism."""
    def field(name):
        match = re.search(r"(?im)^" + re.escape(name) + r":[ \t]*([^\r\n]+)$", body)
        if not match:
            raise Rejected("task PR missing " + name)
        return match[1].strip()
    if not re.fullmatch(r"[A-Z]+[0-9]{2}[a-z]?", field("Task-ID")):
        raise Rejected("invalid Task-ID")
    if field("Contract-Version") != str(policy["contract_version"]):
        raise Rejected("contract version mismatch")
    # Dependency resolution is still reviewed by the integrator. A self-reported
    # Depends-on line cannot prove merged dependencies or authorize shared edits.
    for name in ("Variant", "Depends-on", "Scope"):
        field(name)


def git(root, *args, env=None):
    return subprocess.check_output(
        ["git", "-C", str(root), *args], env=env, stderr=subprocess.PIPE,
    )


def changed_paths(root, base, head):
    # Disable rename detection: both old and new names are checked, including
    # rename OUT of a protected path, copies, deletions and type changes.
    return [p.decode("utf-8", errors="strict") for p in git(
        root, "diff", "--no-ext-diff", "--no-textconv", "--name-only", "-z", "--no-renames", base + "..." + head,
    ).split(b"\0") if p]


def fetch_head(root, number):
    env = os.environ.copy()
    token = env.get("GH_TOKEN", "")
    if token:
        auth = base64.b64encode(("x-access-token:" + token).encode()).decode()
        env.update(GIT_CONFIG_COUNT="1", GIT_CONFIG_KEY_0="http.extraheader", GIT_CONFIG_VALUE_0="AUTHORIZATION: basic " + auth)
    git(root, "fetch", "--no-tags", "--no-recurse-submodules", "origin", f"refs/pull/{number}/head", env=env)


def publish_status(policy, context, passed, description):
    # Endpoint and head come only from validated trusted policy/event, not PR text.
    data = json.dumps({"state": "success" if passed else "failure", "context": "legacy/scope",
                       "description": description[:140]}).encode()
    req = urllib.request.Request(
        f'https://api.github.com/repos/{policy["repository"]}/statuses/{context["head"]}',
        data=data, method="POST", headers={"Authorization": "Bearer " + os.environ["GH_TOKEN"],
        "Accept": "application/vnd.github+json", "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as response:
        response.read()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--policy", type=Path, required=True)
    parser.add_argument("--event-file", type=Path, default=os.environ.get("GITHUB_EVENT_PATH"))
    parser.add_argument("--event-name", default=os.environ.get("GITHUB_EVENT_NAME"))
    parser.add_argument("--repo-dir", type=Path, default=Path.cwd())
    parser.add_argument("--fetch", action="store_true")
    parser.add_argument("--publish", action="store_true")
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    policy = json.loads(args.policy.read_text())
    event = json.loads(args.event_file.read_text())
    context = None
    try:
        context = classify(event, args.event_name, policy)
        if context["role"] == "aggregate":
            context["head"] = sha(git(args.repo_dir, "rev-parse", "HEAD").decode().strip())
            mode = "full"  # Every integration head gets an actual aggregate matrix.
        else:
            if args.fetch:
                fetch_head(args.repo_dir, context["number"])
            paths = changed_paths(args.repo_dir, context["base"], context["head"])
            mode = inspect_paths(paths, context["role"], policy)
            if context["role"] == "task":
                metadata(event["pull_request"].get("body") or "", policy)
        report = dict(context, status="passed", mode=mode,
                      candidate=git(args.repo_dir, "rev-parse", "HEAD").decode().strip())
        if args.publish:
            publish_status(policy, context, True, "C01 scope accepted; base " + context["base"][:12])
    except (Rejected, KeyError, ValueError, subprocess.CalledProcessError) as error:
        # Never echo subprocess environment/headers. Git failure stderr may be
        # attacker-influenced, so keep it out of workflow command annotations.
        reason = str(error) if not isinstance(error, subprocess.CalledProcessError) else "git object/diff check failed"
        report = dict(context or {}, status="failed", error=reason)
        if args.publish and context and context.get("head"):
            publish_status(policy, context, False, "Scope/metadata rejected; see trusted scope run")
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, ensure_ascii=True))
    if report["status"] != "passed":
        return 1
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as out:
            for key in ("mode", "head", "base", "candidate", "role"):
                out.write(f'{key}={report[key] or ""}\n')
    return 0


if __name__ == "__main__":
    sys.exit(main())
