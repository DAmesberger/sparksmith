"""Stage 1: the parts library of the ESP32-S31 dev board, in the project."""
import json

DS = {
    "tlv757p": ("TLV757P 1-A Low-Dropout Regulator datasheet", "SBVS320", "https://www.ti.com/lit/ds/symlink/tlv757p.pdf"),
    "tpd2e009": ("TPD2E009 2-Channel ESD Protection for High-Speed Interfaces", "SLLSE91", "https://www.ti.com/lit/ds/symlink/tpd2e009.pdf"),
    "usb4105": ("GCT USB4105 USB Type-C receptacle for USB 2.0, drawing", "B4 (2023-12-18)", "https://gct.co/files/drawings/usb4105.pdf"),
    "b3u": ("Omron B3U ultra-small surface-mounting tactile switch", "catalog", "https://omronfs.omron.com/en_US/ecb/products/pdf/en-b3u.pdf"),
    "apt1608": ("Kingbright APT1608SGC 1.6 x 0.8 mm SMD chip LED", "V.22B (2023-12-07)", "https://www.kingbrightusa.com/images/catalog/SPEC/APT1608SGC.pdf"),
}


def meta(name, provenance, description=""):
    return {"name": name, "description": description, "provenance": provenance}


def drafts(s):
    """{id: draft} of the session's drafts."""
    r = s.raw("project.inspect")["result"]
    return {d["draft"]["id"] if "draft" in d else d["id"]: d["draft"] for d in r["drafts"]}


def edit(s, items, note):
    s.op("library.edit", {"edits": items}, note)


def pin(row, num, name, direction, variant, signal="power", cls="electrical", page=None, table=None):
    r = {"row": row, "number": {variant: num}, "name": name, "class": cls,
         "source": {k: v for k, v in (("page", page), ("table", table)) if v is not None}}
    if cls == "electrical":
        label = name.lower().replace("+", "p").replace("-", "n")
        r["functions"] = [{"label": label, "direction": direction, "signal": signal}]
        r["default"] = label
    return r


def pinout(s, name, ds, variant, count, pins, onto=None):
    title, rev, url = DS[ds]
    doc = {"format": 1, "device": {"name": name, "reference": {"title": title, "document_revision": rev, "url": url}},
           "variants": [{"name": variant, "terminal_count": count}], "peripherals": [], "pins": pins, "rules": []}
    target = {"target": "draft", "id": onto} if onto else {"target": "new", "name": name}
    base = s.query("library.pinout_preview", {"target": target, "document": doc})["result"]["base"]
    r = s.op("library.pinout_apply", {"target": target, "document": doc, "base": base}, f"pinout {name}")
    return r["result"]["outcome"]["report"]["drafts"]  # component, symbol, package, binding


KINDS = ["component", "symbol", "package", "footprint", "binding"]


def generate(s, generator, preset, name, params=None, scope="component"):
    """A part made by a shipped generator (`library.generate_part`): its root
    revision and each lineage of the part by kind."""
    if s.rev is None:
        s.rev = s.revision()
    p = {"expected_revision": s.rev, "generator": generator, "scope": scope, "name": name,
         "params": params or {}}
    if preset:
        p["preset"] = preset
    r = s.raw("library.generate_part", p, f"generate {name}")
    s.rev = r["result"].get("revision", s.rev)
    report = r["result"]["outcome"]["report"]
    return report["root"], {x["kind"]: x["id"] for x in report["revisions"]}


def draft_closure(s, root):
    """Draft a generated component's working set to make it this board's own."""
    s.op("library.draft_closure", {"component": {"id": root["id"], "revision": root["revision"]}},
         f"draft {root['id']}")


def footprint_from_pads(s, package, pads, slots=(), holes=()):
    """A new footprint draft landing `package`'s draft, built from a pad table."""
    body = {"package": {"id": package, "revision": 1}, "next_local": 1, "source": None, "pads": [], "graphics": [],
            "polarity": None, "datum": {"offset": {"x_nm": 0, "y_nm": 0}, "rotation_mdeg": 0},
            "defaults": {"mask_expansion_nm": 50000, "paste_reduction_nm": 0}}
    r = s.op("library.new_draft", {"meta": meta(drafts(s)[package]["meta"]["name"] + " footprint",
                                               drafts(s)[package]["meta"]["provenance"]),
                                   "definition": {"kind": "footprint", "body": body}}, "footprint draft")
    fp = r["result"]["outcome"]["id"]
    s.op("library.footprint_from_pads", {"draft": fp, "pads": list(pads), "slots": list(slots),
                                         "holes": list(holes), "assign_by_number": True}, "pad table")
    return fp


def check(s, ids=None):
    c = s.query("component.check_drafts", {})
    bad = []
    for d in c["result"]:
        if ids and d["target"]["id"] not in ids:
            continue
        for f in d["findings"]:
            if f["blocks"]:
                bad.append((d["target"]["id"], d["kind"], f["code"], f["message"][:200]))
    return bad




def nm(mm):
    """A length in millimetres as integer nanometres."""
    return int(round(mm * 1_000_000))


def pt(xy):
    """An `[x, y]` point in millimetres as a stored point."""
    return {"x_nm": nm(xy[0]), "y_nm": nm(xy[1])}


def rect(a, b, width, layer):
    return {"item": {"shape": {"fill": "none", "shape": {"rect": {"a": pt(a), "b": pt(b)}}, "width_nm": nm(width)}},
            "layer": layer, "side": "component"}






def generic_package(s, pkg_id, envelope, prov, material="epoxy_mould"):
    d = drafts(s)[pkg_id]
    body = d["definition"]["body"]
    body["envelope_nm"] = [nm(v) for v in envelope]
    # The switch is no chip: no chip solder joints, and its leads are the
    # part file's.
    body["leads"] = []
    body["joints"] = None
    body["model"] = {"source": "envelope", "material": material}
    edit(s, [{"id": pkg_id, "meta": d["meta"] | {"provenance": prov}, "definition": d["definition"]}], f"generic package {pkg_id}")

