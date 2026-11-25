# Layout blocks: parts that belong together, with what the datasheet says about them

**Status:** design of 25 November 2026. The decisions of section 12 are the founder's of that day. Slice B1 is built; nothing else here is. Section 13 is the schematic side. Hours are agent hours, estimated. Code references are to integ-9 at 0f38f98d. Pictures: [mock-up](floorplan-mockup.html), pictures 1 to 4 and 7 (a mock-up: invented parts and documents). Related: [floorplan](autorouter.md), [repeated sheets](repeated-sheets.md), [part dimensions](../board-sketch-references.md), [copper graph](../copper-graph.md).

**Founder (25 November 2026):** "lets say U1 is a DC/DC converter: maybe it is not super important where U1 exactly is, but all relevant parts that need to stay close to it (inductor, caps, with the recommended layout from the datasheet). so basically this is like a module, and maybe we should have that concept. … these modules might have infos from the datasheet attached to them in a structured way to show if it is the recommended layout." And: "we could point the user to where this might be wrong when he misses something important". He wants "a functional border" and to "show the level of detail we want". On this design he decided section 12 ("yes to all six") and asked for one addition: "I would like to see more info on hover over an area or especially over a part (maybe I dont remember what U1 is, so it would be great to show the part number, or for resistors the value, etc)". What is neither quoted here nor in section 12 is the design proposed with it.

## 1. What the person sees

1. **Select U1, L1 and the capacitors, choose Make block.** A dash-dot border appears around them, with a name. Dragging the border moves everything in it; one part can still be dragged alone. The border is soft: other parts and traces may enter it.
2. **Level of detail.** Far out a block is its border and name; nearer its parts; nearer still pads and copper. By zoom, or fixed by hand. Parts outside blocks are always drawn.
3. **Lock** keeps the block where it is: no drag, no arranging, no planner moves it or one part of it.
4. **Parts not on the board yet** show as a border that reserves room, dragged like any other.
5. **If U1's library part has a recipe**, the block's card lists its roles (which part is the input capacitor), what the document says, each with document and page, and hints: a missing part, the input loop compared with the example. **Arrange as in the datasheet** moves the parts in one undo step.
6. A block made by hand has no recipe. It moves, locks and shows like the others, and nothing is compared.

7. **Hover or focus anything and a small card says what it is.** It appears at once, stands beside the thing and never over it, has the same size at every zoom, and Esc closes it. The keyboard reaches it: focus shows the same card.

| On | The card shows | From |
|---|---|---|
| Part | Designator and, as title, the part number or the value with package ("C21 · 10 µF, 25 V · 0805"); what it is in words; block and role; height; side; "rough position" while it is not on the board | The instance's Part, else `instances::value_text` (the one value source of schematic, BOM and placement file, `instances.rs:545`); the component's name; the package's name and height; the block; the floorplan |
| Area | Name, purpose sentence, use, layer, size, the parts inside, the lanes through it with their nets | The floorplan |
| Lane | Nets, planned and needed width, reference layer | The floorplan, the net classes |
| Block | Its parts with roles, the recipe and whether it is approved, the number of hints | The block and its recipe |
| Edge, corner | What it divides or joins; whether it is locked | The floorplan |
| Via, track, pad | What the via line says today, as rows; a track's net, width and layer; a pad's number, net and function | The board |

A value nothing states is left out, not shown as a guess. The card never holds a button: acting is in Properties.

**It is one card** for the floorplan, Layout, PCB 3D and Schematic: one function from an object to its rows, one drawing in `kit`. **Today** there is no such thing: in Layout, hovering a part lights its row in the parts table and draws its dimensions, with no text (`pcb_editor.rs:2253`, `parts_table.rs:959`); a via has a text line as a pointer tooltip (`pcb_editor.rs:2260`, `via_card.rs:171`) and a drawn dimension says what it measures between (`dimensions/draw.rs:298`); PCB 3D lights the row (`parts_table.rs:922`); in Schematic a pin gets a ring and a wire segment is lit, without text (`schematic_editor.rs:2420`, `2492`). **What changes:** the via line and the dimension's text become cards and their tooltips go; the lights, the ring and the drawn dimensions stay. The floorplan's rationale on hover ([autorouter](autorouter.md) 3.5) is in this card too. While something is dragged, a menu is open or a tool waits for a click on an object, the card stays away.

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
| `Block` | id, name, members, `locked` (these are B1); optional `recipe` (a published revision), `roles` (role → members, each *matched* or *chosen*), optional `room` |
| `room` | An outline with a place and a side. Only while no member is on the board; placing the first one removes it. So a block never has two borders |

The border of a placed block is **derived**: the hull of its members' courtyards. No margin is added; one would be an invented number. A block is on the sides its parts are on, so a placed block stores no side. A part is in at most one block. Blocks do not nest in the first version.

**In the library:** a new definition kind, `layout_recipe`, published in revisions like a package row (decision 2) and shown on the part's page. It names one component lineage and holds:

- `citation`: the existing `RowCitation` (document, revision, page; `crates/editor-core/src/library/package_row.rs:58`), with "checked" meaning revision and page are there. Each statement has its own page and an optional short quote (at most 200 characters).
- `roles`: a name, how it is recognised (section 4), and whether the part is `required`, `recommended` or `optional`, with the value as printed.
- `statements`: one of *near*, *loop*, *on the way*, *keep away*, *text*, each over roles and terminals. `most_critical` marks one the document itself ranks so; an optional `rank` is the author's and is shown as theirs.
- `arrangements`: per package set, the side, position and rotation of each role relative to the anchor. **Drawn by us** with our own footprints after the figure's topology (which part on which side of which pins). No manufacturer drawing is stored or traced.
- `approved`: by whom and when, for this revision, once for the library folder; absent means not approved.

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
| Placement | "Input loop: 3.1 times the example's, our drawing after page 35" | Card, advice |
| Routing (last slice) | "USB_DP reaches U3 before D1" | Card, advice |
| Not approved | "Recipe not approved: check it against the document" | Card; its hints stay out of Issues (decision 5) |

Each opens the statement with its quote and page. The waiver key is the block, the role and the recipe's lineage, so it survives a moved part and ends when the recipe asks for something else.

## 7. Arrange as in the datasheet

One undo step, one `block.board_arrange`. The anchor stays; the other roles go to the arrangement's places, rotated with the anchor. Parts only: copper follows by the existing rules of a part move, and what cannot follow is listed first, in the existing impact review. A locked or held part is not moved; the review names it. Where a part does not fit (another part, the board edge), it stays and is named. Arranging is refused while the block is locked. If the arrangement was drawn with other packages, the button says so and is off.

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
| **H** | The hover card: parts, vias, tracks and pads in Layout, PCB 3D and Schematic; the floorplan's and the blocks' rows come with their slices | Knowing what U1 is without selecting it | 8–12 |
| **B1** | Blocks by hand: model, operations, border, drag, lock, level of detail, card, undo, 2D | Moving a converter as one thing; a calm view of a full board | 18–26 |
| **B2** | Recipe kind with statements and citations, editor, approval; publishing | Written layout notes with page numbers beside the part | 16–24 |
| **B3** | Matching, the question for ambiguous roles, "missing part" in Issues, one-click block from a recipe | Finding a forgotten bootstrap capacitor | 14–20 |
| **B4** | Loop length, ratio, drawing of the loop | Seeing which loop is long | 8–12 |
| **B5** | Arrangements (drawing one in the recipe editor) and Arrange | A converter placed in one step | 16–24 |
| **B6** | Floorplan and repeated sheets: room, block intents, one block per use, "like A" | Needs the floorplan's P2 and P5 | 12–18 |
| **B7** | Border in the peeling view and hulls | Enclosure checks by block | 4–6 |
| **B8** | Routing advice: on the way, keep away | Review of a routed board | 10–16 |
| **B9** | Recipe drafts by the assistant | Filling the library | 10–16 |

B1 alone answers the first half of the founder's words; B1 to B3 the second. H needs nothing else and can go first. Together 116 to 174 hours; B1 to B3 48 to 70.

## 11. What it will not do

Judge a layout as good; give a distance; simulate; check heat, inductance or EMI; stop anything; route; copy a manufacturer's figure; read other tools' sources or libraries; guess roles from names; own copper; nest blocks.

## 12. Decided (founder, 25 November 2026)

1. **Name.** "Block" on the canvas and on the card; "layout block" in Issues, the help and the glossary.
2. **A recipe is a definition of its own** that names the component, published and approved in its own revisions, and shown on the part's page. A corrected page number makes no new part revision, and a component can have several recipes, one per document.
3. **The yardstick is our drawing of the example,** with the packages it was drawn with. With other packages there is no ratio, only millimetres and the reason. The card says "our drawing after page 35".
4. **Order of importance.** The recipe's author may rank the statements and the card marks the order as the author's. "Most critical" appears only where the document itself says so.
5. **Approval.** A person approves a recipe revision once for the library folder, and every project sees it. Until then its hints show on the card only, marked, and never reach Issues.
6. **First recipes.** Three to five written by hand for parts of the example board, after slice B2: facts and page numbers, our own drawings, each checked against its document by a person. A short legal look at quoting single sentences comes first. No generic recipe without a document is shipped.
7. **Membership is explicit.** A part is in a block because a command put it there, never because of where it lies on a sheet or on the board. This holds on the schematic and on the board alike (section 13).

## 13. On the schematic

A block belongs to the whole design: the same block, with the same members, on the sheets and on the board. Membership is explicit (section 12, decision 7): a part joins or leaves only by a command (`block.add`, `block.members`, `block.delete`), never because of where it lies on a sheet or on the board. That is how B1 stores it: one list of component instances per block, in the project (`crates/editor-core/src/model/block.rs`). Nothing on the schematic side adds a second list. Pictures: [mock-up](floorplan-mockup.html), picture 7 (invented parts). Code references in this section are to integ-11 at 8eee1d5f.

### 13.1 What the person sees

1. **Make block from selected symbols.** Select symbols and choose Make block, as on the board. This is the existing `block.add` with the selected symbols' instances, so it is one undo step and MCP already has it. Selected wires and labels are ignored: only components are members.
2. **The border is drawn, not stored.** On each sheet, the border goes around the member units drawn on that sheet, with the block's name on it. It is the same hull function the board uses, here around each unit's drawn box (body, pins, texts). The board's border has no margin, because a margin would be an invented number; on the sheet the border lies one step of the sheet's own grid outside the units, so it doesn't run through pin ends. If some members are drawn on other sheets, the border says "also on sheet 3"; that text jumps to the sheet.
3. **Drag the border** and every member unit on that sheet moves, together with the wires and labels between them. This is the group move that exists today (`schematic.arrange`, `crates/editor-core/src/schematic.rs:92`): a wire with both ends in the group keeps its shape, a wire with one end in the group rubber-bands, and a move that would join two nets is refused with `net.moved_merge` (`nets.rs:847`). What comes along: the member units on the sheet, the labels and power ports whose wires lead only to members, and the wires between them. Units on other sheets do not move.
4. **Lock is the board's.** `Block.locked` holds the block on the board only (`model/block.rs:25`: "symbols are not locked"). The border of a locked block can still be dragged on the sheet. The card on the sheet says "locked on the board".
5. **Cross-probe both ways.** Selecting a border on the sheet selects the block on the board, and the other way round, through the selection the workspaces already share. A member selected alone stays a part selection. MCP's `view.select` cannot name a block today; see 13.4.
6. **Copy and paste.** Copying the border (or all of the members) copies the members with the wires and labels between them. Pasting makes new parts and a new, separate block named after the source with the next free number ("Converter 2"), in the same undo step. A wire that leaves the block is not copied. Labels and ports join by their names under the existing paste rules: power ports and global labels join project-wide, and a local label joins on its own sheet (`schematic/clipboard.rs:1`). One rule is added for a pasted block, so that it stays independent: a local label inside it whose name already exists on the target sheet gets the new block's number ("SW" becomes "SW_2"), and the paste result lists each renamed label. Ports and global labels keep joining by name. The pasted block remembers which of its parts was copied from which part of the source, so that the board can offer "Place like Converter" (item 8).
7. **Update PCB.** A part Update PCB adds that belongs to a block is placed next to that block's border on the board, wherever it fits clear of other parts, and goes to the staging area beside the board if it fits nowhere near. The review names the block of each part it adds. A placed part never moves, so a locked block never moves.
8. **"Place like Converter."** If Update PCB adds every part of a pasted block and the source block is on the board, the review offers, ticked, to place the new parts like the source: the same positions relative to each other, the same sides and rotations, beside the source block. Parts only; the copper comes later (slice S6). Unticking it stages them as usual. It never applies to a part that is locked or held, and a source part that has left the block or is not placed is not copied: those copies are staged, and the review names them.
9. **Copy versus repeated sheet.** A copy is independent: Converter 2's parts are new parts, and editing Converter later does not change them. A repeated sheet ([repeated sheets](repeated-sheets.md)) shares one drawing and has its own physical parts per use. So a block made on a repeated sheet, with all its members in that sheet's definition, is stored once in the definition, as section 8 says. Each use then has its own block on the board, with its own place and lock, and a use added later has it too. The membership is still explicit: it is the definition's, set by a command. The help says which one to choose: copy for a variation, a repeated sheet for the same circuit twice.
10. **Make block of this sheet.** On a sheet symbol, this makes a block of the parts on its child sheet at that moment. It is `block.add` with those parts, so it is explicit like any other block, and parts added to the sheet later do not join by themselves. On a repeated sheet it makes the definition's block, which every use has.

### 13.2 Checked against the code, and where the proposal does not hold

| Proposal | What the code does today | What follows |
|---|---|---|
| Make block from selected symbols | `block.add { parts }` takes instances and refuses a part that is already in a block (`instances/blocks.rs`) | Holds; no new command |
| Border per sheet, "also on sheet N" | A unit is drawn on one sheet; a multi-unit part may have units on several sheets (`symbol.place_unit` takes a sheet) | Holds. Each sheet's border is around that sheet's units only |
| Members are components only; wires and labels come along | `schematic.arrange` moves the objects it is given and rubber-bands the rest; it does not work out which labels belong to a group | **The rule for which labels and wires come along must be in the core**, otherwise the canvas and MCP compute it twice. New entry `block.sheet_move { id, sheet, by }` that builds the object list and calls the arrange code (minor raise of `schematic` and `project`) |
| Drag border moves as a group, no net joins | `schematic.arrange` is a move (`is_move`), so `refuse_moved_merges` applies | Holds |
| Lock is the board's | Lock is board-only by the B1 model | Holds |
| Cross-probe both ways | The selection is shared between workspaces for parts; `view.select` takes `ObjectRef`s, which have no block (`diagnostics.rs:152`) | Holds in the desktop. MCP needs a way to name a block (13.4) |
| Copy/paste makes new parts and a new block | `SheetClip` has no blocks (`schematic/clipboard.rs`); `paste` makes new instances but forgets which source each came from once it returns | **The clip must carry the block**: an optional `blocks` field on `SheetClip`, and the pasted `Block` stores its source pairing (an optional `copied_from`, kept until "Place like" has been done or declined). Minor raises of `schematic` and `project`; the file format is version 1, edited in place |
| Outgoing connections by today's paste rules | A wire end outside the clip becomes a free end (`clipboard.rs:256`); a local label joins on its sheet | **A local label inside the block, pasted onto the same sheet, joins the source block's net** (SW of Converter 2 would join SW of Converter). Pasting a block renames such labels (13.1, item 6). That is a change to the paste rule for blocks only |
| Copy of a part with units on two sheets | A copy holds only the units on the copied sheet (`copy`, `clipboard.rs:183`) | The pasted part's other units are left unplaced. The paste result names them, and the new block's card says "U2: unit B not drawn" |
| Copy keeps the part's settings | `ComponentInstance::duplicate` copies fields, Part, population; **variant overrides are not copied** (`model.rs:131`) | Holds, as for every paste. The help says so |
| Update PCB names joining and leaving parts | Membership is one project-wide list. A `block.members` change is already on the board when it is applied; there is nothing pending for Update PCB to review. Update PCB (`layout.sync`, `layout.rs:831`) only adds unplaced parts, to the staging area (`layout/staging.rs:36`) | **Does not hold as proposed.** Joining and leaving show at once, in their own undo step, on both sides. Update PCB names the block of each part it adds and places it next to the block (a start point for the staging packer, which keeps its clearance rules) |
| Empty block removed and named | `follow_instances` drops a block whose last member is deleted, silently, on every schematic edit (`instances/blocks.rs:255`) | The removal holds. The naming belongs in the review of the deletion that empties it, not in Update PCB. The deletion's review lists "Block Converter goes: its last part is deleted" |
| A locked block is never moved | Update PCB never moves placed parts. An update that changes or moves a locked part is blocked in its review (`updates.rs`), and since replace-lock (969f8f54) a part of a locked block is blocked the same way | Holds |
| "Place like Converter" | Nothing like it exists; B5's `block.board_arrange` places by a recipe | New: one reviewed plan step, using the stored pairing. A held or locked part, or one that does not fit, is staged and named, as in Arrange (section 7) |
| Each use of a repeated sheet has its own block | Each use has its own physical instances ([schematic capture](../schematic-capture.md#sheets-and-hierarchy)) | Holds, with the block stored in the definition (section 8). B1 stores blocks only with the instances, so this is new data: B6 |
| Undo | Every `block.*` command and the paste are each one undo step | Paste and its new block must be one step. That is why the block travels in the clip rather than in a second command after the paste |

### 13.3 Data

- `Block` gains an optional `copied_from`: the source block and the pairs (source part, copy). It is dropped when "Place like" is done or declined, or when the source block goes. Stored in the project, so it survives save and reopen between paste and Update PCB.
- `SheetClip` gains an optional `blocks`: each block whose members are all in the clip, with its name and members by clip identity. A block partly in the clip is not copied, and the copy result says so.
- Nothing stores the sheet border. It is derived each time it is drawn, as on the board.

### 13.4 What naming a block in `view.select` costs

`view.select` takes `objects: [ObjectRef]` (view 1.5). Adding a `Block` variant to `ObjectRef` looks like the obvious way, but `ObjectRef` is also in the replies of almost every entry, inside `Finding`. A new tagged alternative in a reply is a major change (docs/extension-surface.md, reply compatibility). Counted from `surface.schemas` at 8eee1d5f, an `ObjectRef::Block` would change the replies of 231 entries:

| Module | Entries whose reply reaches `ObjectRef` |
|---|---|
| layout | 63 of 72 |
| schematic | 61 of 69 |
| library | 42 of 65 |
| project | 21 of 28 |
| model3d | 19 of 24 |
| rules | 12 of 20 |
| generators | 5 of 7 |
| checks | 4 of 5 |
| files | 2 of 3 |
| fab | 1 of 5 |
| view | 1 of 3 |

That is a major raise of 11 of the 13 modules (all except assist and help), and every MCP client would see it, for one selection.

**Recommended:** an optional `blocks: [id or name]` field on `view.select`'s parameters. That is a minor raise of `view` only (1.5 to 1.6). `ObjectRef::Block` becomes worth its cost when a finding must name a block, which is B3's `block.part_missing`. B3 can make that major raise once, deliberately, for every module at the same time. Until then a finding names the block's anchor part and the block in its text.

### 13.5 Slices

| Slice | What | Useful alone for | Hours |
|---|---|---|---|
| **S1** | Border on the sheet with name and "also on sheet N", Make block on the sheet, cross-probe of a block both ways, `view.select` `blocks` (view 1.6) | Seeing the converter as one thing on the sheet | 8–12 |
| **S2** | `block.sheet_move`: the core rule for what comes along, border drag on the sheet (schematic and project minor) | Moving a converter on a sheet in one drag | 6–10 |
| **S3** | Copy and paste with the block: `SheetClip.blocks`, "Converter 2", `copied_from`, unplaced units named, local labels renamed | A second converter in one paste | 8–12 |
| **S4** | Update PCB: new members next to their block, block named per added part; the deletion review names a block that goes | New parts land where they belong | 5–9 |
| **S5** | "Place like Converter" in the Update PCB review, one step | A pasted block placed like its source | 10–14 |
| **S6** | Copper like the source (after S5): tracks and vias inside the block copied where the new nets allow, through the impact review | A pasted converter routed like its source | 14–20 |
| **S7** | "Make block of this sheet"; on a repeated sheet it waits for B6's block in the definition | A converter sheet as a block in one click | 3–5 |

S1 needs B1 only. S1 to S5 together come to 37 to 57 hours, and with S6 and S7 to 54 to 82. Blocks in a repeated sheet's definition stay in B6.
