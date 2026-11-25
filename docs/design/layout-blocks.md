# Layout blocks: parts that belong together, with what the datasheet says about them

**Status:** design for founder review, 25 November 2026. Nothing here is built. Hours are agent hours, estimated. Code references are to integ-9 at 0f38f98d. Pictures: [mock-up](floorplan-mockup.html), pictures 1 to 4 (a mock-up: invented parts and documents). Related: [floorplan](autorouter.md), [repeated sheets](repeated-sheets.md), [part dimensions](../board-sketch-references.md), [copper graph](../copper-graph.md).

**Founder (25 November 2026):** "lets say U1 is a DC/DC converter: maybe it is not super important where U1 exactly is, but all relevant parts that need to stay close to it (inductor, caps, with the recommended layout from the datasheet). so basically this is like a module, and maybe we should have that concept. … these modules might have infos from the datasheet attached to them in a structured way to show if it is the recommended layout." And: "we could point the user to where this might be wrong when he misses something important". He wants "a functional border" and to "show the level of detail we want". He approved writing this design and extending the mock-up. **Everything else below is a proposal**, including the name.

## 1. What the person sees

1. **Select U1, L1 and the capacitors, choose Make block.** A dash-dot border appears around them, with a name. Dragging the border moves everything in it; one part can still be dragged alone. The border is soft: other parts and traces may enter it.
2. **Level of detail.** Far out a block is its border and name; nearer its parts; nearer still pads and copper. By zoom, or fixed by hand. Parts outside blocks are always drawn.
3. **Lock** keeps the block where it is: no drag, no arranging, no planner moves it or one part of it.
4. **Parts not on the board yet** show as a border that reserves room, dragged like any other.
5. **If U1's library part has a recipe**, the block's card lists its roles (which part is the input capacitor), what the document says, each with document and page, and hints: a missing part, the input loop compared with the example. **Arrange as in the datasheet** moves the parts in one undo step.
6. A block made by hand has no recipe. It moves, locks and shows like the others, and nothing is compared.

Nothing about a block ever blocks placing, routing, publishing or export.

## 2. What datasheets really say

Five documents of different kinds were read on these pages, text and figures, and a sixth (Microchip AN2648D, p. 7) to compare with the third. Quotes are short; no figure was copied.

| Document | Kind | Pages |
|---|---|---|
| TI LMR33630, SNVSAN3F (Nov 2020) | Buck converter | 23, 33–36 |
| TI TPS7A20, SBVS338H (July 2024) | LDO | 3, 27, 31 |
| TI SLAA322D (July 2017) | 32 kHz crystal at an MCU | 8–9 |
| TI TPD2EUSB30, SLVSAC2G (June 2021) | USB ESD protection | 11–12 |
| Raspberry Pi, Hardware design with RP2040 (24 Aug 2026 release; printed pages) | MCU decoupling | 7–8, 11 |

| Statement kind | Where it occurs | Carries numbers? |
|---|---|---|
| **Required part**, with pins and value | LMR p. 23: "A high-quality ceramic capacitor of 100 nF and at least 10 V is required" between BOOT and SW; 10 µF and 220 nF at the input. RP p. 7: 100 nF "per power pin"; p. 11: 27 Ω in series with each USB line | **Yes**, always |
| **Recommended part** (not required) | TPS p. 27: "the LDO itself is stable without an input capacitor"; p. 31 still recommends 1 µF | Yes |
| **Near** (these parts at these pins) | All five. LMR p. 33 "as close as possible"; TPS p. 3 and 31; RP p. 7 "close to the power pins" | **Never a distance** |
| **What matters most** | Only LMR p. 33: "the most critical PCB feature is the loop formed by the input capacitor or input capacitors, and power ground". Its list is numbered but does not say the numbers are a ranking | No |
| **Loop** (a path through named parts) | LMR p. 33 "the loop area as small as possible"; SLAA p. 8 "crystal to CL1 to CL2 to crystal" | No |
| **On the way** (a part between two others) | TPD p. 11: the protected traces "between the TVS and the connector" are straight; RP p. 11: 27 Ω "close to the chip" | No |
| **Keep away** | LMR p. 33 item 4 (feedback line and the SW node), SLAA p. 8 (digital lines; opposite side under the crystal), AN2648 p. 7, TPD p. 11 | **No distance in any of them** |
| **Symmetry, grouping** | SLAA p. 8 "use a symmetric layout" | No |
| **Copper and heat** | LMR p. 34: "4 × 3 array of 10-mil thermal vias", 2 oz / 1 oz / 1 oz / 2 oz; p. 33 wide paths, planes; TPS p. 31: no thermal via directly under the pad of one package; SLAA p. 8 guard ring | **Some** |
| **Example arrangement** | A figure in all five: LMR p. 35 and p. 36 (one per package), TPS p. 31, SLAA p. 9, TPD p. 12, RP p. 8 | The placement is **not dimensioned** in any of them |

So three things carry numbers: part values, some copper and heat statements, and nothing about distance. That settles the rule: **a distance is never a limit here, because no source gives one.**

**Where a model of statements fails, and what is done about it:**

- **The shape of copper.** LMR item 9 wants the switch node "as short and wide as possible" and at the same time its area "minimized". No single number says that. Shown as text, not checked.
- **Heat and the stackup.** "Enough copper area" depends on current and temperature. Shown as text. A via array with a count could be checked later, as its own kind.
- **The other side of the statement is another block.** The ESD part's statement is about the connector. A statement may name a role that the person fills with a part of another block.
- **Conditions.** SLAA p. 8 wants a 5.1 MΩ resistor only below 2.5 V and only for one family; LMR has two figures for two packages. A statement may be limited to packages. Any other condition stays text, and the part is then not "required".
- **Documents depart from themselves.** RP p. 8 shares one capacitor between two pins for lack of room, and says why. A count that differs is therefore a warning a person can waive, never an error.
- **What is only in the figure.** TPS has four short bullets; the rest is the drawing, and that drawing shows its capacitors as schematic symbols, so it has no size at all. RP's is a picture of the finished board. A recipe can say which part is on which side of which pins, not more.
- **More than one loop.** LMR p. 36 splits the input capacitors to both sides of the second package: two loops, and other roles than p. 35. Roles and loops may be limited to packages too.

## 3. Data

**In the project** (format version 1 edited in place; empty is not written):

| Type | Holds |
|---|---|
| `Block` | id, name, members, `locked`, side, optional `recipe` (a published revision), `roles` (role → members, each *matched* or *chosen*), optional `room` |
| `room` | An outline with a place. Only while no member is on the board; placing the first one removes it. So a block never has two borders |

The border of a placed block is **derived**: the hull of its members' courtyards. No margin is added; one would be an invented number. A part is in at most one block. Blocks do not nest in the first version.

**In the library:** a new definition kind, `layout_recipe`, published in revisions like a package row (decision 2). It names one component lineage and holds:

- `citation`: the existing `RowCitation` (document, revision, page; `crates/editor-core/src/library/package_row.rs:58`), with "checked" meaning revision and page are there. Each statement has its own page and an optional short quote (proposed: at most 200 characters).
- `roles`: a name, how it is recognised (section 4), and whether the part is `required`, `recommended` or `optional`, with the value as printed.
- `statements`: one of *near*, *loop*, *on the way*, *keep away*, *text*, each over roles and terminals. `most_critical` marks one the document itself ranks so.
- `arrangements`: per package set, the side, position and rotation of each role relative to the anchor. **Drawn by us** with our own footprints after the figure's topology (which part on which side of which pins). No manufacturer drawing is stored or traced.
- `approved`: by whom and when, for this revision; absent means not approved.

A part update changes nothing by itself. A newer recipe revision is offered like any update, with the hints before and after.

## 4. Matching roles

Deterministic, from the schematic's connectivity and declared facts only: nothing from designators, names or distance.

1. The anchor is an instance of the recipe's component.
2. A role says *a two-terminal part with a capacitance (or resistance, inductance) field between these nets*, where a net is given by a terminal of the anchor or of an already matched role. The fields are the library's standard fields.
3. The most specific roles first (BOOT to SW before VIN to GND). A part already matched is not used again.
4. Exactly the wanted number: matched. Fewer: *missing*. **More: ambiguous, and the person is asked.** On a shared rail every bypass capacitor of the board fits "VDD to GND", so this is the usual case for decoupling. The nearest one is not assumed. What the person picks is stored as *chosen* and is kept until its connection changes.
5. Values are not used to match, only shown. Where a document gives two roles between the same pins (LMR: 10 µF and 220 nF), the person is asked.

## 5. What is measured

**Loop length.** A loop is an ordered list of pads, for example U1.VIN → C1.1 → C1.2 → U1.GND and back.

- *Before routing:* the length of the closed line over the pad centres, in the board's plane, in millimetres.
- *After routing:* between two pads of one net, the shortest copper path the copper graph finds (`NetPaths::path`, `layout/connectivity/lengths.rs:530`): track centre lines plus via barrels. Inside a part the straight line between its two pads. A hop through a pour has **no length in the copper graph** (`lengths.rs:40`); that hop is measured straight from pad to pad and the result is marked "partly in a pour".
- The **ratio** is this length divided by the same length on the recipe's arrangement: before routing against pads, after routing still against pads, and it says which. It is shown as a number. No value is good or bad, there is no colour and no threshold.
- It is shown only if every part of the loop has the package the arrangement was drawn with (decision 3), is on the side the arrangement says, and the arrangement exists. Otherwise only the millimetres, with the reason.

Length and not enclosed area, because the area of a loop whose pads lie in a line is zero and a ratio of areas then means nothing. The area is shown beside it.

**Keep away** has no distance, so only a fact is reported: which nets of the other kind (by net profile, where one exists) have copper inside the block's border, per layer. **On the way:** whether the copper path from the connector pad to the chip pad passes the protecting part's pad.

**Not measured:** inductance, the plane under the loop (which largely decides it), current, heat, width and shape of copper, coupling, symmetry. The card says "length only".

## 6. Hints

| Hint | Wording | Where |
|---|---|---|
| A required part is missing | "No capacitor between U1.BOOT and U1.SW (SNVSAN3F, page 23)" | **Issues, warning**, `block.part_missing`; waivable with a reason, like any warning. Not in the production check's blocking list |
| A recommended part is missing | "TPS7A20 recommends a capacitor at IN (SBVS338H, page 27)" | Card, advice |
| Ambiguous | "4 capacitors fit U2.VDD; choose which are its own" | Card, to do |
| Placement | "Input loop: 3.1 times the example's, page 35" | Card, advice |
| Routing (last slice) | "USB_DP reaches U3 before D1" | Card, advice |
| Not approved | "Recipe not approved: check it against the document" | Card; its hints stay out of Issues (decision 5) |

Each opens the statement with its quote and page. The waiver key is the block, the role and the recipe's lineage, so it survives a moved part and ends when the recipe asks for something else.

## 7. Arrange as in the datasheet

One undo step, one `layout.block_arrange`. The anchor stays; the other roles go to the arrangement's places, rotated with the anchor. Parts only: copper follows by the existing rules of a part move, and what cannot follow is listed first, in the existing impact review. A locked or held part is not moved; the review names it. Where a part does not fit (another part, the board edge), it stays and is named. Arranging is refused while the block is locked. If the arrangement was drawn with other packages, the button says so and is off.

## 8. Together with what exists

- **Selection groups** are a moment's selection (`pcb_editor/group.rs`). A block is kept. Dragging a block is the existing group move, with the members as the group: a held or locked member refuses the drag and is named, as today.
- **Dimensions and holds** win. A block adds no dimension. A part held to an edge keeps its hold, and Arrange skips it. `block.locked` does not set the parts' own locks; a move of one part names the block ("U1 is in the locked block Converter").
- **Repeated sheets.** If all members are in one sheet definition, the block is stored there, once, with the members' local ids; each use of the sheet (`SheetPath`) has its own block on the board, with its own place and lock. The recipe and the roles are the definition's. "Arrange B like A" copies A's relative places and is one step; it comes after the first slices. A block whose members span sheets is stored with the instances.
- **The floorplan** ([autorouter](autorouter.md)). A block may be a member of an area. The planner gives a rough position to the block; the members' places inside come from the arrangement, else from the constraint placer. The placement intent `group` becomes `block`. `room` takes part in the *area fill* note. A distance in `near(…, ≤ d)` comes only from the person or a cited rule. The router does not see blocks. In the peeling view and the hulls the border is drawn on its side, up to the highest member.
- **Copper** belongs to nets, not to blocks. A block does not own tracks.

## 9. The assistant's part

Later, and always reviewed. From a datasheet the assistant proposes roles and statements, each with page and quote, through the existing import review (rows unreviewed until a person confirms). It never proposes a distance the document does not state, never an arrangement in coordinates, and cannot approve. It may explain a hint and propose matches for the ambiguous ones; the person answers.

## 10. Slices

| Slice | What | Useful alone for | Hours |
|---|---|---|---|
| **B1** | Blocks by hand: model, operations, border, drag, lock, level of detail, card, undo, 2D | Moving a converter as one thing; a calm view of a full board | 18–26 |
| **B2** | Recipe kind with statements and citations, editor, approval; publishing | Written layout notes with page numbers beside the part | 16–24 |
| **B3** | Matching, the question for ambiguous roles, "missing part" in Issues, one-click block from a recipe | Finding a forgotten bootstrap capacitor | 14–20 |
| **B4** | Loop length, ratio, drawing of the loop | Seeing which loop is long | 8–12 |
| **B5** | Arrangements (drawing one in the recipe editor) and Arrange | A converter placed in one step | 16–24 |
| **B6** | Floorplan and repeated sheets: room, block intents, one block per use, "like A" | Needs the floorplan's P2 and P5 | 12–18 |
| **B7** | Border in the peeling view and hulls | Enclosure checks by block | 4–6 |
| **B8** | Routing advice: on the way, keep away | Review of a routed board | 10–16 |
| **B9** | Recipe drafts by the assistant | Filling the library | 10–16 |

B1 alone answers the first half of the founder's words; B1 to B3 the second. Together 108 to 162 hours; B1 to B3 48 to 70.

## 11. What it will not do

Judge a layout as good; give a distance; simulate; check heat, inductance or EMI; stop anything; route; copy a manufacturer's figure; read other tools' sources or libraries; guess roles from names; own copper; nest blocks.

## 12. Open decisions

1. **The name.** *What it is:* the word in the window, the file and the commands. *Today:* "module" is used for generators and plug-ins; the glossary has "Title block"; pin layouts of symbols are called block in the code; findings "block" things, as a verb. *Options:* "block" (short, what engineers say; the verb can read oddly in Issues); "layout block" everywhere (clear, long on a small card); "cluster" (free in the glossary, says less about function). *Recommendation:* "block" on the canvas and the card, "layout block" in Issues, help and the glossary.
2. **Where a recipe lives.** *What it is:* inside the component, or a definition of its own. *Today:* all facts of a part are in its component revision; a changed component is a new revision, offered to every project that uses it. *Options:* (a) in the component: one place, but correcting a page number makes a new part revision and an update for every board; (b) its own kind that names the component: corrected and approved alone, several per component are possible (one per document), one more kind in the library. *Recommendation:* (b), shown on the part's page. It departs from "on the library part" in storage only.
3. **The yardstick.** *What it is:* what the loop is divided by. *Today:* nothing. The figures are not dimensioned, so "the example's length" is the length on **our drawing** of it, with our footprints. *Options:* (a) only with the packages it was drawn with, else millimetres without a ratio: honest, and often no ratio, since capacitor sizes vary; (b) recompute the arrangement with the person's packages: nearly always a ratio, but the yardstick then moves with the board and is Sparksmith's more than the datasheet's; (c) no ratio, millimetres only. *Recommendation:* (a) first, and the card says "our drawing after page 35".
4. **Order of importance.** *What it is:* the order of the statements on the card. *Today:* nothing. Of the five documents one calls one feature the most critical; none ranks the rest. *Options:* (a) document order, with "most critical" only where the document says it; (b) the recipe's author ranks, marked as the author's; (c) no order. *Recommendation:* (b) with the mark, because a novice asks "what first?" and the answer must not look like the manufacturer's.
5. **What a recipe may do before a person approved it, and who that is.** *What it is:* whether a wrong recipe can raise warnings. *Today:* import rows stay unreviewed until a person confirms; an agent can clear the flag only after that. *Options:* (a) unapproved recipes show hints on the card only, marked; a person approves a revision once for the library folder, and every project sees it; (b) each project approves again: safer against a careless library, tedious; (c) unapproved ones show nothing: the first look at a draft is then impossible. *Recommendation:* (a).
6. **The first recipes.** *What it is:* whether Sparksmith ships any. *Today:* the shipped library has generic families and few specific parts. *Options:* (a) ship none: the feature is empty until a person or the assistant writes one; (b) we write a handful by hand, for parts of the example board, facts and page numbers only, our own drawings: shows the feature and tests the model, and each must be checked against its document by a person; (c) ship generic recipes ("any buck converter"): no document stands behind them, which breaks the no-invented-limits rule. *Recommendation:* (b), three to five, after B2; a short legal look at quoting single sentences first, which was not done here.
