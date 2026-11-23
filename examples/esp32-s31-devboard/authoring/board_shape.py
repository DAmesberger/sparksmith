"""Record the board's outline sketch and its part dimensions into the replay.

The outline is drawn and constrained in the board sketch and driven by the
board parameters in PARAMETERS: a tab that carries the USB-C receptacle
through an enclosure wall, a notch under the module's antenna between two
ears that carry the upper mounting features, three mounting holes and one
mounting slot, and a pass-through window. Every point is drawn where the
parameters put it and then held by a constraint or a dimension, so the
sketch solves with no freedom left; nothing is held by its coordinates.

Every placed part is then held by part dimensions: the receptacle, the
module, the two headers and the fiducials to board edges, every other part
at its offset from the receptacle or the module it serves.

This tool replays the replay through `sparksmith stdio --operator` and
replaces three runs of requests with the ones it makes:

- the `board.edit` requests after `board.start_sketch` (the outline);
- the first run of `layout.dimension_add` requests, kept at their place and
  their number, so the ids of everything recorded after them stay (the
  receptacle and the three parts beside it);
- the `layout.dimension_add` requests after the last `layout.place` (every
  other part).

Each kept request is sent at the revision the session is at. Run it from the
repository root, then `regenerate.sh`:

    python3 -B examples/esp32-s31-devboard/authoring/board_shape.py

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

# Board parameters: name, value in mm or an expression over the others, and
# the comment the Board card shows. `width` and `height` come with the sketch.
PARAMETERS = [
    ("width", 40.0, "Board width"),
    ("height", 64.0, "Board height, from the USB tab's edge to the ears"),
    ("corner_r", 1.5, "Outside corner radius"),
    ("inner_r", 0.5, "Inside corner radius (the router bit's)"),
    ("usb_tab_w", 12.0, "Width of the tab that carries the USB-C receptacle"),
    ("usb_tab_d", 1.0, "How far the tab stands out of the bottom edge (the enclosure wall)"),
    ("tab_x", "(width - usb_tab_w) / 2", "Tab side from the nearer side edge"),
    ("ant_w", 23.2, "Width of the notch under the module's antenna"),
    ("ant_d", 5.5, "Depth of the antenna notch"),
    ("ear_w", "(width - ant_w) / 2", "Width of each ear beside the antenna notch"),
    ("inset", 3.5, "Mounting hole inset from the edges"),
    ("hole_d", 2.7, "Mounting hole and slot diameter (M2.5 clearance)"),
    ("slot_play", 1.0, "Travel of the mounting slot, between its cap centres"),
    ("slot_edge", "inset - slot_play / 2", "Mounting slot's outer cap centre from the right edge"),
    ("win_w", 8.0, "Pass-through window width"),
    ("win_h", 10.0, "Pass-through window height"),
    ("win_off", 7.0, "Pass-through window centre, left of the board's centre line"),
    ("win_x", "width / 2 - win_off - win_w / 2", "Pass-through window from the left edge"),
    ("win_y", 19.0, "Pass-through window from the bottom edge"),
    ("header_span", 34.92, "Spacing of the two header rows"),
    ("header_y", 21.0, "Header centres from the bottom edge"),
    ("fid_off", 14.0, "Fiducials from the board's centre line"),
]

# The front face of the USB4105 part file's Shell body (parts/usb4105.solidcraft)
# and the antenna end of the module's own board (the module example's part
# file): each by body, persistent Solidcraft face name and a point on it in
# the model frame.
USB_FRONT = ("Shell", "F2:start", [0.0, -3.675, 0.2])
ANTENNA_END = ("board", "F1:+y", [0.0, 15.0, 0.5])

# Parts held at their offset from the part they serve.
BESIDE = {
    "J1": ("U3", "R4", "R5", "U2", "C1", "C2", "R2", "D1"),
    "U1": ("C3", "R1", "C4", "R3", "D2", "SW1", "SW2", "J4"),
}
# The first run of part dimensions keeps its place in the replay: the
# receptacle's two and these parts' offsets from it.
EARLY = ("U3", "R4", "R5")


def values():
    """Each parameter's value in mm, expressions evaluated in order."""
    out = {}
    for name, value, _ in PARAMETERS:
        out[name] = float(eval(value, {"__builtins__": {}}, out)) if isinstance(value, str) else value
    return out


def expr(value):
    return value if isinstance(value, str) else f"{value:g} mm"


def sketch(edit):
    return {"op": "sketch", "edit": edit}


def constraint(kind, *refs):
    return sketch({"add_constraint": {"kind": kind, "refs": list(refs)}})


def dimension(kind, param, *refs):
    return sketch({"add_dimension": {"kind": kind, "refs": list(refs), "value": {"param": param}}})


def point(ref):
    return {"point": ref}


def curve(ref):
    return {"curve": ref}


class Recorder:
    """A live session that records every request it is sent."""

    def __init__(self):
        base = tempfile.mkdtemp(prefix="sparksmith-shape-")
        env = dict(os.environ, SPARKSMITH_CONFIG_DIR=base + "/config", SPARKSMITH_CACHE_DIR=base + "/cache")
        self.proc = subprocess.Popen([CLI, "stdio", "--operator"], stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, text=True, bufsize=1, env=env)
        self.out = []
        self.revision = 0

    def ask(self, request):
        self.proc.stdin.write(json.dumps(request, separators=(",", ":"), ensure_ascii=False) + "\n")
        self.proc.stdin.flush()
        reply = json.loads(self.proc.stdout.readline())
        if not reply.get("ok"):
            sys.exit(f"refused: {json.dumps(request)[:600]}\n{json.dumps(reply, ensure_ascii=False)[:3000]}")
        result = reply["result"]
        if isinstance(result, dict) and "revision" in result:
            self.revision = result["revision"]
        return result

    def send(self, request):
        """Send a recorded request at the session's revision, and record it."""
        if "expected_revision" in request.get("params", {}):
            request["params"]["expected_revision"] = self.revision
        self.out.append(json.dumps(request, separators=(",", ":"), ensure_ascii=False))
        return self.ask(request)

    def apply(self, editor, command, params):
        return self.send({"command": "project.apply", "params": {"expected_revision": self.revision, "operation": {
            "editor": editor, "operation": {"command": command, "params": params}}}})

    def edit(self, edits):
        """One `board.edit`; the sketch entities it created, by kind, in order."""
        created = self.apply("Board", "board.edit", {"edits": edits})["outcome"]["created"]
        return ([c["point"] for c in created if "point" in c], [c["curve"] for c in created if "curve" in c])

    def project(self):
        return self.ask({"command": "project.inspect", "params": {}, "verbose": True})["project"]

    def close(self):
        self.proc.stdin.close()
        self.proc.wait()


def outline(s):
    """Draw and constrain the outline; returns the named edges' curves."""
    v = values()
    w, h, d = v["width"], v["height"], v["usb_tab_d"]
    xl, xr, ear, floor = v["tab_x"], w - v["tab_x"], v["ear_w"], h - v["ant_d"]
    board = s.project()["board_shape"]["sketch"]
    # The rectangle `board.start_sketch` drew goes, with its corner points
    # and the dimensions on them.
    gone = ([curve(c["id"]) for c in board["sketch"]["curves"]]
            + [point(p["id"]) for p in board["sketch"]["points"] if p["id"] != "origin"])
    # Counter-clockwise: the left shoulder a-b, the tab b-c-d-e, the right
    # shoulder e-f, the right edge f-g, the right ear g-h-i, the antenna
    # notch's floor i-j, the left ear j-k-m and the left edge m-a.
    #
    #   m-----k           h-----g
    #   |     |           |     |
    #   |     j-----------i     |
    #   |                       |
    #   a-----b           e-----f
    #         c-----------d
    corners = [[0.0, d], [xl, d], [xl, 0.0], [xr, 0.0], [xr, d], [w, d],
               [w, h], [w - ear, h], [w - ear, floor], [ear, floor], [ear, h], [0.0, h]]
    points, lines = s.edit(
        [{"op": "unname_edge", "name": e["name"]} for e in board["edges"]]
        + [sketch({"delete": {"ids": gone}}),
           sketch({"add_polyline": {"points": corners, "closed": True, "construction": False}})])
    a, b, c, _, e, f, g, h_, i, j, k, m = points
    ab, bc, cd, _, _, fg, _, _, _, jk, km, ma = lines
    edits = [{"op": "set_parameter", "param": {"name": n, "expr": expr(value), "unit": "mm", "comment": comment}}
             for n, value, comment in PARAMETERS]
    edits += [constraint("horizontal" if n % 2 == 0 else "vertical", curve(line)) for n, line in enumerate(lines)]
    edits += [
        # The left edge stands on the origin's x, the tab's edge on its y.
        constraint("vertical", point("origin"), point(a)),
        constraint("horizontal", point("origin"), point(c)),
        dimension("horizontal_distance", "width", point(a), point(f)),
        dimension("vertical_distance", "height", point(c), point(k)),
        # The tab: centred, both shoulders on one line.
        dimension("vertical_distance", "usb_tab_d", curve(bc)),
        constraint("point_on_curve", point(e), curve(ab)),
        dimension("distance", "tab_x", point(b), curve(ma)),
        dimension("distance", "tab_x", point(e), curve(fg)),
        # The antenna notch: centred between two ears whose tops are one line.
        constraint("point_on_curve", point(g), curve(km)),
        dimension("distance", "ear_w", point(k), curve(ma)),
        dimension("distance", "ear_w", point(h_), curve(fg)),
        dimension("vertical_distance", "ant_d", curve(jk)),
    ]
    edges = {"bottom": cd, "right": fg, "top": km, "left": ma}
    edits += [{"op": "name_edge", "name": name, "curve": line} for name, line in edges.items()]
    s.edit(edits)
    # Outside corners take `corner_r`, inside corners the router bit's
    # radius; the tab's own two corners stay square.
    s.edit([{"op": "fillet", "corners": [a, f, g, h_, k, m], "radius": {"param": "corner_r"}},
            {"op": "fillet", "corners": [b, e, i, j], "radius": {"param": "inner_r"}}])
    return edges


def mounting(s, edges):
    """Three round holes and one slot, each `inset` from its two edges."""
    v = values()
    w, h, inset = v["width"], v["height"], v["inset"]
    at = [(inset, inset, "left", "bottom"), (w - inset, inset, "right", "bottom"), (inset, h - inset, "left", "top")]
    points, _ = s.edit([sketch({"add_point": {"at": [x, y]}}) for x, y, _, _ in at])
    edits = []
    for p, (_, _, side, end) in zip(points, at):
        edits += [dimension("distance", "inset", point(p), curve(edges[side])),
                  dimension("distance", "inset", point(p), curve(edges[end])),
                  {"op": "add_mounting_hole", "center": p, "drill": "hole_d", "plated": False}]
    s.edit(edits)
    # The fourth fastener sits in a slot along the width, so the enclosure's
    # boss spacing may differ from the board's by `slot_play`.
    y = h - inset
    outer, inner = w - v["slot_edge"], w - v["slot_edge"] - v["slot_play"]
    points, _ = s.edit([{"op": "add_slot", "a": [inner, y], "b": [outer, y],
                         "width": {"param": "hole_d"}, "plated": False}])
    ca, cb = points[0], points[1]
    s.edit([dimension("distance", "slot_edge", point(cb), curve(edges["right"])),
            dimension("distance", "inset", point(cb), curve(edges["top"])),
            constraint("horizontal", point(ca), point(cb)),
            dimension("distance", "slot_play", point(ca), point(cb))])


def window(s, edges):
    """The pass-through window: a rounded rectangle held to the left and
    bottom edges."""
    v = values()
    x, y = v["win_x"], v["win_y"]
    points, lines = s.edit([sketch({"add_rect": {"a": [x, y], "b": [x + v["win_w"], y + v["win_h"]],
                                                 "construction": False}})])
    s.edit([dimension("distance", "win_x", point(points[0]), curve(edges["left"])),
            dimension("distance", "win_y", point(points[0]), curve(edges["bottom"])),
            dimension("horizontal_distance", "win_w", curve(lines[0])),
            dimension("vertical_distance", "win_h", curve(lines[1])),
            {"op": "fillet", "corners": points, "radius": {"param": "inner_r"}}])


def origin(instance):
    return {"part": {"feature": "origin", "target": {"kind": "placement", "id": instance}}}


def face(instance, named):
    body, name, pick = named
    return {"part": {"feature": "face", "placement": instance, "face": {"body": body, "face": name, "pick": pick}}}


def hold(s, kind, refs, value):
    s.apply("Dimension", "layout.dimension_add", {"dimension": {"kind": {"dim": kind}, "refs": refs, "value": value}})


def state(s):
    """Instance ids and positions by designator, and the named edges."""
    project = s.project()
    ids = {i["designator"]: i["id"] for i in project["instances"]}
    by_id = {i: d for d, i in ids.items()}
    at = {by_id[p["instance"]]: p["position"] for p in project["placements"]}
    edges = {e["name"]: {"board": {"curve": e["curve"]}} for e in project["board_shape"]["sketch"]["edges"]}
    return ids, at, edges


def beside(s, ids, at, anchor, part):
    """`part` at its offset from `anchor`, along x and along y."""
    for kind, axis in (("horizontal_distance", 0), ("vertical_distance", 1)):
        hold(s, kind, [origin(ids[anchor]), origin(ids[part])],
             {"fixed": round(abs(at[part][axis] - at[anchor][axis]), 6)})


def receptacle(s):
    """J1 by its 3D model: the shell's front face on the tab's edge and its
    origin on the board's centre line; the parts beside it follow it."""
    ids, at, edges = state(s)
    hold(s, "distance", [face(ids["J1"], USB_FRONT), edges["bottom"]], {"fixed": 0.0})
    hold(s, "distance", [origin(ids["J1"]), edges["left"]], {"expr": "width / 2"})
    for part in EARLY:
        beside(s, ids, at, "J1", part)


def parts(s):
    """Every other part: the module by its antenna end on the top edge and
    its origin on the centre line, the headers and fiducials from the
    edges, the rest beside the receptacle or the module."""
    ids, at, edges = state(s)
    hold(s, "distance", [face(ids["U1"], ANTENNA_END), edges["top"]], {"fixed": 0.0})
    hold(s, "distance", [origin(ids["U1"]), edges["left"]], {"expr": "width / 2"})
    for anchor, served in BESIDE.items():
        for part in served:
            if part not in EARLY:
                beside(s, ids, at, anchor, part)
    # Along x every part is measured from the left edge, which stands on
    # the sketch's origin and so never passes a part when the width
    # changes; a distance does not say which side of its edge it is on.
    for header, sign in (("J2", "-"), ("J3", "+")):
        hold(s, "distance", [origin(ids[header]), edges["left"]], {"expr": f"(width {sign} header_span) / 2"})
        hold(s, "distance", [origin(ids[header]), edges["bottom"]], {"expr": "header_y"})
    v = values()
    fiducials = sorted((d for d in ids if d.startswith("FID")), key=lambda d: int(d[3:]))
    for fiducial in fiducials:
        x, y = at[fiducial]
        sign = "-" if x < v["width"] / 2 else "+"
        end = "bottom" if y < v["height"] / 2 + 1e-9 else "top"
        hold(s, "distance", [origin(ids[fiducial]), edges["left"]], {"expr": f"width / 2 {sign} fid_off"})
        hold(s, "distance", [origin(ids[fiducial]), edges[end]],
             {"fixed": round(y if end == "bottom" else v["height"] - y, 6)})


def kind(request):
    """What a recorded request is, for finding the runs this tool makes."""
    if request["command"] != "project.apply":
        return request["command"]
    return request["params"]["operation"]["operation"]["command"]


def main():
    with open(REPLAY, encoding="utf-8") as f:
        requests = [json.loads(line) for line in f.read().splitlines()]
    kinds = [kind(r) for r in requests]
    start = kinds.index("board.start_sketch") + 1
    end = start
    while kinds[end] in ("board.edit", "project.inspect"):
        end += 1
    first = kinds.index("layout.dimension_add")
    first_end = first
    while kinds[first_end] == "layout.dimension_add":
        first_end += 1
    if first_end - first != 2 + 2 * len(EARLY):
        sys.exit(f"the first run of part dimensions holds {first_end - first} requests; this tool records "
                 f"{2 + 2 * len(EARLY)} there, and another number would move every id recorded after them")
    last_place = max(i for i, k in enumerate(kinds) if k == "layout.place")
    s = Recorder()
    for i, request in enumerate(requests):
        if start <= i < end or first < i < first_end or (i > last_place and kinds[i] == "layout.dimension_add"):
            continue
        if i == start - 1:
            s.send(request)
            edges = outline(s)
            mounting(s, edges)
            window(s, edges)
        elif i == first:
            receptacle(s)
        elif kinds[i] == "project.rename" and i > last_place:
            parts(s)
            s.send(request)
        elif kinds[i] == "project.save":
            s.out.append(json.dumps(request, separators=(",", ":"), ensure_ascii=False))
        else:
            s.send(request)
    s.close()
    target = os.environ.get("SHAPE_OUT", REPLAY)
    with open(target, "w", encoding="utf-8") as f:
        f.write("\n".join(s.out) + "\n")
    print(f"{len(s.out)} requests recorded in {target}")


if __name__ == "__main__":
    main()
