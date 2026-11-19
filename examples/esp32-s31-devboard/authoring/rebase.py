"""Carry the replay's package snapshot edits onto the current generators.

The parts stage renames two generated packages (the SOT-23-5 and SOT-23-3
of the regulator and the ESD part: build_lib.py, `own(..., package=)`) by
reading the draft and sending its whole body back, since `library.edit`
replaces a draft's meta and definition. The replay records that body
literally, part-file values and the facts the part file evaluated to
included. After the generator or its part file changes, replaying puts the
old values back over the new generation, and the board build refuses the
package (`part_file.stale`: the part file no longer builds what the package
recorded).

This tool replays the replay through `sparksmith stdio --operator` and, at
each such edit, reads the package draft as the current generator made it
and sends it with only the authored name; the rewritten requests replace
the replay. The components' field edits are left as recorded: they are the
authored component bodies (this board's typed fields), not the generator's.
Run it from the repository root after a generator change, then
`regenerate.sh`:

    python3 -B examples/esp32-s31-devboard/authoring/rebase.py

`cargo test -p sparksmith-automation --test example_replay` fails until the
committed example is what the replay builds.
"""
import json
import os
import subprocess
import sys
import tempfile

REPLAY = "examples/esp32-s31-devboard/esp32-s31-devboard.replay.jsonl"
CLI = os.environ.get("SPARKSMITH_CLI", "target/nix/ci/sparksmith")
GENERATED = "Made by the generator"


def snapshot(edit):
    """Whether `edit` (a `library.edit` item) sends a generated package back."""
    return edit["definition"].get("kind") == "package" and GENERATED in edit["meta"].get("provenance", "")


def main():
    base = tempfile.mkdtemp(prefix="sparksmith-rebase-")
    env = dict(os.environ, SPARKSMITH_CONFIG_DIR=base + "/config", SPARKSMITH_CACHE_DIR=base + "/cache")
    proc = subprocess.Popen([CLI, "stdio", "--operator"], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, text=True, bufsize=1, env=env)

    def send(request):
        proc.stdin.write(json.dumps(request, separators=(",", ":"), ensure_ascii=False) + "\n")
        proc.stdin.flush()
        reply = json.loads(proc.stdout.readline())
        if not reply.get("ok"):
            sys.exit(f"refused: {json.dumps(request)[:300]}\n{json.dumps(reply)[:2000]}")
        return reply

    out, report = [], []
    with open(REPLAY, encoding="utf-8") as f:
        lines = f.read().splitlines()
    for n, line in enumerate(lines, 1):
        request = json.loads(line)
        operation = request.get("params", {}).get("operation", {}).get("operation", {})
        edits = operation.get("params", {}).get("edits", []) if operation.get("command") == "library.edit" else []
        if any(snapshot(e) for e in edits):
            inspect = send({"command": "project.inspect", "params": {}, "verbose": True})["result"]
            drafts = {d["draft"]["id"] if "draft" in d else d["id"]: d["draft"] for d in inspect["drafts"]}
            for k, e in enumerate(edits):
                if snapshot(e):
                    draft = drafts[e["id"]]
                    rebuilt = {"id": e["id"], "meta": dict(draft["meta"], name=e["meta"]["name"]),
                               "definition": draft["definition"]}
                    if rebuilt != e:
                        report.append(f"line {n}: package {e['id']} ({e['meta']['name']}) carried onto the current generator")
                    edits[k] = rebuilt
            line = json.dumps(request, separators=(",", ":"), ensure_ascii=False)
        out.append(line)
        if request["command"] != "project.save":
            send(request)
    proc.stdin.close()
    proc.wait()
    with open(REPLAY, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    print("\n".join(report) if report else "nothing to carry")


if __name__ == "__main__":
    main()
