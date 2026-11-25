"""Carry the replay's snapshot edits of generated parts onto the current generators.

The parts stage edits generated drafts by reading a draft and sending its
whole body back, since `library.edit` replaces a draft's meta and
definition: it renames two generated packages (the SOT-23-5 and SOT-23-3 of
the regulator and the ESD part: build_lib.py, `own(..., package=)`), types
the chip resistor's and capacitors' components with this board's fields
(`set_fields`) and gives the B3U-1000P push button its `SW` prefix and the USB4105 its `J`; its
pinout plans are applied with the base token their preview gave. The
replay records each body literally, as the generator made it then. After the
generator or its part file changes, replaying puts the old generation back
over the new one: a package's old part-file values make the board build
refuse it (`part_file.stale`), and a component loses what the generator now
adds (its marking and value template).

This tool replays the replay through `sparksmith stdio --operator` and, at
each such edit, reads the draft as the current generator made it and sends
it with only the authored changes:

- a pinout plan (`library.pinout_apply`): the base token of the preview
  just before it, since the generated draft it targets records its
  generator's bundle;
- a package: its name;
- a component: its name, its designator prefix, and this board's fields in
  place of the generator's fields of the same name (each with the recorded
  "show on symbol"), the generator's marking and value template naming the
  board's field instead. The generator's other fields stay. A component
  recorded with no fields (the push button, which is not a resistor) keeps
  none of the generator's fields, nor its marking or value template.

The rewritten requests replace the replay. Run it from the repository root
after a generator change, then `regenerate.sh`:

    python3 -B examples/esp32-s31-devboard/authoring/rebase.py

`cargo test -p sparksmith-automation --test example_replay` fails until the
committed example is what the replay builds.
"""
import json
import os
import re
import subprocess
import sys
import tempfile

REPLAY = "examples/esp32-s31-devboard/esp32-s31-devboard.replay.jsonl"
CLI = os.environ.get("SPARKSMITH_CLI", "target/nix/ci/sparksmith")
GENERATED = "Made by the generator"


def snapshot(edit):
    """Whether `edit` (a `library.edit` item) sends a generated package or component back."""
    return (edit["definition"].get("kind") in ("package", "component")
            and GENERATED in edit["meta"].get("provenance", ""))


def field_names(inspect):
    """Each field lineage's name, published or drafted."""
    names = {f["id"]: f["meta"]["name"] for f in inspect["project"]["library"]["fields"]}
    for d in inspect["drafts"]:
        draft = d.get("draft", d)
        if draft["definition"].get("kind") == "field":
            names[draft["id"]] = draft["meta"]["name"]
    return names


def renamed(value, swap):
    """`value` (a marking or value template) naming `swap[old]` for each old field id."""
    if isinstance(value, dict):
        return {k: (swap.get(v, v) if k == "field" and isinstance(v, int) else renamed(v, swap))
                for k, v in value.items()}
    if isinstance(value, list):
        return [renamed(v, swap) for v in value]
    if isinstance(value, str):
        return re.sub(r"field:D(\d+)", lambda m: f"field:D{swap.get(int(m.group(1)), int(m.group(1)))}", value)
    return value


def component(recorded, current, names):
    """The current generation of a component with `recorded`'s authored changes."""
    body = json.loads(json.dumps(current["body"]))
    authored = recorded["body"]
    body["designator_prefix"] = authored["designator_prefix"]
    if not authored["fields"]:
        # A part redrawn as something else (the push button made from the
        # 1206 resistor) carries none of the generator's fields, so neither
        # its marking nor its value template, which can only name them.
        body["fields"] = []
        body.pop("marking", None)
        body.pop("value_template", None)
        return {"kind": "component", "body": body}
    generated = {names.get(f["field"]["id"]): i for i, f in enumerate(body["fields"])}
    swap = {}
    for f in authored["fields"]:
        i = generated.get(names.get(f["field"]["id"]))
        if i is None:
            sys.exit(f"the generator has no field named {names.get(f['field']['id'])!r} to carry {f} onto")
        swap[body["fields"][i]["field"]["id"]] = f["field"]["id"]
        body["fields"][i] = f
    for key in ("marking", "value_template"):
        if key in body:
            body[key] = renamed(body[key], swap)
    return {"kind": "component", "body": body}


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
    preview_base = None
    with open(REPLAY, encoding="utf-8") as f:
        lines = f.read().splitlines()
    for n, line in enumerate(lines, 1):
        request = json.loads(line)
        operation = request.get("params", {}).get("operation", {}).get("operation", {})
        edits = operation.get("params", {}).get("edits", []) if operation.get("command") == "library.edit" else []
        if any(snapshot(e) for e in edits):
            inspect = send({"command": "project.inspect", "params": {}, "verbose": True})["result"]
            drafts = {d["draft"]["id"] if "draft" in d else d["id"]: d["draft"] for d in inspect["drafts"]}
            names = field_names(inspect)
            for k, e in enumerate(edits):
                if snapshot(e):
                    draft = drafts[e["id"]]
                    kind = e["definition"]["kind"]
                    definition = (component(e["definition"], draft["definition"], names)
                                  if kind == "component" else draft["definition"])
                    rebuilt = {"id": e["id"], "meta": dict(draft["meta"], name=e["meta"]["name"]),
                               "definition": definition}
                    if rebuilt != e:
                        report.append(f"line {n}: {kind} {e['id']} ({e['meta']['name']}) carried onto the current generator")
                    edits[k] = rebuilt
            line = json.dumps(request, separators=(",", ":"), ensure_ascii=False)
        out.append(line)
        if operation.get("command") == "library.pinout_apply" and preview_base is not None:
            params = operation["params"]
            if params.get("base") != preview_base:
                report.append(f"line {n}: pinout plan applied at its preview's base {preview_base}")
                params["base"] = preview_base
                line = json.dumps(request, separators=(",", ":"), ensure_ascii=False)
                out[-1] = line
        if request["command"] != "project.save":
            reply = send(request)
            if request.get("params", {}).get("query") == "library.pinout_preview":
                preview_base = reply["result"]["result"].get("base")
    proc.stdin.close()
    proc.wait()
    with open(REPLAY, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    print("\n".join(report) if report else "nothing to carry")


if __name__ == "__main__":
    main()
