# Via types and drills: blind, buried, laser, back-drilled, filled and capped

The design. S1a, S2 and S1b are written (through vias, filled and capped); the other slices are not. Hours are estimates of agent time, not measurements. Every fabricator, specification and tool documentation page was read on 2026-11-24 unless noted. Code references are to `integ-3` at 8f6b8d6b.

A mock-up of the window is in `via-types-mockup.html`. Its build orders, effort counts and refusals were computed by a small prototype of section 3.

Behaviour comes from this project's documents, the Gerber, XNC and job file specifications, the fabricators' own pages, and Altium's public user documentation. No other tool's code was read.

## 1. The concept

A **drill** is one kind of hole the board can have: it goes from a first layer to a last layer, through everything between. A **via** uses one drill and has its own fill. Three places hold the facts, and each fact is in one place:

| Place | Holds | Why there |
| --- | --- | --- |
| The via (`Via`) | `span`, `drill`, `diameter`, `tent`, `fill` | Fill is a setting on one via (founder, 2026-11-24). |
| The stackup (`Stackup`) | `drills`: the drills this board uses besides through | What is drilled is part of how this board is built. It moves with the layers. |
| The one fab profile | what the fabricator can make, every limit, the published charges, the order text, each with its source | It is the fabricator's statement. It never lists combinations. |

The board picks. The check says what the profile does not make. The window says what the picks cause.

### The drill

```
Drill { first: CopperLayer, last: CopperLayer, kind: mechanical | laser | back }
```

- **The word is "drill", never "pair".** A through drill on eight layers joins eight layers. It is L1–L8, not a pair. Outputs say "from layer 1 to layer 8", as the specifications do.
- **Through** is always there and cannot be removed. It is not stored. Component and mounting holes use it too.
- `mechanical` and `laser` are plated. A drill of one first and last layer has one plated kind, so a via's two layers name its drill.
- `back` removes plating. It starts on an outer layer and stops before `last`, which keeps its copper. It is designed here and built later (section 5).
- The default is no drills besides through. Every board today is unchanged.
- Sizes are not part of a drill. They come from the via, its net class and the profile's limits.
- Drills are edited through `layout.edit_stackup`.

**Blind, buried, through and microvia** are words computed from a drill: blind has one outer end, buried none, microvia is a laser drill. Nothing stores them. A drill is named by them: "Buried L2–L5", "Laser L1–L2".

**The lamination step a drill is made in is derived** (section 3). It is never entered, so it cannot contradict the layers.

### The via

```
ViaSpan  = Through | Between { from, to }        (exists; Between is refused today)
ViaFill  = none | plugged | capped | copper
Tent     = { top: bool, bottom: bool }           (replaces tented: bool)
```

| Fill | Meaning | IPC-4761 value written |
| --- | --- | --- |
| `none` | the hole stays open under any solder mask | field left out |
| `plugged` | closed with solder mask ink, mask over both faces | field left out (see below) |
| `capped` | filled, then plated over with copper, solderable | `VII` |
| `copper` | a laser via filled with plated copper | field left out (see below) |

- `none` is the default and is today's via. A saved via without the field reads as `none`.
- `Through` always means the whole board. `Between { Top, Bottom }` is stored as `Through`. `from` is always the upper layer.
- A tent on a face the via does not reach is refused.
- `plugged` needs both tents. JLCPCB: "Filled vias must not have soldermask openings on either side" ([capabilities][jcap], Plugged vias).
- The filling material (epoxy or copper paste) is a board-wide order choice. It uses the existing `FabOption` mechanism, not a field on the via.

**IPC-4761 is a paid standard and was not read.** The names come from two public sources. Ucamco's Gerber Job Format 2020.08, p. 8, lists `Ia` Tented - Single-sided, `Ib` Tented - Double-sided, `IIa`/`IIb` Tented and Covered, `IIIa`/`IIIb` Plugged, `IVa`/`IVb` Plugged and Covered, `V` Filled (fully plugged), `VI` Filled and Covered, `VII` Filled and Capped, `None` No protection. PCBWay's [via covering page][pcov] adds materials.

- `capped` is `VII` in both.
- An ordinary tented via gets **no** value. PCBWay's table gives type I the material "Dry film solder mask", and liquid mask ink over a hole is not that. Writing `Ib` for every tented via would claim a process nobody ordered. The mask layer already says which vias are covered.
- `plugged` gets no value. PCBWay's types III and IV are "Plugging Epoxy"; JLCPCB's plugging is "Filled with soldermask". The order notes say it instead.
- `copper` gets no value. Neither source lists a plated-copper filled laser via.

### When things change

- **Restack.** Both ends of a drill and of every via move with their layer. `layout/restack.rs:104` already does this for vias. A removed end layer refuses the edit and names the vias and the drill. A drill whose ends become Top and Bottom becomes through, with its vias; the preview says so.
- **Another profile.** Vias and drills never change. What the new profile does not make becomes a finding, and the profile suggestions that exist today (`fabrication/suggest.rs`) list the profiles that make it.
- **Removing a drill** that vias use is refused, naming the count.
- **Unused drills** are kept, marked unused, and write no file. They still take part in the check that the set can be built.

### What the model leaves out, on purpose

- No named via styles and no library of them.
- No stored stack of vias. A stack is vias at one point on one net.
- No per-via filling material; it is a board-wide order choice (`FabOption`).
- No laser taper in geometry. The fabricators publish one diameter.
- No prices.

### What was learned from Altium's documentation

Altium's public pages were read for the idea only: [layer stack][alt1], [via definitions][alt2], [back drills][alt3].

| Altium documents | Taken | Not taken |
| --- | --- | --- |
| Definitions have a first and a last layer; "The default thruhole span cannot be deleted" | yes | |
| Names come from the layers; the type is detected | yes | |
| Sizes "are not defined in the Via Types tab" | yes | |
| "All vias used in the board must be one of the Via Types defined" | yes | |
| A mirror option for symmetric stacks | yes, as "Both sides" | |
| "Create Via Types from Used Vias" | only for import | |
| Back drill "Last layer … drilling stops before" | yes, the same meaning as Gerber's example | |
| A microvia's order of first and last sets its drilling direction | | direction is derived from the build |
| Back drilling set by a stub length rule | | later, by a net class limit; first explicit |
| No lamination order, fabricator verdict or effort appears on the pages read | | these are ours |

## 2. What the fabricators publish

### JLCPCB, conventional boards

Source: [capabilities][jcap].

| Subject | Text |
| --- | --- |
| Via size | "Multilayer: 0.15 mm hole size / 0.25 mm via diameter (0.1mm hole size / 0.2mm via diameter only available for board thickness ≤1mm; ENIG/OSP surface finish required)" |
| Filled and capped | "Vias are filled with epoxy resin or copper paste and then plated over to achieve an opaque and smooth finish." "Compatible with via diameters from 0.15 to 0.55 mm." Named "Epoxy Filled & Capped" and "Copper paste Filled&Capped" |
| Default | "This process is the default for 6-layer and above multilayer boards." |
| Plugged | "Filled vias should have ≥ 0.35 mm clearance from other soldermask openings (e.g. pads)"; "Filled vias must be no wider than 0.5 mm diameter" |
| In a pad, without HDI | "Via-in-pad alone does not necessarily require HDI: filled and capped through vias can also be used on conventional multilayer boards." ([HDI page][jhdi]) |
| How to say which | "For epoxy-filled vias, please leave a note or upload an image along with your design files to explain which vias should be filled. Alternatively, you can specify to fill all vias of a particular diameter." ([via covering][jcov]) |

JLCPCB's pages disagree about the smallest filled hole:

| Page | Filled and capped hole | Depth ratio |
| --- | --- | --- |
| [Capabilities][jcap] | 0.15 to 0.55 mm | not given |
| [POFV news][jnews] | "Via hole size: 0.2 to 0.5 mm", ring "0.05 mm minimum, 0.075 mm preferred" | not given |
| [Design guide][jblog] | 0.25 to 0.35 mm, "Min: 0.20 mm / Max: 0.50 mm" | "Less than or equal to 8:1", "Max Limits: 10:1" |
| [Via covering help][jcov] | "should be less than or equal to 0.5mm" | not given |

Where its pages conflict the size is not stated (decision 2). **The multi-tool's 0.15 mm filled via is not stated under this profile: it can be ordered only with a waiver that gives JLCPCB's answer as its reason.**

The 4-layer charge: the news page says "POFV for 4-layer PCBs still requires charges." The amount is not published. The order form is drawn by a script, so its labels for via covering were **not read**. Only its text on via size was.

### JLCPCB, HDI

Sources: [HDI page][jhdi] and [HDI help][jhelp] ("Last updated on Nov 09, 2026").

| Subject | Text |
| --- | --- |
| Layers | "4–24 layers", "1st-, 2nd-, and 3rd-order HDI" |
| Base material | "Select HDI under Base Material"; Shengyi S1000-2M or Nan Ya NP-175F |
| Laser blind via | 0.075–0.15 mm, "Default laser-via diameter is 0.10 mm"; "Pad diameter = laser-via diameter + 6 mil", ring "3 mil per side" |
| Laser depth ratio | "≤1:1" |
| Laser dielectric | 0.05–0.127 mm ([help][jhelp]); the HDI page also lists 2116 at 150 µm |
| Buried via | mechanical, 0.15–0.55 mm, pad + 6 mil, core "≥0.30 mm" across the span, ratio "≤16:1"; 0.10 mm when the span is ≤1.0 mm |
| Fill | "Laser blind vias are copper filled and planarized"; buried vias "resin plugging and copper capping" |
| Spacing, blind via to any via | same net ≥0.13 mm, other net ≥0.24 mm, edge to edge |
| Via edge to trace | ≥6 mil (0.15 mm) |
| Blind via to board edge | ≥0.35 mm |
| Stacked, staggered | "JLCPCB supports both stacked and staggered microvias within its HDI design rules." Staggered is "generally preferred". |
| Skip via (L1 to L3) | "manufactured as stacked blind/buried-via structures" |
| Files | "Clearly identify the start and end layers in each blind- or buried-via drill-file name—for example, 1_2 for L1–L2 or 3_4 for L3–L4." |
| Review | "Blind/buried-via and lamination charges are confirmed after engineering review." |

Conflicts and gaps:

- Finished thickness: 0.5–3.0 mm on the HDI page, 0.5–2.4 mm in the help. The profile takes the narrower and quotes both.
- An old JLCPCB help page ([naming patterns][jname]) still says "we don't support blind/buried vias". The dated HDI help contradicts it.
- Not published: an offset for staggered vias, a limit on how many are stacked, a blind via drilled mechanically, one-sided build-ups.

### PCBWay

| Subject | Text | Source |
| --- | --- | --- |
| HDI | material choice "HDI(Buried/blind vias) ≥4 Layers"; "HDI is available for 4-layer or more." | [order form][porder] |
| Structures | "1+N+1, 2+N+2 , ... , 6+N+6（≥6 orders require evaluation）", up to 60 layers (evaluation above 30) | [HDI][phdi] |
| Laser | "Standard 4 mil, 3 mil require evaluation (corresponding to single 106PP)"; "8 mil (corresponding dielectric thickness cannot exceed 0.15mm)" | [HDI][phdi] |
| Mechanical | 0.15 mm; "Min. controlled depth drilling PTH: 0.15mm"; ratio "Max 14:1; evaluate if greater" | [HDI][phdi] |
| Advanced table | ratio ≤8 normal, hole copper thickness for "Blind hole (mechanical hole)" and "Buried hole" | [capabilities][pcap] |
| Stacked, staggered | "staggered and stacked vias" for HDI, on a 2016–2018 roadmap | [advanced][padv] |
| Filled | "Diameter range of resin-filled vias 0.254-6.5mm" on the HDI page; "Min.size can be plugged: 0.15mm(CNC)", max 0.7 mm on the roadmap | [HDI][phdi], [advanced][padv] |
| Order form | "Via process: Tenting vias / Plugged vias with solder mask / Vias not covered"; "All vias filled with resin and capped"; "Via in pad" | [order form][porder] |
| Form and files | "For Gerber files this choice will not affect anything, PCB will be made using the parameters of the files." and "If you want to do filled vias with resin, please select in Additional Options!" | [order form][porder] |
| Via in pad | "requires resin filling, with an electroplated cap" | [via-in-pad][pvip] |
| Mechanical drills | "must always span an even number of copper layers"; "cannot terminate on the top side of a core"; "cannot start on the bottom side of a core"; "cannot start or end inside another blind or buried via unless one is completely enclosed" | [blind and buried][pblind] |

**PCBWay does not publish** a laser annular ring, a laser depth ratio, spacing between laser vias, any stacking limit, or a file naming rule. Its figures for filled holes conflict (0.15 mm or 0.254 mm). For an HDI build these are `not stated` (below).


### Back-drilling

| Subject | JLCPCB ([capabilities][jcap]) | PCBWay ([backdrill][pbd]) |
| --- | --- | --- |
| Boards | "Supports 4-32-layer FR4 boards with a thickness of ≥0.8mm" | "Typically used for boards with four layers or more." "0.8mm is acceptable." |
| Hole | "0.2-0.5mm(Epoxy filled after backdrilling)" | "Minimum diameter for back-drilled through holes is 0.15mm" |
| Back drill | "typically 0.2mm larger than through-hole diameter" | "0.1mm larger than through holes" |
| Depth | "≥0.15mm(from bottom of backdrill to the adjacent inner copper layer)" | tolerance "+/-0.1mm" |
| Around it | "≥0.2mm(edge to pad/trace/copper safety distance)" | not published |
| With HDI | not stated | not stated |

### As one profile

A profile says what is possible as **rules over layers**, not as a list of combinations. Numbers stay in `rules`. A new list, `drill_rules`, holds the structure:

```json
"drill_rules": [
  {"id": "through", "kind": "mechanical", "layers": {"span": "board"},
   "fills": [{"fill": "none"}, {"fill": "plugged", "…": "…"},
             {"fill": "capped", "default_from_layers": 6, "order": {"…": "…"}}]},
  {"id": "laser", "kind": "laser", "copper_layers": [4, 24],
   "layers": {"span": "build_up", "crosses": 1, "order_max": 3},
   "stacked": "stated", "staggered": "stated", "fills": [{"fill": "copper"}]},
  {"id": "buried", "kind": "mechanical", "copper_layers": [4, 24],
   "layers": {"span": "inner"}, "fills": [{"fill": "capped"}]},
  {"id": "back", "kind": "back", "copper_layers": [4, 32], "layers": {"span": "from_face"}}
]
```

Every entry and value carries its source text and date.

- `build_up` with order *k* means: L1–L2 to Lk–L(k+1), and the mirror from the bottom. On eight layers at order 2 that is L1–L2, L2–L3, L6–L7 and L7–L8. It covers every layer count, so no profile is made per count or combination.
- `inner` means neither end is outer. `board` is through. `from_face` starts on an outer layer.
- **JLCPCB stays one profile.** `jlcpcb-4-layer` already covers 4 to 32 layers. It gains one option, **Build**, with the choices Standard and HDI, and the HDI limits as `cases` of the same rules. This is the mechanism of `rules.set_fab_options`.
- **Build follows the board.** Like the solder mask colour today (`layout/mask_colour.rs`, `FabOption::follows`), it is chosen by the board's drills: a laser or buried drill in use makes it HDI, and no command sets it. So the order and the design cannot disagree. Adding the first such drill previews the limits that change.
- **An HDI limit the fabricator does not publish is not stated.** It is never taken from the conventional row (founder, 2026-11-24). A case says so in place of a value: `{"when": {"build": "hdi"}, "not_stated": true, "source": "…"}`.
- `ChoiceRequires` gains layer counts and a least thickness (0.5 mm, 0.5 mm only on 4 layers).
- A profile without `drill_rules` makes through drills with fill `none` only, as today.
- **PCBWay stays one profile per existing file**, with its HDI structures, laser and back-drill rules added to it. Its shipped profiles cover 2 and 4 layers today.
- New optional rule keys: laser drill, ring, depth ratio and dielectric window; buried drill, ring, ratio and core; capped and plugged drill ranges; same-net hole spacing; back-drill growth, depth and distance. One new `CopperObject`, `LaserHole`, brings its clearance rows.

**Three verdicts**, never two:

| Verdict | When | Effect |
| --- | --- | --- |
| made | a rule covers it | nothing |
| not made | the profile states a rule and this is outside it | error, never waivable, carries a need so profile suggestions work |
| not stated | the fabricator says nothing: a feature (PCBWay stacking, JLCPCB back drill on an HDI board) or an HDI limit the board uses | error; a person may waive it with a reason, and the drawing asks the fabricator to confirm, as waivers do today |

#### A limit that is not stated

A rule that is not stated has no number. Zero would let copper touch, and the conventional number is not the fabricator's.

- `board.limit_not_stated.<rule>` names each not-stated rule **that the board uses**: an unplated-hole clearance only if there is an unplated hole. It is an error; a person may waive it with a reason.
- A tool that needs the number (routing, a fill, a via size) refuses and names the rule: "JLCPCB states no unplated hole clearance for HDI; set your own in Board › Rules".
- A project override (`rules.set_override`, which exists) gives the number. It is the designer's value and is shown as such. Tools and checks then use it, and the finding stays until it is waived, because the fabricator has still said nothing.
- Conventional builds keep today's convention ("not published: 0, not checked"). It is unchanged there.

What JLCPCB states for HDI, from its [HDI page][jhdi] and [help][jhelp], and the HDI entry on [capabilities][jcap]:

| Stated | Not stated there |
| --- | --- |
| line and space 3/3 mil | pad to pad, pad to track, fill spacing as such |
| laser drill, pad, ring, ratio, dielectric | through and buried hole to hole spacing other than next to a blind via |
| buried drill, pad, ring, ratio, core, fill | ring and copper clearance of component (PTH) holes |
| through hole ≥ 0.15 mm, pad 0.10 mm larger, ratio ≤ 16:1 | unplated (NPTH) hole clearances and rings |
| blind via to any via, same and other net | filling a through via, and the default from 6 layers |
| via edge to trace ≥ 0.15 mm | via edge to a pad or fill |
| blind via to board edge ≥ 0.35 mm | copper to board edge, other holes to board edge |
| thickness, layers, order | solder mask expansion and web, legend, paste |

The right column is read strictly: a row counts only if an HDI text states it. The capabilities page lists HDI among its materials and gives mask, legend and outline rows without naming a build. Whether those rows count as stated for HDI is open (decision 1). Until it is decided they do not.

## 3. What is derived: can it be built, and in which order

Input: the dielectrics' kinds (core or prepreg) and the drills. Output: the stages, the step of each drill, the effort, or a reason.

A **stage** is a run of layers that is one solid piece at some moment. The whole board is the last stage. A core with its two foils is a stage that needs no pressing. Any other stage is one pressing.

1. A stage ends only where the next dielectric is prepreg. A core cannot be split. (PCBWay: a via "cannot terminate on the top side of a core".)
2. Two stages nest or share no layer. (PCBWay: "unless one is completely enclosed within the other".)
3. A plated mechanical drill needs a stage with exactly its first and last layers outside. It is drilled in that stage's step and passes everything in it.
4. A laser drill needs a stage with one of its ends outside. It crosses prepreg only. It is fired at the first stage that offers it a face.
5. An inner copper layer lies on a core or is the outside of some stage. A foil between two prepregs forces a pressing, with or without vias.
6. Back drills and unplated holes come after the last plating.

Of all stage sets that satisfy the drills, the engine takes the one with the fewest pressings, then the fewest one-sided stages, then the fewest stages. The prototype does this by trying sets; a board has at most 64 layers, and candidate stages are only the runs that rule 1 allows. The product needs a bounded search and refuses past the bound, naming it.

When none exists it names the cause: "L2 is the upper face of the core L2–L3; a drill cannot end inside a core", "crosses Buried L2–L5", "L2–L3 is a core; a laser drills prepreg only".

### Several drills at once, or separate operations

| Question | Derivable? |
| --- | --- |
| Which drills are made in the same step | yes: those of one stage, and those of stages that do not depend on each other |
| A through drill passes all layers at once | yes: one hole, one plating, after the last pressing |
| L1–L2 and L(n-1)–Ln laser | same step, two faces |
| L2–L3 and L4–L5 in two cores | same step, two separate panels, before pressing |
| L1–L2 and L1–L4 mechanical | different steps: the inner stage first |
| Fewest pressings, plating steps and laser steps | yes, as the minimum |
| Order inside one step (through drill before or after laser) | no. Nothing read states it. |
| Whether filled and open through vias need two drill passes | no. JLCPCB's filling text describes the steps of a filled via only. Shown as one fill and cap step, no second pass claimed. |
| How many panels, machines or depths per pass | no |
| The fabricator's real sequence | no. The window says "a possible build; the fabricator decides". |

### The founder's example, computed

On a 4-layer board of prepreg, core, prepreg:

| Drills | Build | Pressings | Mechanical | Laser | Fill and cap | Order |
| --- | --- | --- | --- | --- | --- | --- |
| Through | press L1–L4, drill | 1 | 1 | 0 | 0 | 0 |
| + laser L1–L2 and L3–L4 | press, drill, laser both faces | 1 | 1 | 1 | 0 | 1 |
| + buried L2–L3, capped | core drilled, plated, filled and capped; press; drill; laser | 1 | 2 | 1 | 1 | 1 |
| mechanical L1–L2 instead | not possible: L2 is the upper face of the core | | | | | |

So 2–3 adds a drill, a plating and a fill step, and no pressing. Whether the fabricator then charges for a pressing is its own matter.

**HDI order** is the longest chain of nested stages that lasers fire from. This is my reading. JLCPCB's: "1st-order means one lamination cycle and one laser drilling operation; 2nd-order means two" ([help][jhelp]). Our pressings also count the press of an inner stage, so a 1+4+1 board with a buried L2–L5 has two pressings and order 1. The window shows both and quotes JLCPCB.

## 4. Effort, shown and not guessed

The drill view shows, for the drills in use, next to the same stackup with through only:

- pressings, mechanical drill steps, laser steps, plating steps, fill and cap steps, back-drill steps, HDI order;
- for each, **which drill it goes away without**, computed by taking each drill out; for lasers, the group that shares a step;
- only drills in use are counted; a defined but unused drill shows what it would add;
- before a drill is added, what it would add ("+1 mechanical, +1 plating, +1 fill and cap").

JLCPCB names the drivers: "The main HDI-specific cost drivers are the number of lamination cycles, laser-drilling stages (HDI order), and mechanical buried-via drilling operations. Layer count also affects the total cost." ([HDI page][jhdi]).

**Charges** are profile data: a condition, the published sentence, an amount only if published, and the source. A charge is shown only when its condition holds on the board.

| Condition | Published text | Amount |
| --- | --- | --- |
| via hole < 0.3 mm and diameter ≤ 0.4 mm | "Additional charges will apply …" (JLCPCB quote page) | not published |
| 0.1 or 0.15 mm hole | "will cost more" ([capabilities][jcap]) | not published |
| capped, 4 layers | "POFV for 4-layer PCBs still requires charges." ([news][jnews]) | not published |
| over 150,000 holes per m² | "an extra cost will be applied" ([extra charges][jextra]) | not published |
| HDI | "Blind/buried-via and lamination charges are confirmed after engineering review." ([HDI page][jhdi]) | not published |
| drill under 0.3 mm | "will be subject to extra charges" (PCBWay, quoted in the shipped profile) | not published |

No page read gives an amount for drilling, filling or lamination. So no amount is shown.

Templates, computed by the prototype:

| Layers | Template | Dielectrics, top to bottom | Pressings | Mechanical | Laser | Fill and cap | Order |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4 | Through only | pp core pp | 1 | 1 | 0 | 0 | 0 |
| 4 | Blind top | pp core pp | 1 | 1 | 1 | 0 | 1 |
| 4 | 1+N+1 | pp core pp | 1 | 1 | 1 | 0 | 1 |
| 4 | 1+N+1 and buried core | pp core pp | 1 | 2 | 1 | 1 | 1 |
| 6 | Through only | pp core pp core pp | 1 | 1 | 0 | 0 | 0 |
| 6 | Blind top | pp core pp core pp | 1 | 1 | 1 | 0 | 1 |
| 6 | 1+N+1 | pp core pp core pp | 1 | 1 | 1 | 0 | 1 |
| 6 | 1+N+1 and buried core | pp core pp core pp | 2 | 2 | 1 | 1 | 1 |
| 6 | 2+N+2 | pp pp core pp pp | 2 | 1 | 2 | 0 | 2 |
| 6 | 2+N+2 and buried core | pp pp core pp pp | 2 | 2 | 2 | 1 | 2 |
| 8 | Through only | pp core pp core pp core pp | 1 | 1 | 0 | 0 | 0 |
| 8 | Blind top | pp core pp core pp core pp | 1 | 1 | 1 | 0 | 1 |
| 8 | 1+N+1 | pp core pp core pp core pp | 1 | 1 | 1 | 0 | 1 |
| 8 | 1+N+1 and buried core | pp core pp core pp core pp | 2 | 2 | 1 | 1 | 1 |
| 8 | 2+N+2 | pp pp core pp core pp pp | 2 | 1 | 2 | 0 | 2 |
| 8 | 2+N+2 and buried core | pp pp core pp core pp pp | 3 | 2 | 2 | 1 | 2 |

## 5. Back-drilling, designed now and built later

- A back drill is `Drill { first: Top or Bottom, last, kind: back }`. It removes plating from the face to just before `last`. Gerber's example is the same: "backdrilling to remove via plating between layers 8 and 7 … `NonPlated,8,7,Blind`" (2026.05, p. 172).
- A via lists the back drills it uses, at most one per face. Only a plated drill that reaches that face may.
- `Via::on` drops the removed layers. The graph, lands, clearance, fills, lengths and stubs then follow. A track on a removed layer is unrouted, not silently cut.
- The hole it makes is larger than the via's. The profile gives the growth. Copper keeps the profile's distance from that larger hole on the removed layers. Pads on them are not written.
- The check compares the dielectric below `last` with the profile's least depth. JLCPCB's 0.15 mm refuses a stop at L6 above a 0.10 mm prepreg.
- File: `name-backdrill-8_6.drl`, `.FileFunction,NonPlated,8,6,Blind`, tool function `BackDrill`, tool size the via's: "The drill tool has the same diameter as the via – the manufacturer will determine the tool diameter." The specification writes `BackDrill` on p. 153 and `Backdrill` on p. 172; the table's spelling is used.
- 3D: the removed part of the barrel is cut from the plan. JLCPCB fills the hole with epoxy afterwards; the order notes say so.
- A stub limit on a net class can later propose back drills. It is not part of the first build.

## 6. Effects, with the code each touches

"Aware" means it already asks `Via::on`.

| Area | Today | Change |
| --- | --- | --- |
| Document rules | `layout/via.rs:911` refuses `Between` | accept a via whose layers match a defined drill; refuse tent on an unreached face |
| Copper graph | aware; C4 is closed (`docs/copper-graph.md`) | none. Stacked vias join by their overlapping lands on the shared layer. |
| Net length, pair path | aware (graph barrels) | none |
| Stub | not reported | `layout.length_report` gains each via's unused barrel length, read off the graph |
| Impedance | planar model only | none; a via has no impedance model (G2 stays open) |
| Copper clearance | aware (`layout/via.rs:191`, `:226`) | none |
| **Hole to copper** | **not aware**: `layout/via.rs:637` measures every near shape on every layer | only shapes on a layer of the span; `LaserHole` for laser |
| **Hole to hole** | **not aware**: `layout/via.rs:664` | only holes that share a dielectric; a same-net concentric pair is a stack |
| **Pad and board holes** | `pad_hole.rs`, `mount()` in `layout/via.rs:693` | these pass through everything, so they meet every via; kind becomes `LaserHole` where it is laser |
| Aspect ratio | `layout/via.rs` uses the board thickness | the depth of the span: dielectrics and foils between its ends |
| Ring, drill | `via_size_fits` (`layout.rs:1295`), `ViaAnnularRingMin` | read by process |
| Fills, antipads, thermals | aware (`fill/compute.rs:314`) | the antipad's hole part uses the via's hole kind |
| Router world | aware in `collides` (`router/world.rs`) | `too_close` takes the hole kind of each piece |
| Router, hard-coded span | `route_tool.rs:833`, `pair.rs:415` write `Through` | take the chosen span and fill |
| Shove | moves vias | a via concentric with another never moves; it is an obstacle, named "stacked via" |
| Pair tool | `via_pitch` (`pair.rs:504`) | uses the chosen drill's hole kind |
| Keepouts | per layer, `router/world.rs:540` | none: a piece's layers come from the span |
| Via in pad | accepted silently | finding when an open hole lies under a paste opening (below) |
| Net classes | width, gap, via size | `vias: { spans, max }` and `layers` (below) |
| Restack | aware (`restack.rs:104`) | drills move too |
| **Copper Gerbers** | **not aware**: `artwork.rs:359` flashes every via's pad on every layer | filter with `Via::on` |
| **Mask** | **not aware**: `artwork.rs:413` opens an untented via on both faces | by tent, only on faces it reaches |
| Back drill (later) | none | `Via::on` also drops the layers the back drill removes, so the graph, lands, fills and lengths follow |
| **Board plan** | refused (`plan.rs:361`); drills cut in every slab (`plan.rs:863`) | cut only the span's slabs; barrel range by span exists (`plan.rs:803`) |
| Capped fill in 3D | none | the end foils are full discs, a fill body fills the bore |
| Live 3D and exact | both read the plan | none beyond the plan; `board-parity` holds them |
| KiCad import | maps the layer pair (`routing.rs:397`), then the engine refuses it; tents front and back are merged with "and" | adds the drill to the stackup in the import review; keeps both tents; a drill that cannot be built is refused by name |
| Footprint pad vias (`PadVia`) | through, no lands elsewhere, written as `ComponentDrill` | gains `fill`; written as `ViaDrill` (see finding F2) |
| fabcheck | `sets.rs:172` keys drill files by "Plated" | key by the whole file function; refuse two files of one function |

**Finding F1 (existing fault).** `excellon.rs:152` writes `TA.AperFunction,Plated,PTH,ViaDrill`. The XNC specification 2021.11, p. 18, shows `; #@! TA.AperFunction,ViaDrill`. The Gerber specification 2026.05, p. 153, puts the function first. A strict reader sees the unknown function "Plated" and loses the via information. It must be fixed before the IPC-4761 field is added, because that field follows `ViaDrill`.

**Finding F2 (existing fault).** A pad-owned via is written as `ComponentDrill`. The Gerber specification 2026.05, p. 153, defines that as "a hole that is used for the attachment and/or electrical connection of component terminations" and `ViaDrill` as a hole "whose sole function is to connect different layers".

### Net classes (closes G5's via part)

- `vias: { spans: [..], max: n }`. Absent means any drill the stackup has. An empty list means no vias. `max` is per net.
- `layers: [..]`. Absent means any copper layer.
- The router offers only allowed drills and refuses a layer change with the class's name. The check finds what got in by import or a class edit.
- The multi-tool: A-row class `spans: []`; B-row class `spans: [through], max: 1`. The four filled vias need no exception.

### Findings

| Code | Severity | When | Waivable |
| --- | --- | --- | --- |
| `board.drill_build` | error | no lamination sequence serves the drills | no |
| `board.drill_not_made` | error | the profile's rules exclude this drill, process, structure or fill | no |
| `board.drill_not_stated` | error | the fabricator states nothing about it | with a reason |
| `board.limit_not_stated.<rule>` | error | the board uses an HDI limit the fabricator does not publish | with a reason |
| `board.via_stack` | error | a stack or stagger the profile excludes | no |
| `board.drill_unused` | information | a drill no via uses | |
| `board.via_in_pad_open` | warning | an open hole under a paste opening | yes |
| `board.via_fill_told` | warning | filled and unfilled vias share a hole size and the fabricator is told by diameter | yes |
| `board.via_class` | error | a via or track its net class does not allow | as other class findings |
| existing `board.via_*` | as today | limits read by process and depth | as today |

JLCPCB fills and caps by default from 6 layers on conventional boards. A board there with `none` vias gets a finding that asks to set them to `capped` or confirm (decision 3). For HDI its pages do not state how through vias are filled, so a fill there is not stated.

## 7. How the fabricator is told

### Drill files

One file per drill. The Gerber specification 2026.05, §6.6, p. 172: "Each drill span (from-to layers) must be put in a separate Gerber file. PTH and NPTH must be split into two separate files." XNC files take the same attributes.

| Drill | File | `.FileFunction` |
| --- | --- | --- |
| Through | `name-plated.drl` (as today) | `Plated,1,4,PTH` |
| Blind, L1 to L2 | `name-plated-1_2.drl` | `Plated,1,2,Blind` |
| Buried, L2 to L3 | `name-plated-2_3.drl` | `Plated,2,3,Buried` |
| Back drill, L8 to L6 | `name-backdrill-8_6.drl` | `NonPlated,8,6,Blind` |

The quotation, Gerber Layer Format Specification revision 2026.05, p. 142: "`Plated,i,j,(PTH|Blind|Buried)[,<label>]` Plated drill/rout data, span from copper layer i to layer j. The from/to order is not significant. The (PTH|Blind|Buried) field is mandatory. The label is optional. If present it must take one of the following values: Drill, Rout or Mixed." For plated drills `i` is always the smaller number here.

A laser L2–L3 is `Buried` in the file: it reaches no outer layer.

**Laser is not in X2** on the pages read (pp. 142, 153). The process goes in a header comment, the drill table, the drawing and the order notes. The file name follows JLCPCB's `1_2` request.

### Tools and the IPC-4761 field

The quotation, revision 2026.05, p. 153: "`ViaDrill[,<IPC-4761>]` … `<IPC-4761>=(Ia|Ib|IIa|IIb|IIIa|IIIb|IVa|IVb|V|VI|VII|None)` … An optional field specifies the via protection according to IPC-4761". The field came with revision 2019.09 (p. 214).

- A capped via's tool: `; #@! TA.AperFunction,ViaDrill,VII`.
- The tool key gains span, process and fill. A filled and an open via of one size get two tools. The specification asks for this, p. 151: "One function, one aperture. Objects with different functions must be created with different apertures, even if they have the same shape and size."
- Tool numbers stay unique across all files.
- No `Viafill` layer is written. P. 144: "It is however recommended to specify the filled via's with the optional field in the .AperFunction ViaDrill."

### The other outputs

| Output | Change |
| --- | --- |
| Drill map | one per drill file; `Drillmap` has no layer field (p. 144), so its name carries it |
| Drill table | columns From, To, Process, Fill; totals per drill |
| Job file | `GeneralSpecs.ViaProtection` lists the values the tools carry, here `["VII"]` (job format 2020.08, p. 8: "This via protection types present"). Left out when no tool carries one. |
| Fabrication drawing | the section with its drills, one line per drill and fill with count and size, the build order when it has more than one step, marked "a possible build; the fabricator decides", the fill note with tools |
| Order notes | new `name-order-notes.txt`, group `Order`: what to choose, what to write, the charges that apply, each with its source. Part of the release digest. |
| Copper, mask, paste | by span and tent; a capped via keeps its pad's paste |
| Netlist | Sparksmith writes no IPC-D-356 file today, so nothing changes. The X2 `.N` attribute names the net. |

Order notes for the multi-tool under `jlcpcb-4-layer` (labels not read on the form are marked):

```
ORDER NOTES                                  release 3f9c1d20a7b45e68
JLCPCB 4-layer · 4 copper layers · 0.8 mm

On the order form
  Base material      FR-4. No HDI: every via passes through the board.
  Via size           0.15 mm hole / 0.25 mm diameter
  Via covering       Epoxy Filled & Capped       (form label not verified)

Write in the remarks
  Fill with epoxy and cap: all vias of 0.15 mm finished hole
  (drill tool T1, 4 holes, in the pads of J1 B2, B3, B10, B11).
  All other vias (tools T2, T3) are tented and not filled.

JLCPCB asks for this: "For epoxy-filled vias, please leave a note …
Alternatively, you can specify to fill all vias of a particular diameter."
  jlcpcb.com/help/article/pcb-via-covering, read 2026-11-24
```

For PCBWay the same board reads: tick **Via in pad** under Customized Services and Advanced Options; do not tick **All vias filled with resin and capped**; Via process **Tenting vias**; the same remark. Whether ticking Via in pad alone fills exactly those vias is my reading of its via-in-pad page. PCBWay is not offered for this board until its smallest filled hole is confirmed.

### Refuse, never drop

- Not made or not buildable: never waivable. Export writes nothing.
- `fabrication::representable` stays the last gate: what the files cannot hold is refused, whatever was waived.
- A slice that adds a kind of drill adds its files in the same slice, or `representable` refuses it until they come.
- Importing a via the engine lacks names it. A blind via never becomes a through via.

## 8. In the window

The design system, tokens and kit are used as they are. Colours are the layer colours of `palette.rs`.

### The drill view in the stackup window

A tab beside the layer table.

- **Cross-section.** Layers top to bottom, foils in their colours, core and prepreg in two tints with thickness.
- **Barrels.** Each drill is a barrel across its layers. The upper half has its first layer's colour, the lower half its last layer's. Lands are ticks on every layer it joins. Laser barrels taper. A filled core is grey with caps. A back drill shows the wider cut.
- **Add and remove.** Drag from one layer to another to add. Drag an end to change it. Drag a barrel out, or Delete, to remove. Arrow keys and the list do the same, so no drag is required. "Both sides" adds the mirror. On release between adjacent layers a small choice asks mechanical or laser, when both can be built.
- **While dragging** the barrel shows what it would add, or turns red with the reason under the picture. A drop that cannot be built is refused.
- **Templates:** Through only, Blind top, 1+N+1, 2+N+2, Buried core. A template that needs another dielectric order (2+N+2 on the preset) shows the stackup change first and applies both as one undo step. It never invents a thickness: it moves the board's own layers, and the profile's window check then speaks.
- **Build order:** the build with the fewest pressings, marked "a possible build; the fabricator decides": a step list and a strip that plays the steps; with reduced motion the steps stand side by side.
- **Effort:** the table of section 4.
- **Not stated:** the HDI limits this board uses and the fabricator does not publish, each with the designer's value if one is set, and the waiver.
- **List:** drill, process, step, vias on the board counted, fills in use, verdict with the source on hover. Unused drills are dashed and marked.
- **Undo:** the window edits a draft, as it does today (`StackupDraft`). OK is one `layout.edit_stackup`.

### Layout: the mark on a via

- **Ring:** the focused layer's colour where the via joins it, as today.
- **Hole:** a through via keeps today's dark disc. Any other is split: upper half its first layer's colour, lower half its last layer's, the same as its barrel.
- **Laser:** a notched ring, so it does not depend on colour.
- **Capped:** a copper-coloured hole with a cross.
- **On a layer the via does not reach:** a thin dashed outline.
- **Label:** above 24 px across, "L2–L5" in the drill files' numbers. L2 is Inner 1.
- **Hover:** "Via on LANE_P · through, filled and capped · 0.15 / 0.25 mm".

The Vias table gains Fill, Process and Drill columns in its filter. The view popover's Vias row opens into one toggle per drill in use.

### Cards

- **Via card:** Drill (a choice when the stackup has others), Fill, Tent top, Tent bottom, Ring, Depth ratio. What the profile does not make is listed and disabled with one reason.
- **Via tool and Route cards:** "Via type" lists the drills that join both layers, are made, and are allowed by the class. Default: the net's last choice, else the class's first, else Through, else the shallowest. The tool never builds a stack by itself.
- **Net class:** "Vias: any / none / only…", "At most", "Layers".

### Issues wording

Each at most 80 characters, naming objects:

- "Laser L2–L3: JLCPCB makes build-up drills to order 1 on this board"
- "Mechanical L1–L3 crosses Buried L2–L5"
- "Via on LANE_P in J1 B2 is open under solder paste"
- "Filled and open vias share 0.15 mm; JLCPCB is told by hole size"
- "Via on LANE_N: net class B-row allows 1 via, this is the 2nd"
- "Back drill L8 to L5: JLCPCB does not state it with HDI"
- "JLCPCB states no unplated hole clearance for HDI"

### 3D

Barrels, lands, caps and fill bodies come from the plan, so 2D, live 3D and the exact board agree. The Vias toggle splits by drill. There is no section plane today; the cut view is the layer peel and X-ray.

## 9. Commands and file format

| Entry | Module | Does |
| --- | --- | --- |
| `layout.add_via`, `layout.route_*` | layout | take `span`, `fill`, `tent` |
| `layout.via_set { ids, span?, fill?, tent? }` | layout | one undo step, refused whole; replaces `layout.via_tent` |
| `layout.edit_stackup` | layout | carries `drills` |
| `stackup.drill_plan { drills? }` | layout | query: stages, step of each drill, effort, causes, verdicts and reasons; for the board or a proposal. The window and an agent read the same answer. |
| `rules.*` class edits | rules | `vias`, `layers` |

File version 1 is edited in place, as before the first release: `Via.fill`, `Via.tent`, later `Via.back`, `Stackup.drills`, the class fields, the profile lists. All but `tent` are left out when empty. Module minor versions rise; `cargo xtask surface --update` records them.

## 10. Checks and tests

**Independent oracle.** `fabrication/tests/oracle.rs` decides point by point from the document. It gains a per-layer rule written without `Via::on`.

**Inventory per drill.** `tests/inventory.rs` reads every drill file back and groups holes by `.FileFunction`. For each drill: the same count, positions within 1 µm, the same finished size, the same fill. Then the reverse: every hole belongs to one via of exactly that drill. All plated files together are the board's plated holes, each once.

**The build.** The prototype's 15 hand-worked cases become the first tests: through, the founder's example, 1+4+1 with and without a buried drill, 2+N+2, a core that is cut, two drills that cross, a laser through a core, a foil between prepregs. In the product a second, brute-force implementation over all stage sets checks every set of up to four drills on boards up to 8 layers, and two properties hold: removing a drill never adds effort, and every drill has exactly one step. Neither property was tested on the prototype beyond the cases.

**Seeded faults.** Each must be reported:

1. A blind via written to the through file.
2. A wrong span: `Plated,1,3,Blind` for L1–L2, and L1–L2 written as L2–L3.
3. A via missing from its drill's file, and one present in two.
4. `VII` lost from a filled tool; put on an open tool; filled and open merged.
5. A via land outside its span; one missing inside it.
6. A tented face opened; an open face covered.
7. `Blind` and `Buried` swapped.
8. The order notes naming the wrong tool or count, or a charge whose condition does not hold.
9. The job's `ViaProtection` missing a value a tool carries, or listing one no tool has.
10. A slab drilled outside the span in the plan.
11. A back drill written as plated, or stopping one layer late (later).
12. A not-stated rule resolved to the conventional value or to zero; its finding missing, or raised for a rule the board does not use.

## 11. Slices

Each slice is useful alone and can refuse what it cannot make. The last column is what is lost if blind and buried vias never come.

| Slice | Contents | Hours | Lost without blind or buried |
| --- | --- | --- | --- |
| **S1a: through, filled and capped, in the files** | `fill`; capped in the JLCPCB profiles with limits and charges; fix F1; tool key; `VII`; job; drawing note; order notes; in-pad, told-by and not-made findings; `layout.via_set` for fill; oracle, faults 4, 8, 9. Closes G4c for an agent. | 8–11 | nothing |
| **S2: net-class via and layer rules** | `vias`, `layers`, refusals, `board.via_class`, card rows. Closes G5's via part. | 5–8 | the list of drills reduces to through or none |
| **S1b: the same in the window and 3D** | Fill on the cards, capped mark, hover, filter; cap and fill bodies with parity | 7–10 | nothing |
| **S3: drills on the stackup and the build** | `Stackup.drills`, restack, derivation, `stackup.drill_plan`, effort and causes, `board.drill_build`. Vias still through only. | 12–16 | most; the pressings count and step list remain for plain multilayers and back drills |
| **S4: files and checks per drill** | vias on drills, span-aware hole checks, copper and mask by span, drill files, map, table, drawing, fabcheck, inventory, faults 1–3, 5–7 | 18–24 | nearly all; the mask and fabcheck corrections remain |
| **S5: what the profiles can make** | `drill_rules`, Build option that follows the board, HDI cases with not-stated limits, refusals and designer's values, fault 12, `LaserHole`, stacks, verdicts, charges | 17–23 | all but charges and verdicts |
| **S6: the drill view** | cross-section, drag, templates, build strip, effort, list | 16–22 | nearly all; a static section remains useful |
| **S7: routing with drills** | Route and pair tools, shove, `via_pitch`, reasons | 8–12 | all |
| **S8: plan, live and exact 3D** | slabs, barrels, fault 10, parity | 8–11 | all, unless back drills come |
| **S9: marks and toggles in Layout** | split hole, notched ring, toggles | 6–9 | all but the capped mark, which is in S1b |
| **S10: back-drilling** | section 5, fault 11. Needs S3 and the file part of S4. | 14–20 | nothing; it serves through vias |
| **S11: tents per side, plugged, import, pad vias, F2** | as named | 7–10 | the span part of the import |

Order: S1a, S2, S1b for the multi-tool (20–29 hours). S3 to S9 are 85–117 hours together, and blind and buried are orderable after S5. Until S8 the 3D view and the exact export refuse such a board, as they do today; Layout draws it. Back-drilling without blind and buried costs about 14 to 20 hours more than S10 alone, for its share of S3, S4 and S8.

## 12. Founder decisions, open

1. **Which rows of JLCPCB's shared capabilities page count as stated for HDI?** Mask, legend and outline rows name no build. *Recommended:* count a row that names no build and no layer count as stated for every build on that page, and nothing else. Until decided: none count.
2. **JLCPCB's smallest filled hole.** Its pages say 0.15 mm, 0.2 mm and 0.2 mm. The profile says, as data: 0.2 to 0.5 mm is made (every page), 0.15 to 0.2 and 0.5 to 0.55 mm is not stated (the pages disagree), anything else is not made. Open: whether to keep that or take one page. Ask JLCPCB before the order.
3. **From 6 layers JLCPCB fills every via by default.** *Recommended:* a finding on such boards.
4. **Skip laser drills (L1–L3).** JLCPCB makes them as stacks. *Recommended:* refuse under JLCPCB and offer the two.
5. **How far to go now.** Decided: S1a, S2 and S1b.
6. **F1 in S1a, F2 in S11.** *Recommended:* yes; F1 is in every plated drill file today.

## 13. Not verified

- IPC-4761 itself. Names only, from Ucamco and PCBWay. A copy of the standard that a fabricator hosts publicly was downloaded by mistake while looking for a summary; only its front matter was read, nothing here comes from it, and it was deleted.
- JLCPCB's order form labels for via covering and HDI, its charges, and its answer for 0.15 mm filled on 4 layers.
- The JLCPCB quote page's charge sentence was read by an earlier agent (`docs/design/usb-c-multitool.md`, 2026-11-23) and found again in the page source on 2026-11-24.
- Whether either fabricator reads `ViaDrill,VII`, or fills only some vias without a remark.
- The stage rules are physics and PCBWay's published rules. No fabricator was asked whether they match its lines. One-sided build-ups are derived as possible; JLCPCB publishes none.
- The HDI order count is a reading of JLCPCB's text.
- Whether JLCPCB combines back-drilling with HDI.
- PCBWay's laser ring, spacing, depth ratio and stacking limits. The stagger offset anywhere.
- Altium's program was not run. Its documentation was read in parts.
- The XNC subset was not compared with today's whole Excellon header.
- The derivation prototype passes 15 hand-worked cases. No product code was built or run.

## Sources

All read 2026-11-24 unless noted.

- Altium, public documentation: [Defining the Layer Stack][alt1], [Blind, Buried & Micro Via Definition][alt2], [Controlled Depth Drilling][alt3].
- Ucamco, Gerber Layer Format Specification, revision 2026.05 ([PDF][gerber]): pp. 142, 144, 151, 153, 172, 214.
- Ucamco, XNC Format Specification, revision 2021.11 ([PDF][xnc]): pp. 17–18.
- Ucamco, Gerber Job Format Specification, revision 2020.08 ([PDF][job]): p. 8, and the [JSON schema][schema].

[gerber]: https://www.ucamco.com/files/downloads/file_en/554/gerber-layer-format-specification-revision-2026-05_en.pdf
[xnc]: https://www.ucamco.com/files/downloads/file_en/452/xnc-format-specification-revision-2021-11_en.pdf
[job]: https://www.ucamco.com/files/downloads/file_en/435/gerber-job-format-specification-revision-2020-08_en.pdf
[schema]: https://www.ucamco.com/files/downloads/file_en/397/gerber-job-file-schema_en.json
[jcap]: https://jlcpcb.com/capabilities/pcb-capabilities
[jhdi]: https://jlcpcb.com/pcb-fabrication/hdi-pcb
[jhelp]: https://jlcpcb.com/help/article/hdi-pcb-capabilities-faq
[jcov]: https://jlcpcb.com/help/article/pcb-via-covering
[jnews]: https://jlcpcb.com/news/free-via-in-pad-6-20-layer-pcbs-pofv
[jblog]: https://jlcpcb.com/blog/via-in-pad-design-deep-dive
[jname]: https://jlcpcb.com/help/article/suggested-naming-patterns
[pcap]: https://www.pcbway.com/capabilities.html
[padv]: https://www.pcbway.com/advanced-pcb-capabilities.html
[phdi]: https://www.pcbway.com/hdi-pcb.html
[porder]: https://www.pcbway.com/orderonline.aspx
[pcov]: https://www.pcbway.com/pcb_prototype/PCB_Via_Covering.html
[pvip]: https://www.pcbway.com/pcb_prototype/PCB_Via_in_Pad.html
[pblind]: https://www.pcbway.com/pcb_prototype/Blind_vias_and_Buried_Vias.html
[pbd]: https://www.pcbway.com/pcb_prototype/PCB_Backdrill.html
[jextra]: https://jlcpcb.com/help/article/in-what-cases-will-there-be-charged-extra
[alt1]: https://www.altium.com/documentation/altium-designer/pcb/defining-layer-stack
[alt2]: https://www.altium.com/documentation/altium-designer/pcb/blind-buried-micro-vias
[alt3]: https://www.altium.com/documentation/altium-designer/pcb/high-speed-design/configuring-stackup-controlled-depth-drilling-back-drilling
