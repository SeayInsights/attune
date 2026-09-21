"""Invariants on whatever in .github/workflows can publish a GitHub release.

Both of these were live and neither was caught by a test:

  * A ``workflow_dispatch`` input reached the release step from any branch
    while the tag/AppVersion guard -- which keys off ``github.ref`` -- sat
    skipped. A different knob opened the same publishing path.
  * Two workflows could answer the same tag. That one is fixed by namespacing
    Attune's tags, but the inherited ``release.yml`` still creates releases and
    an upstream merge could give it a push trigger again at any time.

So this asserts on the shape rather than on the two files that happened to be
wrong: any step that creates a release must be guarded by the same condition as
the version check in its job, and at most one workflow may answer a tag push.

Run it the way CI does:  py ci/check-release-workflow.py
"""

from __future__ import annotations

import sys
from pathlib import Path

import yaml

WORKFLOWS = Path(__file__).resolve().parent.parent / ".github" / "workflows"

# Steps that hand an artifact to the outside world.
PUBLISHERS = ("softprops/action-gh-release", "gh release create", "ncipollo/release-action")


def _on(doc: dict) -> dict:
    """`on:` parses as the boolean True in YAML 1.1, so look for both."""
    raw = doc.get("on", doc.get(True))
    return raw if isinstance(raw, dict) else {}


def _triggers(doc: dict) -> set:
    """The trigger names, whichever of the three shapes `on:` was written in.

    All of these are legal and all three appear in this repo:
    `on: workflow_dispatch` (scalar), `on: [ push, pull_request ]` (list), and
    `on:` with a mapping. Reading only the mapping form silently treated the
    other two as having no triggers at all, which is how the first draft of
    this check decided upstream's dispatch-only release.yml was not
    dispatch-only.
    """
    raw = doc.get("on", doc.get(True))
    if isinstance(raw, str):
        return {raw}
    if isinstance(raw, list):
        return {t for t in raw if isinstance(t, str)}
    if isinstance(raw, dict):
        return set(raw)
    return set()


def _is_publisher(step: dict) -> bool:
    blob = str(step.get('uses') or '') + ' ' + str(step.get('run') or '')
    return any(p in blob for p in PUBLISHERS)


def main() -> int:
    failures: list[str] = []
    tag_publishers: list[str] = []

    for path in sorted(WORKFLOWS.glob("*.y*ml")):
        doc = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        on = _on(doc)
        push = on.get("push") if isinstance(on.get("push"), dict) else {}
        tags = push.get("tags") if isinstance(push, dict) else None

        # A workflow only a human can start by hand is exempt from the
        # tag-guard rule: upstream's inherited release.yml is workflow_dispatch
        # only, and demanding a ref guard on a deliberate manual run would keep
        # this red forever over a file the fork does not own. It is still
        # counted below, so if an upstream merge ever gives it a tag trigger
        # both rules start applying to it again.
        triggers = _triggers(doc)
        manual_only = bool(triggers) and triggers <= {"workflow_dispatch", "workflow_call"}

        for job_name, job in (doc.get("jobs") or {}).items():
            steps = job.get("steps") or []
            publishers = [s for s in steps if isinstance(s, dict) and _is_publisher(s)]
            if not publishers:
                continue
            if tags:
                tag_publishers.append(f"{path.name}:{job_name} (tags={tags})")

            # Every guard in the job that gates on being a tag ref.
            guards = {
                str(s.get("if"))
                for s in steps
                if isinstance(s, dict) and s.get("if") and "refs/tags/" in str(s.get("if"))
            }
            if manual_only:
                continue

            for step in publishers:
                name = step.get("name") or step.get("uses")
                cond = str(step.get("if") or "")

                if "refs/tags/" not in cond:
                    failures.append(
                        f"{path.name}:{job_name}: the step {name!r} publishes a release "
                        f"but its `if` ({cond or 'absent'}) does not require a tag ref. "
                        f"Anything else that can start this job -- a dispatch input, a "
                        f"branch push -- reaches it with the version guard skipped."
                    )
                    continue

                # An `||` is the whole failure mode. `startsWith(github.ref,
                # 'refs/tags/attune-v') || inputs.publish` mentions the tag and
                # still publishes from any branch on a dispatch. An earlier
                # draft of this check asked whether the guard was a SUBSTRING
                # of the publish condition, which is true exactly when the
                # condition is the guard plus an escape hatch -- it passed the
                # mutation that restored the bug. Any alternative is a way in.
                if "||" in cond:
                    failures.append(
                        f"{path.name}:{job_name}: the step {name!r} publishes a release on "
                        f"({cond}). The `||` is an alternative route that does not require "
                        f"a tag, so the tag/version guard can be skipped on that route. "
                        f"A release comes from a tag or it does not happen."
                    )
                    continue

                for guard in guards:
                    if guard != cond:
                        failures.append(
                            f"{path.name}:{job_name}: the publish condition ({cond}) and "
                            f"the version guard ({guard}) are different expressions, so "
                            f"there is a way to reach the publish with the guard skipped. "
                            f"Make them the same."
                        )

    if len(tag_publishers) > 1:
        failures.append(
            "More than one workflow publishes a release on a tag push, so one tag "
            "starts both and whichever finishes last defines the release: "
            + "; ".join(tag_publishers)
        )

    if failures:
        print("::error::Release workflow invariants violated.")
        for f in failures:
            print(f"  - {f}")
        return 1

    print(
        f"Clean: {len(tag_publishers)} workflow publishes on a tag, "
        "and every publish step is guarded by the same tag condition as its version check."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
