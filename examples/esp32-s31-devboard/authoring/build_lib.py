"""Build the dev board's parts in the session (stage 1)."""
from decimal import Decimal

from stage_lib import *

B3U_PART = "examples/esp32-s31-devboard/parts/b3u-1000p.solidcraft"  # from the repository root
USB4105_PART = "examples/esp32-s31-devboard/parts/usb4105.solidcraft"
PROV_NOTE ="Authored for the ESP32-S31 dev board e2e run from the manufacturer's public datasheet; checked by the author, not by a second reviewer."


def quantity(s, name, dim, symbol, aliases):
    body = {"units": {"dimension": dim, "units": [{"symbol": symbol, "aliases": aliases, "scale": 1.0, "si_prefixes": True}]},
            "default_unit": symbol}
    r = s.op("library.new_draft", {"meta": meta(name, "SI quantity"), "definition": {"kind": "quantity_kind", "body": body}})
    return r["result"]["outcome"]["id"]


def field(s, name, kind):
    body = {"next_local": 1, "value_type": {"type": "quantity", "kind": {"id": kind, "revision": 1}, "min": 0.0, "max": None},
            "default": None, "required_at": None}
    r = s.op("library.new_draft", {"meta": meta(name, "Project field"), "definition": {"kind": "field", "body": body}})
    return r["result"]["outcome"]["id"]


def set_fields(s, comp, fields, name=None, prov=None):
    d = drafts(s)[comp]
    d["definition"]["body"]["fields"] = [{"field": {"id": f, "revision": 1}, "default": None, "show_on_symbol": show}
                                         for f, show in fields]
    m = d["meta"]
    if name: m["name"] = name
    if prov: m["provenance"] = prov
    edit(s, [{"id": comp, "meta": m, "definition": d["definition"]}], f"fields of {comp}")


def part(s, mpn, manufacturer, comp, binding, specs, ds_title, ds_url, ds_rev=None, desc="", rev=1):
    body = {"next_local": 2, "manufacturer": manufacturer, "mpn": mpn,
            "component": {"id": comp, "revision": rev}, "binding": {"id": binding, "revision": rev},
            "specs": [{"field": {"id": f, "revision": 1}, "value": {"type": "quantity", "value": {"si": format(Decimal(str(v)).normalize(), "f"), "display_unit": None}},
                       "meaning": m} for f, v, m in specs],
            "references": [{"id": 1, "kind": "datasheet", "title": ds_title, "url": ds_url, "document_revision": ds_rev,
                            "asset": None, "licence": None,
                            "provenance": {"origin": None, "basis": {"basis": "authored"}, "licence": None}}],
            "offers": [], "provenance": {"origin": None, "basis": {"basis": "authored"}, "licence": None}}
    r = s.op("library.new_draft", {"meta": meta(mpn, PROV_NOTE, desc), "definition": {"kind": "part", "body": body}}, f"part {mpn}")
    return r["result"]["outcome"]["id"]




def replace_pads(s, fp, pads, prov=None):
    """Replace a generated footprint's pads (keeping ids and terminals in order)."""
    d = drafts(s)[fp]
    body = d["definition"]["body"]
    for p, (at, size) in zip(body["pads"], pads):
        p["position"] = pt(at)
        p["shape"] = {"rect": {"size": pt(size)}}
    if prov:
        d["meta"]["provenance"] = prov
    edit(s, [{"id": fp, "meta": d["meta"], "definition": d["definition"]}], f"pads of {fp}")


def set_graphics(s, fp, keep_text=True, extra=(), silk_text_at=None):
    d = drafts(s)[fp]
    body = d["definition"]["body"]
    n = body["next_local"]
    gfx = [g for g in body["graphics"] if keep_text and "text" in g["item"]]
    if silk_text_at is not None:
        for g in gfx:
            if g["layer"] == "silkscreen":
                g["item"]["text"]["at"] = pt(silk_text_at)
    for g in extra:
        gfx.append(dict(g, id=n)); n += 1
    body["graphics"] = gfx
    body["next_local"] = n
    edit(s, [{"id": fp, "meta": d["meta"], "definition": d["definition"]}], f"graphics of {fp}")


def fit_pin_numbers(s, comp, sym, numbers):
    """Give each side of a pinout symbol stubs long enough for its longest
    pin number at full height (`numbers` in the component's terminal order).
    The need and the module rounding are library::graphics::pin_number_need
    and the template generator's: 0.9 cap heights a character plus 0.4 mm
    clearance at each end, rounded up to the 2.5 mm module. The body stays;
    the pin ends move out."""
    module, advance, clearance = 2_500_000, 0.9, 400_000
    all_drafts = drafts(s)
    terminals = all_drafts[comp]["definition"]["body"]["terminals"]
    number = {t["id"]: n for t, n in zip(terminals, numbers)}
    d = all_drafts[sym]
    body = d["definition"]["body"]
    height = body["pin_text_height_nm"]
    pins = [p for u in body["units"] for p in u["pins"]]
    longest = {}
    for p in pins:
        need = len(number[p["terminal"]]) * advance * height + 2 * clearance
        stub = max(p["length_nm"], -(-int(need) // module) * module)
        longest[p["direction"]] = max(longest.get(p["direction"], 0), stub)
    for p in pins:
        by = longest[p["direction"]] - p["length_nm"]
        dx, dy = {"right": (-by, 0), "left": (by, 0), "up": (0, -by), "down": (0, by)}[p["direction"]]
        p["position"] = {"x_nm": p["position"]["x_nm"] + dx, "y_nm": p["position"]["y_nm"] + dy}
        p["length_nm"] = longest[p["direction"]]
    edit(s, [{"id": sym, "meta": d["meta"], "definition": d["definition"]}], f"pin stubs of {sym}")


def publish_and_close(s, ids, note):
    """Publish; library.publish closes the drafts it published by default,
    so they do not count against the project's 64-draft limit."""
    s.op("library.publish", {"drafts": ids}, note)


def drafts_name(s, i):
    return drafts(s)[i]["meta"]["name"]


def build(s):
    L = {}
    # The module: the datasheet assistant's reviewed drafts, published here.
    publish_and_close(s, [1, 2, 3, 4, 5, 6], "publish ESP32-S31-WROOM-3")
    L["module"] = {"component": 1, "symbol": 2, "binding": 4, "footprint": 5}

    # Typed fields.
    res_k = quantity(s, "Resistance", [2, 1, -3, -2, 0, 0, 0], "Ω", ["ohm", "R"])
    cap_k = quantity(s, "Capacitance", [-2, -1, 4, 2, 0, 0, 0], "F", [])
    vol_k = quantity(s, "Voltage", [2, 1, -3, -1, 0, 0, 0], "V", [])
    publish_and_close(s, [res_k, cap_k, vol_k], "publish quantity kinds")
    F = {"res": field(s, "Resistance", res_k), "cap": field(s, "Capacitance", cap_k), "volt": field(s, "Rated voltage", vol_k)}
    publish_and_close(s, list(F.values()), "publish fields")
    L["fields"] = F

    publish = []

    def own(generator, preset, name, params=None, package=None):
        """Generate a part with a shipped generator and draft its working set
        to make it this board's own (`library.draft_closure`): each lineage's
        next revision is the board's. `package` names the package (the
        purchasing files print it)."""
        root, ids = generate(s, generator, preset, name, params)
        draft_closure(s, root)
        if package:
            d = drafts(s)[ids["package"]]
            d["meta"]["name"] = package
            edit(s, [{"id": ids["package"], "meta": d["meta"], "definition": d["definition"]}], f"name {package}")
        entry = {k: ids[k] for k in KINDS} | {"revision": root["revision"] + 1}
        return entry, [ids[k] for k in KINDS]

    # Generic 0603 resistor, generic 0603 and 0805 MLCCs: the chip
    # generator's parts (with their 3D part files), typed with this board's
    # fields.
    for key, preset, name, flds in (("r0603", "resistor-0603", "Resistor 0603", [(F["res"], True)]),
                                    ("c0603", "capacitor-0603", "Capacitor 0603", [(F["cap"], True), (F["volt"], False)]),
                                    ("c0805", "capacitor-0805", "Capacitor 0805", [(F["cap"], True), (F["volt"], False)])):
        L[key], ids = own("chip", preset, name)
        set_fields(s, L[key]["component"], flds)
        publish.append(ids)

    # Green 0603 LED: Kingbright APT1608SGC. The chip generator's 0603 LED
    # is this datasheet's: 1.6 x 0.8 x 0.75 body, 0.8 x 0.8 lands 0.85
    # apart, pad 1 the cathode (K). Used as generated.
    root, ids = generate(s, "chip", "led-0603", "APT1608SGC")
    L["led"] = {k: ids[k] for k in KINDS} | {"revision": root["revision"]}

    # Omron B3U-1000P tactile switch (BOOT, RESET): a two-terminal chip
    # part redrawn as a push button, with the datasheet's lands and the
    # example's part file.
    L["sw"], ids = own("chip", "resistor-1206", "B3U-1000P")
    comp, sym, pkg, fp, bind = ids
    d = drafts(s)[comp]
    d["definition"]["body"]["designator_prefix"] = "SW"
    # A push button has no resistance: none of the resistor's fields, so no
    # resistor-code marking and no value template.
    d["definition"]["body"]["fields"] = []
    d["definition"]["body"].pop("marking", None)
    d["definition"]["body"].pop("value_template", None)
    edit(s, [{"id": comp, "meta": d["meta"], "definition": d["definition"]}], "B3U-1000P prefix")
    s.op("library.restyle_symbol", {"draft": sym, "kind": "pushbutton"}, "B3U-1000P push button")
    replace_pads(s, fp, [([-1.7, 0.0], [0.8, 1.7]), ([1.7, 0.0], [0.8, 1.7])],
                 "Omron B3U datasheet p.2, PCB Pad (Top View), B3U-1000P: two 0.8 x 1.7 pads, 2.6 between, 4.2 overall")
    # The designator sits outside the body outline, on the -y side.
    silk_half, silk_w, text_h, text_gap = 1.4, 0.12, 1.0, 0.15
    set_graphics(s, fp, extra=[rect([-1.5, -1.25], [1.5, 1.25], 0.1, "assembly"),
                               rect([-2.35, -1.5], [2.35, 1.5], 0.05, "courtyard"),
                               rect([-1.05, -silk_half], [1.05, silk_half], silk_w, "silkscreen")],
                 silk_text_at=[0.0, -round(silk_half + silk_w / 2 + text_gap + text_h / 2, 3)])
    generic_package(s, pkg, [3.0, 2.5, 1.6], "Omron B3U datasheet p.2, Dimensions (B3U-1000P): 3 x 2.5 body (4 over terminals), 1.6 high", "pa9t")
    # The 3D model: the example's B3U-1000P part file (original work, its
    # defaults are the datasheet's dimensions; see README, Sources).
    if s.rev is None:
        s.rev = s.revision()
    r = s.raw("library.use_part_file", {"expected_revision": s.rev, "package": pkg, "path": B3U_PART,
                                        "name": "B3U-1000P", "licence": "MIT OR Apache-2.0",
                                        "redistribution": "permitted"}, "B3U-1000P part file")
    s.rev = r["result"].get("revision", s.rev)
    L["sw"]["model"] = r["result"]["outcome"]["id"]
    publish.append(ids + [L["sw"]["model"]])

    # Pin headers: the header generator's, used as generated.
    for key, preset, name in (("h12", "header-1x12-2.54", "Pin header 1x12 2.54 mm"),
                              ("h4", "header-1x4-2.54", "Pin header 1x4 2.54 mm")):
        root, ids = generate(s, "header", preset, name)
        L[key] = {k: ids[k] for k in KINDS} | {"revision": root["revision"]}

    # TLV75733PDBVR 3.3 V 1 A LDO (module needs >= 0.6 A, datasheet Table 6-2):
    # the tabbed generator's SOT-23-5 with TI's DBV0005A example board
    # layout lands (p.35: 0.6 x 1.1 pads, 2.6 between row centres), then the
    # datasheet pinout onto its drafts.
    L["ldo"], ids = own("tabbed", "tabbed-sot-23-5", "TLV75733PDBVR",
                        {"lands": "explicit", "land_width": 600000, "land_length": 1100000, "land_span": 2600000},
                        "TLV75733PDBVR SOT-23-5")
    V = "SOT-23-5"
    pinout(s, "TLV75733PDBVR", "tlv757p", V, 5, [
        pin("in", "1", "IN", "power_input", V, page=3, table="Table 4-1"),
        pin("gnd", "2", "GND", "power_input", V, page=3, table="Table 4-1"),
        pin("en", "3", "EN", "input", V, "digital", page=3, table="Table 4-1"),
        pin("nc", "4", "NC", None, V, cls="device_nc", page=3, table="Table 4-1"),
        pin("out", "5", "OUT", "power_output", V, page=3, table="Table 4-1")], onto=L["ldo"]["component"])
    publish.append(ids)

    # TPD2E009DBZR USB ESD (SOT-23-3): TI's DBZ0003A example board layout
    # (0.6 x 1.3 pads, 2.1 between row centres).
    L["esd"], ids = own("tabbed", "tabbed-sot-23-3", "TPD2E009DBZR",
                        {"lands": "explicit", "land_width": 600000, "land_length": 1300000, "land_span": 2100000},
                        "TPD2E009DBZR SOT-23-3")
    V = "SOT-23-3"
    pinout(s, "TPD2E009DBZR", "tpd2e009", V, 3, [
        pin("dp", "1", "D+", "passive", V, "unspecified", page=3, table="Pin Functions"),
        pin("dn", "2", "D-", "passive", V, "unspecified", page=3, table="Pin Functions"),
        pin("gnd", "3", "GND", "power_input", V, "power", page=3, table="Pin Functions")], onto=L["esd"]["component"])
    publish.append(ids)

    # GCT USB4105-GF-A USB-C receptacle (USB 2.0, 16 contacts on 12 pads, shell).
    V = "USB4105"
    rows = [("a1b12", "A1B12", "GND"), ("a4b9", "A4B9", "VBUS"), ("b8", "B8", "SBU2"), ("a5", "A5", "CC1"),
            ("b7", "B7", "DN2"), ("a6", "A6", "DP1"), ("a7", "A7", "DN1"), ("b6", "B6", "DP2"),
            ("a8", "A8", "SBU1"), ("b5", "B5", "CC2"), ("b4a9", "B4A9", "VBUS"), ("b1a12", "B1A12", "GND"),
            ("shell", "S", "SHIELD")]
    ids = pinout(s, "USB4105-GF-A", "usb4105", V, 13,
                 # VBUS brings the board its power: a power output, so ERC
                 # finds the rail supplied. The other contacts are passive.
                 [pin(r, n, name, *(("power_output", V, "power") if name == "VBUS" else ("passive", V, "unspecified")),
                      page=1, table="Pin/Signal table") for r, n, name in rows])
    comp, sym, pkg, bind = ids
    # The stacked numbers ("A1B12", "B4A9") need 7.5 mm stubs at full
    # height; the pinout's 2.5 mm stubs shrank them to less than half of it.
    fit_pin_numbers(s, comp, sym, [n for _, n, _ in rows])
    # The receptacle's envelope and lands from the drawing (GCT USB4105
    # rev B4 sheet 1): 12 SMD lands, four plated shell slots and two NPTH
    # pegs, landing the pinout's terminals by number. Footprint origin: the
    # outline centre, mating face toward -y.
    d = drafts(s)[pkg]
    d["definition"]["body"]["envelope_nm"] = [nm(v) for v in (8.94, 7.35, 3.31)]
    edit(s, [{"id": pkg, "meta": d["meta"], "definition": d["definition"]}], "USB4105 envelope")
    lands = [("A1B12", -3.2, 0.6), ("A4B9", -2.4, 0.6), ("B8", -1.75, 0.3), ("A5", -1.25, 0.3), ("B7", -0.75, 0.3),
             ("A6", -0.25, 0.3), ("A7", 0.25, 0.3), ("B6", 0.75, 0.3), ("A8", 1.25, 0.3), ("B5", 1.75, 0.3),
             ("B4A9", 2.4, 0.6), ("B1A12", 3.2, 0.6)]
    slots = [{"number": "S", "at": [x, y], "slot": [0.6, length], "land": [1.0, land], "terminal": "S"}
             for y, length, land in ((-1.075, 1.4, 1.8), (3.105, 1.7, 2.1)) for x in (-4.32, 4.32)]
    fid = footprint_from_pads(s, pkg, [{"number": n, "at": [x, 3.68], "size": [w, 1.15]} for n, x, w in lands],
                              slots, [{"at": [x, 2.605], "diameter": 0.65} for x in (-2.89, 2.89)])
    # The pad table centres the envelope over the pads and outlines it
    # there; the drawing's origin is the outline centre, so the envelope
    # stays put and the outline is the drawing's: the 8.94 x 7.35 body on
    # assembly, silkscreen clear of the shell slots and open over the
    # lands, the courtyard and pin A1B12's dot.
    d = drafts(s)[pkg]
    d["definition"]["body"]["placement"] = {"translation_nm": [0, 0, 0], "rotation_mdeg": [0, 0, 0]}
    edit(s, [{"id": pkg, "meta": d["meta"], "definition": d["definition"]}], "USB4105 envelope at the outline centre")
    d = drafts(s)[fid]
    body = d["definition"]["body"]
    line = lambda a, b: {"shape": {"shape": {"line": {"a": pt(a), "b": pt(b)}}, "width_nm": 150000, "fill": "none"}}
    text = lambda at, layer: {"item": {"text": {"content": "designator", "at": pt(at), "height_nm": 1000000, "rotation": "r0",
                                                 "justify": {"h": "center", "v": "middle"}, "visible": True, "line_spacing": 1.6}},
                              "layer": layer, "side": "component"}
    silk = [line([-4.62, -3.825], [4.62, -3.825]), line([4.62, -3.825], [4.62, -2.2]), line([4.62, 0.05], [4.62, 1.83]),
            line([-4.62, -3.825], [-4.62, -2.2]), line([-4.62, 0.05], [-4.62, 1.83])]
    gfx = ([rect([-4.47, -3.675], [4.47, 3.675], 0.1, "assembly")]
           + [{"item": g, "layer": "silkscreen", "side": "component"} for g in silk]
           + [rect([-5.1, -3.95], [5.1, 4.55], 0.05, "courtyard"), text([0.0, 5.2], "silkscreen"), text([0.0, 0.0], "assembly"),
              {"item": {"shape": {"shape": {"circle": {"center": pt([-4.05, 3.68]), "radius_nm": 200000}}, "width_nm": 0,
                                  "fill": "solid"}}, "layer": "silkscreen", "side": "component"}])
    n = max(p["id"] for p in body["pads"]) + 1
    body["graphics"] = [dict(g, id=n + k) for k, g in enumerate(gfx)]
    body["next_local"] = n + len(gfx)
    # The locating pegs' holes open the mask to the hole itself.
    for p in body["pads"]:
        if p["kind"] == "non_plated_hole":
            p["mask"] = {"expansion_nm": 0}
    edit(s, [{"id": fid, "meta": d["meta"], "definition": d["definition"]}], "USB4105 outline")
    b = drafts(s)[bind]
    b["definition"]["body"]["footprints"] = [{"id": fid, "revision": 1}]
    edit(s, [{"id": bind, "meta": b["meta"], "definition": b["definition"]}], "USB4105 binding footprint")
    # The 3D model: the example's USB4105 part file (original work from the
    # same drawing; see README, Sources), whose shell front face the board
    # holds flush to its edge.
    if s.rev is None:
        s.rev = s.revision()
    r = s.raw("library.use_part_file", {"expected_revision": s.rev, "package": pkg, "path": USB4105_PART,
                                        "name": "USB4105-GF-A", "licence": "MIT OR Apache-2.0",
                                        "redistribution": "permitted"}, "USB4105 part file")
    s.rev = r["result"].get("revision", s.rev)
    L["usbc"] = {"component": comp, "symbol": sym, "package": pkg, "binding": bind, "footprint": fid,
                 "model": r["result"]["outcome"]["id"]}
    publish.append(ids + [fid, L["usbc"]["model"]])

    bad = check(s)
    if bad:
        raise SystemExit(f"blocking findings: {bad}")
    for group in publish:
        publish_and_close(s, group, "publish part " + drafts_name(s, group[0]))

    # Manufacturer Parts.
    P = []
    R, C3, C8 = L["r0603"], L["c0603"], L["c0805"]
    yag = ("Yageo RC_L series thick-film chip resistors datasheet", "https://www.yageo.com/upload/media/product/productsearch/datasheet/rchip/PYu-RC_Group_51_RoHS_L_12.pdf")
    for mpn, ohm in (("RC0603FR-075K1L", 5.1e3), ("RC0603FR-0710KL", 10e3), ("RC0603FR-07470RL", 470.0)):
        P.append(part(s, mpn, "Yageo", R["component"], R["binding"], [(F["res"], ohm, "exact_nominal")], *yag, desc="0603 1 % 0.1 W", rev=R.get("revision", 1)))
    mur = "Murata GRM series chip monolithic ceramic capacitors, product page datasheet"
    for mpn, farad, volt, c, url in (
            ("GRM188R71C104KA01D", 100e-9, 16.0, C3, "https://www.murata.com/en-us/products/productdetail?partno=GRM188R71C104KA01%23"),
            ("GRM188R61C105KA93D", 1e-6, 16.0, C3, "https://www.murata.com/en-us/products/productdetail?partno=GRM188R61C105KA93%23"),
            ("GRM21BR61C106KE15L", 10e-6, 16.0, C8, "https://www.murata.com/en-us/products/productdetail?partno=GRM21BR61C106KE15%23"),
            ("GRM21BR61A226ME51L", 22e-6, 10.0, C8, "https://www.murata.com/en-us/products/productdetail?partno=GRM21BR61A226ME51%23")):
        P.append(part(s, mpn, "Murata", c["component"], c["binding"],
                      [(F["cap"], farad, "exact_nominal"), (F["volt"], volt, "minimum_rating")], mur, url,
                      rev=c.get("revision", 1)))
    for key, mpn, man, ds in (("led", "APT1608SGC", "Kingbright", "apt1608"), ("sw", "B3U-1000P", "Omron", "b3u"),
                              ("ldo", "TLV75733PDBVR", "Texas Instruments", "tlv757p"),
                              ("esd", "TPD2E009DBZR", "Texas Instruments", "tpd2e009"),
                              ("usbc", "USB4105-GF-A", "GCT", "usb4105")):
        t, rev, url = DS[ds]
        P.append(part(s, mpn, man, L[key]["component"], L[key]["binding"], [], t, url, rev,
                      rev=L[key].get("revision", 1)))
    for key, mpn in (("h12", "TSW-112-07-G-S"), ("h4", "TSW-104-07-G-S")):
        P.append(part(s, mpn, "Samtec", L[key]["component"], L[key]["binding"], [],
                      "Samtec TSW through-hole 2.54 mm terminal strip", "https://www.samtec.com/products/tsw",
                      rev=L[key].get("revision", 1)))
    P.append(part(s, "ESP32-S31-WROOM-3", "Espressif", 1, 4, [],
                  "ESP32-S31-WROOM-3 & WROOM-3U Datasheet", "https://documentation.espressif.com/esp32-s31-wroom-3_wroom-3u_datasheet_en.pdf", "v0.7"))
    publish_and_close(s, P, "publish Parts")
    L["parts"] = P
    return L
