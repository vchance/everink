#!/usr/bin/env python3
"""Decides what a scheduled run of `ci` does (the integration lane of core D2).

The lane runs every job on the head of the default branch, but only when that head is not already covered:

  covered by a successful run on this head         -> skip (run nothing, pass)
  covered by a failed run on this head             -> carry the failure (run nothing, fail)
  head moved, last covering run succeeded < 4 h ago -> gate (run nothing, pass; evidence of nothing)
  anything else, or any error                      -> run every job

The covering run is the latest scheduled or dispatched run on the default branch that executed its product jobs.
A run whose product jobs were all skipped (an earlier skip or gate) does not count. The lane fails toward running.

Usage:
  ci-lane.py            prints one line: `run`, `skip`, `gate` or `carry-failure`, then a reason on stderr
  ci-lane.py --self-test
"""
import datetime
import json
import os
import subprocess
import sys

PRODUCT_JOBS = ("lint", "test", "text-trial", "render-trial")
# How many runs' job lists one decision may fetch. Past this the lane stops looking and runs, so a long row of
# skip runs can never make the `changes` job slow.
MAX_JOB_LOOKUPS = 40
LANE_EVENTS = ("schedule", "workflow_dispatch")
GATE = datetime.timedelta(hours=4)


def executed(jobs):
    """True when the run executed at least one product job (it was not a skip or gate run)."""
    product = [j for j in jobs if j.get("name") in PRODUCT_JOBS]
    return bool(product) and any(j.get("conclusion") != "skipped" for j in product)


def find_covering(runs, jobs_of, branch, exclude_id=None):
    """`runs` newest first. Returns the latest lane run on `branch` that executed its product jobs, or None."""
    looked = 0
    for r in runs:
        if r.get("id") == exclude_id or r.get("status") != "completed":
            continue
        if r.get("event") not in LANE_EVENTS or r.get("head_branch") != branch:
            continue
        if looked >= MAX_JOB_LOOKUPS:
            return None
        looked += 1
        if executed(jobs_of(r["id"])):
            return r
    return None


def decide(head, now, covering):
    if covering is None:
        return "run", "no covering run"
    rid, sha, conclusion = covering["id"], covering["head_sha"], covering.get("conclusion")
    if sha == head:
        if conclusion == "success":
            return "skip", f"covered by run {rid} on {sha}"
        if conclusion == "failure":
            return "carry-failure", f"carrying the failure of run {rid} on {sha}"
        return "run", f"covering run {rid} on this head ended {conclusion}"
    if conclusion == "success":
        started = datetime.datetime.fromisoformat(covering["run_started_at"].replace("Z", "+00:00"))
        if now - started < GATE:
            return "gate", f"lane gate: covering run {rid} started {covering['run_started_at']}"
        return "run", f"head moved and covering run {rid} started {covering['run_started_at']}, 4 hours ago or more"
    return "run", f"head moved and covering run {rid} ended {conclusion}"


def gh_json(path):
    out = subprocess.run(["gh", "api", path], check=True, capture_output=True, text=True, timeout=60).stdout
    return json.loads(out)


def main():
    repo, head = os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_SHA"]
    branch = os.environ.get("DEFAULT_BRANCH", "main")
    this_run = int(os.environ.get("GITHUB_RUN_ID", "0")) or None
    base = f"repos/{repo}/actions/workflows/ci.yml/runs?status=completed&per_page=100"

    def jobs_of(run_id):
        return gh_json(f"repos/{repo}/actions/runs/{run_id}/jobs?per_page=100").get("jobs", [])

    # First the runs on this head (answers the covered cases however quiet the branch has been), then the newest
    # runs on the branch.
    covering = find_covering(gh_json(f"{base}&head_sha={head}").get("workflow_runs", []), jobs_of, branch, this_run)
    if covering is None:
        newest = []
        for page in (1, 2):  # the newest 200 runs on the branch
            newest += gh_json(f"{base}&branch={branch}&page={page}").get("workflow_runs", [])
        covering = find_covering(newest, jobs_of, branch, this_run)
    return decide(head, datetime.datetime.now(datetime.timezone.utc), covering)


def self_test():
    t0 = datetime.datetime(2026, 11, 12, 12, 0, tzinfo=datetime.timezone.utc)
    ago = lambda h: (t0 - datetime.timedelta(hours=h)).isoformat().replace("+00:00", "Z")
    ran = [{"name": "changes", "conclusion": "success"}, {"name": "lint", "conclusion": "success"},
           {"name": "test", "conclusion": "success"}]
    skipped = [{"name": "changes", "conclusion": "success"}, {"name": "lint", "conclusion": "skipped"},
               {"name": "test", "conclusion": "skipped"}]
    half = [{"name": "lint", "conclusion": "skipped"}, {"name": "test", "conclusion": "failure"}]

    def run(i, event, sha, conclusion, hours, jobs, branch="main", status="completed"):
        return {"id": i, "event": event, "head_sha": sha, "conclusion": conclusion, "run_started_at": ago(hours),
                "head_branch": branch, "status": status, "_jobs": jobs}

    def case(name, runs, head, want):
        by_id = {r["id"]: r["_jobs"] for r in runs}
        got, why = decide(head, t0, find_covering(runs, lambda i: by_id[i], "main", exclude_id=999))
        assert got == want, f"{name}: wanted {want}, got {got} ({why})"

    case("no runs at all", [], "aaa", "run")
    case("covered, success", [run(1, "schedule", "aaa", "success", 1, ran)], "aaa", "skip")
    case("covered, failure is carried", [run(1, "schedule", "aaa", "failure", 1, half)], "aaa", "carry-failure")
    case("covered, cancelled runs again", [run(1, "schedule", "aaa", "cancelled", 1, ran)], "aaa", "run")
    case("head moved, recent success gates", [run(1, "schedule", "old", "success", 3, ran)], "aaa", "gate")
    case("head moved, success 4 hours old runs", [run(1, "schedule", "old", "success", 4, ran)], "aaa", "run")
    case("head moved, failure never gates", [run(1, "schedule", "old", "failure", 1, half)], "aaa", "run")
    case("skip runs do not cover", [run(2, "schedule", "aaa", "success", 1, skipped),
                                    run(1, "schedule", "aaa", "success", 2, ran)], "aaa", "skip")
    case("only skip runs: run", [run(2, "schedule", "aaa", "success", 1, skipped)], "aaa", "run")
    case("push and pull_request runs never cover", [run(2, "push", "aaa", "success", 1, ran),
                                                    run(1, "pull_request", "aaa", "success", 1, ran)], "aaa", "run")
    case("a dispatched run covers", [run(1, "workflow_dispatch", "aaa", "success", 1, ran)], "aaa", "skip")
    case("another branch does not cover", [run(1, "workflow_dispatch", "aaa", "success", 1, ran, branch="topic")],
         "aaa", "run")
    case("the newest covering run decides", [run(3, "schedule", "aaa", "failure", 1, half),
                                             run(2, "schedule", "aaa", "success", 2, ran)], "aaa", "carry-failure")
    case("this run is ignored", [run(999, "schedule", "aaa", "success", 0, ran)], "aaa", "run")
    case("a run still in progress is ignored", [run(1, "schedule", "aaa", None, 0, ran, status="in_progress")],
         "aaa", "run")
    many_skips = [run(100 - i, "schedule", "aaa", "success", i, skipped) for i in range(60)]
    case("a long row of skip runs stops the search and runs", many_skips + [run(1, "schedule", "aaa", "success", 70, ran)],
         "aaa", "run")
    case("a run with no product jobs does not cover", [run(1, "schedule", "aaa", "success", 1,
                                                           [{"name": "changes", "conclusion": "success"}])],
         "aaa", "run")
    print("ci-lane self-test: 17 cases passed")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        sys.exit(0)
    try:
        decision, why = main()
    except Exception as e:  # any error fails toward running
        decision, why = "run", f"could not read earlier runs ({type(e).__name__}: {e})"
    print(why, file=sys.stderr)
    print(decision)
