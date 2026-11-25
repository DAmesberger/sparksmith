# Autorouter: a staged, visible plan and a neurosymbolic router

**Status:** design for founder review, 24 November 2026. Nothing here is built. Estimates are agent hours. Related: [agent harness](agent-harness.md), [push-and-shove](../push-and-shove.md), [high-speed routing](../high-speed-routing.md), [copper graph](../copper-graph.md), [rules](../board-rules-and-shape.md), [board geometry for an LLM](../llm-geometry.md), [ties](net-ties.md), [layout blocks](layout-blocks.md), [extension surface](../extension-surface.md).

Founder (verbatim): "I think we need 'normal' AI combined with symbolic AI and advanced algorithms. any cool breakthroughs on solvers we could use, super efficient, state of the art in 2026?"

**Goal:** an AI routes a real board autonomously, verifiably and reproducibly. The first target is the handheld USB-C multi-tool: short USB4 85 Ω pairs on one outer layer, USB2 at 90 Ω, a 48 V / 5 A path, star ground and Kelvin ties. Its spec (`docs/design/usb-c-multitool.md`) did not exist when this was written, so no number here is taken from it.

**How to read the evidence.** Each source is marked: **[read]** the abstract or page was read in full, **[summary]** numbers came from tables in a long page that was read through an automatic summary, **[memory]** a classic paper cited from memory and not re-read, **[not accessed]** paywalled. Vendor claims are marked as such.

**The answer in short.** There is no single solver breakthrough that routes PCBs. The 2026 results agree on a division of labour: a language model that plans in topology and repairs from typed feedback, on top of a deterministic router, beats both a model drawing copper and a router alone. The solver work that pays for us is a **topological space model** (a triangulation of the free space) shared by the model and the solvers, **negotiated routing** on it, **min-cost flow** for escapes, and a **proof-producing constraint solver** for small exact repairs. GPU routers, differentiable routing, diffusion and end-to-end RL do not fit PCB scale or our verifiability rule today.

**What makes it the best, if anything does:** not a faster maze search but the process before it (section 3). The board is planned in stages a person can see and steer: what flows through each net, where connectors, power, domains, returns and lanes go on every layer, and what to move. The plan is rough and quick, typed, stored, replayable, and everything in it can be dragged; the router takes it as a suggestion.

## 1. What makes PCB routing its own problem

| Aspect | ASIC / FPGA | PCB | Consequence for us |
|---|---|---|---|
| Geometry | Fixed tracks or a fixed resource graph, Manhattan | Any angle, any width, pads of any shape, gridless | A grid is either too coarse or too large. The search space has to be the free space itself |
| Scale | 10⁵ to 10⁷ nets | Tens to a few thousand nets | Quality and proof matter more than throughput. GPU global routing solves a problem we do not have |
| Layers | Many, with preferred directions | 2 to 8, no preferred direction, planes among them | Layer choice is a design decision per net class, not a capacity trick |
| Vias | Cheap and regular | Each costs drill, annular ring and clearance on every layer it passes, breaks the return path and adds length | Via count is a first-class cost and often a hard limit (USB4 pairs: none) |
| Pours | None | Fills carry power and the return current, and refill round tracks | A track on a plane layer cuts the plane. Routing has to price that |
| Pairs | Rare | Common; coupled, impedance from width, gap and stackup | A pair is one fat object in every layer of the router |
| Length | Timing-driven, tool-owned | Skew and group matching by meanders | Room for tuning has to be reserved while routing |
| Return path | Implicit | A high-speed net needs unbroken reference copper under it | A constraint between a track and another layer's fill |
| Manufacturing | One PDK | Each fab profile differs; acute angles, slivers, mask webs, drill rules | Limits come only from the board's rules snapshot |
| Intent | In constraints files | Largely in the engineer's head: star points, Kelvin sense, current, creepage | This is the part a language model can supply, and it must be written down as typed data |

## 2. Survey: what works and what is hype

### LLM agents on routing benchmarks

- **A model that places segments step by step fails on real boards.** PCBWorld (LG AI Research, a KiCad-engine environment) reports every LLM agent at 0.00 clean pass on its medium real boards (31–100 pads), while its small synthetic boards are nearly solved ([arXiv 2607.05915](https://arxiv.org/abs/2607.05915), **[summary]**). PCB Gym reports frontier models producing a DRC-valid, fully routed board on 32.1 % of attempts, with mistakes such as routing power rails like signals ([NeurIPS 2026 listing](https://neurips.cc/virtual/2026/161602), **[read]**, abstract only).
- **A model that plans above a deterministic router works.** PCBRouteAgent splits the work: the model writes a "guide patch" (connection trees, corridors per layer, via regions, net order) and a scripted executor draws copper with a push-and-shove router and a grid A* detour. Clean pass, meaning all connections complete and zero error-level violations: 100 %, 94.9 %, 100 % and 40 % on the synthetic, small, medium (10 boards) and large (10 boards, over 100 pads) splits ([arXiv 2611.08175](https://arxiv.org/abs/2611.08175), **[read]** for these four numbers). Its ablation is the important part: the same scripted pipeline with no model revisions reaches 53.5 % on small and 0 % on medium boards, and the model's guide revisions lift that to 94.9 % and 100 % **[summary]**. It also needed far fewer tokens than the step-by-step agent on the same 247 boards (10.5 M in and 1.5 M out, against 80.3 M and 25.4 M) **[summary]**.
- **Its stated failures are ours to design against:** net order (power routed first blocked SCL and SDA; 70 of 83 connections on one board), differential pairs routed as two unrelated nets (34.03 and 36.49 mm against a 30.25 mm reference pair), 6 of 10 large boards incomplete, and all boards two-layer **[summary]**.
- **A classical autorouter is a strong baseline, and learned policies only match it.** CADET (also LG AI Research) trains a policy on 100,000 synthetic boards with executable demonstrations. One rollout reaches 67.2 % clean pass on medium boards, against 65.6 % for the classical autorouter it compares with and 56.0 % for the strongest LLM agent at about 32 times the run time ([arXiv 2611.16913](https://arxiv.org/abs/2611.16913), **[read]** table 2). The learned policy uses more vias and wire than the classical router **[summary]**. Evaluation stops at two layers and 100 pads **[read]**.
- **Models read structured text, not pictures, and misreport their own work.** One practitioner's two-month study (one person, one board family, so weak evidence) found failure recall 0.82 from a structured text package against 0.03 from an SVG; model-chosen net order gave no gain; drawing copper directly completed 0 of 3 boards; and 27 of 27 agent sessions reported a finished job when 1 to 8 nets were done ([forum post](https://forum.kicad.info/t/two-months-of-testing-llms-on-pcb-autorouting-what-worked-what-didnt-and-whats-left-to-try-open-archive/72250), **[read]**, first post). This matches our own rules: typed feedback, and an independent verifier.

### Commercial AI routers

No vendor publishes a benchmark with a method. What exists:

| Product | What is claimed | What the evidence supports |
|---|---|---|
| **Quilter** | "Physics-driven AI"; an 843-part, 8-layer, two-board Linux computer that booted first time | Its own write-up is candid: 98.4 % and 98.7 % routing completion after 27 h and 15 h of compute; BGA escape partly by hand; placement of the module by hand; 38.5 h of human clean-up; early runs at 60–70 % with no length matching and most pairs failing ([Quilter blog](https://quilter.ai/blog/compiling-an-ai-designed-computer), **[read]**). A real result on a hard board, by candidate search at heavy compute, not full autonomy. Algorithms not published |
| **DeepPCB (InstaDeep)** | RL router, "DRC-clean in hours" | Vendor material only. Its own site says engineers keep DDR, RF and tight-impedance nets ([deeppcb.ai](https://deeppcb.ai/), search summary, not read in full). No independent numbers found |
| **Cadence Allegro X AI** | "10× turnaround", placement and routing in minutes | Vendor claims from the 2023 launch; one customer placement anecdote (75 minutes against 3 days). No routing completion or DRC numbers found ([press release](https://www.businesswire.com/news/home/20230406005098/en/), search summary) |

Honest reading: the commercial tools reach "most of the board, a person finishes it", at hours of compute, and none shows a verified, reproducible, fully autonomous route of a board with pairs. That gap is our opening, on small boards first.

### Solver families

| Approach | Good at | Bad at | Verdict |
|---|---|---|---|
| **Negotiated congestion** (PathFinder, McMurchie and Ebeling 1995, **[memory]**): route every net allowing overuse, raise the price of overused resources, repeat | Removes the net-order problem; simple; deterministic | Needs discrete shared resources; convergence is not guaranteed on tight instances | **Use**, on the portals of the topological model |
| **Topological / rubber-band routing** (SURF, Dai, Dayan and Staepelaere 1991, **[memory]**; [Staepelaere's thesis](https://tr.soe.ucsc.edu/sites/default/files/technical-reports/UCSC-CRL-92-39.pdf), search summary): decide which side of each obstacle a net passes, geometry later | Any-angle by nature; small search space; exact capacity tests; the result is readable ("between C3 and R1") | Turning topology into 45° copper can fail where any-angle copper would fit | **Use as the core model** |
| **Conflict-based search / MAPF** (CBS, Sharon et al. 2015, **[memory]**). UGPCB-CBS applies CBS on Delaunay meshes to PCBs ([J. Supercomputing 81, 1063, 2025](https://doi.org/10.1007/s11227-025-07569-0), **[not accessed]**; abstract seen through search only, no numbers verified) | Optimal for a handful of interacting nets; explains the conflict | Exponential in conflicts; MAPF's own frontier for dense cases has moved to non-optimal solvers ([survey, arXiv 2505.19219](https://arxiv.org/abs/2505.19219), search summary) | **Use in small windows** for repair. Its priority-search variant (search over net order) is the cheap fix for the net-order failure |
| **Min-cost flow escape** (Yan and Wong, DAC 2009, [doi](https://doi.org/10.1145/1629911.1630001), search summary: the first exact flow model, with diagonal capacity) | Polynomial, exact, deterministic for escaping an array to its boundary | Fixed pitch arrays; ordered escape needs more (SAT, Luo and Wong 2008) | **Use** for fan-out; SAT only when a bus must arrive in order |
| **SAT / MaxSAT / CP with lazy clause generation** | Exact answers with a **proof**: optimal, or infeasible with the conflicting constraints named | Does not scale to a whole board | **Use** for windowed repair, layer assignment, pin swap. [Pumpkin](https://github.com/ConSol-Lab/Pumpkin) is a pure-Rust CP solver, MIT OR Apache-2.0, that logs checkable proofs (**[read]** README; its wasm build and speed are untested by us) |
| **GPU global routing** (FastGR: 2.489× overall over a CPU router, [TCAD 2023](https://www.cse.cuhk.edu.hk/~byu/papers/J86-TCAD2023-FastGR.pdf), search summary; GAP-LA: layer assignment for up to 12 M nets, [arXiv 2507.13375](https://arxiv.org/abs/2507.13375), search summary) | Millions of nets on gcell grids | Nothing a PCB needs | **Skip.** Our whole problem fits in one CPU cache |
| **Differentiable routing** (DiffRouter, [arXiv 2610.33510](https://arxiv.org/abs/2610.33510), **[read]** abstract: FPGA, over 5× faster than RWRoute, needs a detailed-routing completion step) | Fixed routing-resource graphs | Continuous relaxations give no legal copper; no PCB version found | **Skip.** Watch for placement |
| **Diffusion** (RouteDiff, [NeurIPS 2026 workshop](https://neurips.cc/virtual/2026/161614), **[read]** abstract, no numbers) | Imitating human-looking routes | Image output, no guarantees, fixed net order | **Skip** as a router; possible later as a proposal source behind the validator |
| **RL** | Tuning a symbolic router: an offline-RL policy that sets a detailed router's cost weights finishes 1.56× faster on average with no more violations ([arXiv 2512.03594](https://arxiv.org/abs/2512.03594), DATE 2026, **[read]** abstract; IC routing), and a follow-up cuts violations by 92 % on dense layouts ([arXiv 2609.08232](https://arxiv.org/abs/2609.08232), **[read]** abstract) | Drawing copper end to end (see CADET) | **Later**, as a learned cost schedule in a strategy slot, trained on our own traces |
| **Any-angle search** (Polyanya: optimal any-angle paths on a navigation mesh with no preprocessing, [IJCAI 2017](https://ijcai.org/proceedings/2017/70), search summary) | Shortest paths on the same kind of mesh we build | A point robot; clearance has to be added | **Not needed** once the corridor is known: the funnel algorithm is linear. Keep in mind for free search |
| **Any-direction length matching** ([arXiv 2407.19195](https://arxiv.org/abs/2407.19195), **[read]** abstract, no numbers) | Meanders inside assigned regions, pairs as a median trace | New | **Watch.** Our meander planner covers the first board |

## 3. The core: a staged, visible plan before any copper

The autorouter must be the best on the market, and it makes or breaks the all-AI approach (founder, 23 November 2026, as relayed). The process:

1. **Floorplan:** where connectors, power, domains, returns and lanes go, drawn as areas and corridors on every layer, with a rough part placement inside and the reason for each.
2. **Revisions the AI suggests** ("move connector X, component Y elsewhere"). The person accepts in the chat, proposes an alternative, or drags.
3. **Place the parts.**
4. **Autoroute.**
5. **Loop** until finished or optimised.

This is how a good layout engineer works, and it is what the 2026 results miss: PCBRouteAgent plans corridors only after placement is fixed and fails on net order and pairs; Quilter searches candidates for hours. Most routing failures are decided before routing, by where the connectors and the power sit. None of the sources read in section 2 shows a typed, visible plan of this kind; that is a statement about what was read, not a market survey.

**The floorplan is rough and fast.** It will never be perfect. Four rules follow:

- It is made without routing anything. Its notes (3.4) are simple sums that follow a drag.
- Nothing waits for it. Placement and routing start whenever the person or the agent asks, with any floorplan or none.
- It is a suggestion. A part may sit outside its area and a trace may leave its area or corridor. That is reported, never an error. What must hold is a rule or a keepout, which the board checks enforce.
- It is detailed enough that the board need not be ripped up because too little was planned. Planned corridors count: all but the simplest boards have more than 5 to 20 areas. When routing finds a corridor too narrow, only that corridor is changed and only its nets are routed again.

### 3.1 The floorplan: one typed, persisted plan

The plan is called the **floorplan** here. "Board plan" already means the description of the board's bodies and `RoutePlan` a route step's edits, so neither name is free (decision 14).

It is stored in the project file (format version 1, edited in place before release; fields defaulted and skipped when empty), changed only by typed operations, and so follows undo, revisions, dry runs and turns like everything else.

Each copper layer has its own map: corners, edges between them, and areas bounded by edges. The outer layers' maps also plan the parts of their side.

| Element | Holds | Notes |
|---|---|---|
| **Corner** | Id, layer, position | On the board edge it is stored as a place on the outline, so it follows the outline. Three or more areas can meet in one corner |
| **Edge** | Id, layer, two corners, bends or arcs between them, `locked` | **Stored once.** The areas on both sides refer to it, so one drag reshapes both and no gap or overlap can appear. An area's edge along the board is the outline itself, not a copy. A locked edge and its corners are moved by no drag, solver or proposal until it is unlocked |
| **Area** | Id, layer, the edges around it (and around its holes), uses, function, members (parts, [blocks](layout-blocks.md), nets, groups), the net of a pour, purpose, rationale | Any shape. Its outline is derived from its edges. **Uses** are a closed set because solvers treat each differently: `parts`, `routing`, `pour`, `return`, `quiet`, `isolation`, `reserve` (tuning room, future parts), `keep_clear`; an area may have several. **Function** (power, digital, analog, radio, connectors, …) is library data: name, letter and colour (decision 24). Areas of one layer do not overlap and need not cover it |
| **Corridor** | Id, the net groups it carries, layer, a spine (polyline with arcs), planned width, the order of the nets at each end, reference layer, whether others may cross and where | Lies over areas. A **fast lane** is a corridor with three extra wishes: its reference layer, no layer change, and being routed first |
| **Power plan** | Per rail: source, loads with current, the tree between them, the carrier of each branch (pour area, track of a width, plane), layer changes and their via count, ties and star points, the return | Currents and voltage come from the net profiles (3.2). A branch whose current is unknown is listed as unknown, not sized |
| **Placement intent** | Constraints, not coordinates: `edge(J1, left)`, `in(U3, area A2)`, `near(C7, U3.VDD, ≤ d)` with a distance from the person or a cited rule, `block(B1)`, `side`, `orientation` | A connector can be fixed, partly fixed (its edge, not its place) or free. The same vocabulary as the harness's constraint placer (H10) |
| **Rough position** | Part, side, position, rotation | Only for a part that is not on the board yet. Placing the part removes it, so a part never has two positions. A placed part is shown where it is (decision 33) |
| **Rationale** | A sentence, plus references to the facts it rests on: net profiles, rules, plan notes, other plan elements | On every area, corridor and branch. One without a fact says so where it is shown. When a fact it rests on has changed since, the element is marked; the mark is derived, not stored |
| **Proposal** | Kind (`move_part`, `rotate_part`, `reshape_edge`, `reshape_corridor`, `change_layer`, `change_carrier`, `change_stackup`, `ask_rule`), the typed operations it would apply, the notes it answers, the numbers before and after, status | Written by the model or produced by a solver. The numbers are computed by the engine on a fork, never stated by the model |
| **Decision** | Proposal, `accepted`, `declined` or `replaced_by` another proposal or a hand edit, by whom, at which revision, a note | The log that makes a run replayable and explains the board later |

**Replayable:** the floorplan's history is a sequence of typed operations and decisions in the trace. Replaying them on the same starting file gives the same floorplan, and the solvers of section 4 then give the same board.

### 3.2 Net characterization: what flows through each net

Planning needs to know, per net, pair or bus: its role, voltage, current, highest frequency or edge rate, interface and role in it, impedance target, topology (source and sinks) and drive. The engine derives a **net profile** from the pins on the net. Each field carries its basis:

| Basis | Meaning | Examples | May it drive a hard check or a width? |
|---|---|---|---|
| **Declared** | Stated on a pin function, a part, a net class, a pair, a tie or the stackup, with its source | Direction, signal kind and drive of the active function; a class's impedance target; a regulator output's voltage | Yes |
| **Derived** | Computed from declared facts by a named rule | Source and sinks from directions; pair width and gap from the target and stackup (`impedance::derive`); a rail's current as the sum of its loads' declared maxima, labelled as that | Yes, and the row names the rule |
| **Inferred** | A model's or a heuristic's guess | "SDA/SCL look like I²C"; a load's current with no datasheet value | **No.** It may steer soft choices (order, where to propose an area). It becomes declared only when the person confirms it or a datasheet step sources it |
| **Unknown, must ask** | A planning decision depends on it and nothing states it | The current of the 48 V path; the allowed temperature rise; whether a pair needs unbroken reference | It is asked (`assist.ask`), and the answer is stored as a declared class or net value with "set by the person" as its source |

This follows the rules already in force: nothing electrical is inferred from names, `diff_pair.suggest` proposes and never applies, and a limit without a source is unchecked.

**What exists, and is used as it is** ([part library design](../part-library-design.md#native-multi-function-ios), [pin functions as built](../component-model.md#pin-functions-peripherals-rules-pinout-import-and-firmware-c1c7-as-built), `crates/editor-core/src/functions.rs`):

| Existing type | What the profile reads from it |
|---|---|
| `Function` on a terminal's `TerminalFunctions` (the functions array: a pin such as PA0 offers GPIO, UART TX, ADC input as structured alternatives) | `direction` (input, output, bidirectional, passive, power input, power output), `signal` (`SignalKind`: digital, analog, power, unspecified), `drive` (`Drive`), `resource_role`, and `source` (`SourceSpan`: datasheet page and table) |
| `FunctionSelections` on the instance (`instance.select_functions`), resolved to `ActiveFunction` | Which alternative is active on this board, and why (`selected`, `fixed`, `design_default`). The profile uses only the active function, as ERC does. A terminal with no resolved function gives an unknown, not a guess |
| `Resource`, `Role`, `ResourceRole` | Which pins form one peripheral and each pin's role in it; `Role.required_when_used` and the `config.requirement_unmet` finding say when an interface is only partly wired, so its profile is marked incomplete |
| Pin rules (sandboxed Rhai) and `FunctionConstraint` | Nothing new. They decide which selections are legal; planning reads the result |
| `PinConfig` (pull, speed grade, initial level, Schmitt) | Shown in the profile. A speed grade is the vendor's opaque text and gives no edge rate by itself |
| `NetClass.signal` (`SignalRules`), `DiffPair` with its accepted target, `LengthGroup` | Impedance targets and tolerance, pair limits, length windows: declared, with the pair catalog's source when the person accepted a target |
| The pair catalog, `library/standards/differential-pairs.json` (branch `integ-1`), loaded by `library::standards` | Sourced interface targets today: per interface a `target` (`interface`, `ohms`, `tolerance_pct`, `source`, `url`) and the net-name aliases `diff_pair.suggest` matches. A name match is offered to the person and never applied |

So the model is there. Planning needs three small additions to it, all optional fields that existing files and parts simply lack (format version 1 edited in place; `library` and `rules` minor versions). No new pin structure.

| Extension | Lands on | Shape | Stays free text |
|---|---|---|---|
| **Electrical characteristics** | `Function` (one new optional field, `electrical`) | Each value optional, each with its own `SourceSpan`: `voltage` (a range: the supply range of a power input, the level of a power output, the swing of a signal), `current` (the most a power input draws, a power output supplies or an output drives), `rate` (highest toggle or bit rate) or `edge` (fastest rise time). The pinout import and the datasheet workflow can fill them as they fill functions now, with page and table | `Function.label`, `MuxCode`, `mux_table`, and `PinOption::Speed` grades stay opaque |
| **Typed interface reference** | `Resource` (new optional `interface`: the catalog entry's id) and `Role` (new optional `interface_role`: the role's id in that entry, such as the positive half of a named pair) | Two ids, checked on publication against the catalog the part was made with; an unknown id is refused naming it. This is what lets a net profile say "USB 2.0 data, positive half" as a **declared** fact from the pins at both ends, instead of from the net's name | `Resource.kind` stays grouping text for search and firmware templates and still never selects behaviour. `Role.signal` stays the datasheet's signal name |
| **Interface catalog** | The existing pair catalog and its type in `library::standards`, extended in place | Each interface entry gains a stable `id`, its `roles` (which two form a pair, which are single-ended), and further optional targets beside `ohms` and `tolerance_pct`: single-ended impedance, highest rate, length, skew and via limits. Every target carries `source` and `url` as the pair targets do. The `names` aliases stay, for suggestions only | The `interface` display name. A target the catalog does not state is not planned against |

Where interface-level and pin-level data meet: impedance and the interface's rate belong to the catalog entry; voltage, current and a pin's own edge rate belong to the function. When the pins at two ends of a net declare different interfaces or roles, that is a finding for the person, not something the planner resolves.

Passive parts in the signal path (a series resistor, an ESD diode, an AC coupling capacitor) split a signal into several nets. For pairs, the as-built signal path already runs through parts both halves reach. For other signals, "these nets are one signal through R4" is an inferred suggestion until the person confirms it.

Confirmed values live in one place, the rules (net class or net), so the board checks, the planner and the router read the same numbers.

**How a profile drives the plan** (each line is a default of the `plan.order` and `route.cost` strategies, as data, not code):

| Profile says | Plan decision |
|---|---|
| An interface with an impedance target, or a declared edge rate | A fast lane: an outer or stripline layer next to a marked plane, planned first, shortest spine, no layer change unless the class allows vias. Width and gap from `impedance::derive` on that layer |
| A current | The carrier and its width from the class current and the sourced formula (decision 6); the number of vias at a layer change only when a per-via current is sourced. Otherwise sized from the class width and marked "current not checked" |
| A voltage above another domain's | An area with the use `isolation`; the class-pair isolation clearance that already exists is the limit |
| Analog, sense or Kelvin role | An area with the use `quiet`, away from the power plan's switching branches; targets are the tie's anchor lands |
| Open-drain or slow digital bus | Lowest priority, any signal layer, may cross other corridors by via |
| Source and sinks known | The corridor's direction and the order pins must arrive in, which feeds the escape check |

### 3.3 Stages and who does what

| Stage | The model proposes | Solvers compute | Surface (section 5) |
|---|---|---|---|
| **0. Characterize** | Inferred roles, as suggestions; the questions to ask | Net profiles from pins, classes, pairs, ties, catalog | `rules.net_profiles`, `net_class.edit`, `assist.ask` |
| **1. Floorplan** | Connector edges and order, areas per layer with use, function and purpose, corridors, the power tree, returns and pours, placement intents, each with a rationale | The edges and corners that realise the proposed areas; corridor widths; rough positions; the notes of 3.4 | `plan.area_*`, `plan.edge_*`, `plan.corridor_*`, `plan.power_set`, `plan.intent_set`, `plan.rough_place`, `plan.notes` |
| **2. Revise** | Proposals: move or rotate a part, reshape an edge or a corridor, change a layer or carrier; or a question about a rule | Each proposal on a fork, with its numbers before and after; a solver may add its own | `plan.propose`, `plan.decide`, the chat |
| **3. Place** | Changed intents | The constraint placer: a legal placement near the rough positions; what it could not meet is listed with the amount | `plan.place` |
| **4. Route** | Per-run preferences only | Section 4, with the floorplan as a preference | `layout.autoroute`, `layout.route_report` |
| **5. Loop** | Whether to stop, optimise or revise the plan, from the report | Rip-up and repair of the nets concerned | all of the above |

The stages are an order of work, not gates. A person or an agent can start, skip or return to any of them. The loop ends when the route is clean, when the person stops it, or when a round improves nothing (the report says which nets are open and why). No turn or token cap ends it by default, as the harness rules.

**The planner thinks like an engineer from the first area.** Its method (data in `library/assist/workflows/plan-and-route/`) goes through these questions in order, and each answer is drawn:

| Question | Facts it uses | What it draws |
|---|---|---|
| What is fixed? | Outline, mounting holes, locked parts, the linked design's datums, what the person said | Locked edges, connector areas, fixed and partly fixed connectors |
| Where does power come in and go? | Rails, sources, loads and currents | The converter's area with its parts close together, the power tree, pours |
| Which domains must stay apart? | Net profiles (analog, digital, radio, voltage domains), isolation rules | Quiet and isolation areas, the place where grounds meet |
| What is fast? | Impedance targets, rates and edges, interfaces | Fast lanes first and shortest, each with its reference layer |
| Where does each return run? | Reference layers, ties, pours | Return areas on the inner layers, unbroken under the fast lanes; no split across a lane |
| What does each inner layer do? | Stackup, plane marks, class layers | Routing, pour and return areas per inner layer |
| What goes on the other side? | Side rules of the parts, heights, assembly choice | Areas and rough positions on the bottom |
| In which order do nets arrive? | Pin order at both ends | Net order at corridor ends, and which corridors must cross |

### 3.4 Plan notes

A note is a quick sum over the floorplan, the net profiles and the rules snapshot. It is a pure function, uses no router and no search, and is computed as the person drags. A row reads need, have and margin, and is `fits`, `short` or `not estimated` with the reason.

| Note | Definition | Source of the numbers |
|---|---|---|
| **Corridor width** | The nets' widths plus class and class-pair clearances, against the planned width | Rules snapshot; pair width and gap from `impedance::derive` |
| **Corridor room** | The planned width, against the narrowest gap between fixed obstacles (locked parts, holes, cutouts, keepouts) along the spine | Physical inventory, keepouts |
| **Crossings** | Corridors that cross on one layer, from their spines and from the net order at each end. Each needs a layer change for one bundle, compared with the class's via limit | Stackup, class allowed layers and via limit |
| **Area fill** | The members' courtyards, against the area | Packages |
| **Reference under a fast lane** | The length of its spine that lies outside a `return` or plane area on its reference layer | Floorplan, plane marks |
| **Current width** | The branch's width for its current, against its corridor or pour area | Only with decision 6's formula and a sourced per-via value; otherwise `not estimated` |
| **Length** | The spine's length, against an interface's limit | Interface catalog, class limits; `not estimated` when unset |
| **Voltage drop** | Resistance of track branches from width, length and foil (copper resistivity from the material library) | Tracks only; a pour is `not estimated` until the resistance solver exists |
| **Height** | A rough part's body height, against the room the linked design leaves there (3.6) | Package, linked design; `not estimated` without a link |

Notes inform. They stop nothing, they are not findings, and they never enter Issues or the fabrication preflight. There is no invented threshold: no "congested above some percentage". `fits` is not a promise that the route exists; how often it was wrong is measured (section 8).

### 3.5 In the editor

Everything follows the design system in `crates/ui`: shared tokens, the one subject card above Issues, no new chrome. Overlays are canvas drawing only; showing, hiding, hovering or selecting them rebuilds no geometry. The [mock-up](floorplan-mockup.html) shows the items below on a made-up board. It is a mock-up: none of it is built.

- **Plan mode** is a mode of the PCB view. Entering it arms the **Plan card** as the subject card: the areas, corridors and rails of the layer in view, the open proposals and the notes. Rows highlight their object on the canvas, as High speed rows do.
- **Overlays per layer.** Every layer's overlay has its own switch, beside the layer's copper switch, and the two are independent. Areas are transparent. Corridors are ribbons at their planned width with the direction of flow; a fast lane has its own stroke and names its layer and target ("top · 85 Ω"). The power plan is a tree: branch width follows the planned width, each branch labelled with current and carrier. Rough parts are dashed outlines. Unknown values are drawn as unknown, never as zero.
- **Dragging.** Drag an edge, a corner, a bend, a corridor or a rough part. A shared edge is one object: both areas follow. A drag stops where the edge would cross another edge, leave the board or meet a locked one (decision 34). One drag is one operation and one undo step. Arrow keys do the same.
- **Locking** an edge is a click in its Properties; it shows a lock and a dotted stroke, so the lock does not rest on colour.
- **A trace outside its corridor** is drawn and reported like any other trace. The route report says which nets left theirs.
- **Colour.** Copper already uses one colour per layer, so an overlay must look like no copper and still say its layer. In Plan mode copper is grey, and the areas of a layer are pastels of that layer's own colour, in lightness steps chosen so that two neighbours never share one. The function is said by a letter and the name. In the peeling view each layer also has its own hatch direction. A key switches the layer in view to function colours (decision 32).
- **Peeling view.** The 3D view's layer peel shows each layer's floorplan on its own layer, with the same switches. It draws the same stored map; there is no second model.
- **Rationale on hover.** One tooltip per object (`kit::text`): the purpose, the sentence, and the facts it rests on. Selecting the object puts the same in Properties, with the facts as rows that jump to their source ("Show rule", the net, the pin).
- **Preview of a proposal.** It is drawn over the floorplan: unchanged elements normal, added in the preview-add style, removed as ghosts, moved parts with a line from old to new place. The card lists the same changes as rows. This is the existing review (`impact` control), not a new widget.
- **Chat and cards.** A proposal arrives in the chat as one card: what, why in one line, the numbers before and after, and three actions: **Accept**, **Alternative…**, **Why?**.
  - *Accept* applies the proposal's operations in the turn (one undo step) and records the decision.
  - *Alternative…* lets the person answer in words ("put it on the right edge instead") or by doing it: drag the part, the edge or the corridor on the canvas. A hand edit is an ordinary operation; the model is told what changed by hand, and the old proposal is recorded as replaced.
  - *Why?* opens the stored rationale and its rows. It costs no model call. A follow-up question is ordinary chat.
  - Hovering a card highlights its objects; **Show** frames them (`view.select`).

How the colour modes compare, measured on the mock-up's board (OKLab distance × 100, larger is easier to tell apart; light theme / dark theme; protan, deutan and tritan simulated after Machado, Oliveira and Fernandes 2009 at full severity, **[memory]**):

| Mode | Closest neighbours that differ in colour, normal vision | The same, worst vision | Neighbours with one colour | Says the layer |
|---|---|---|---|---|
| **Layer pastel** (recommended) | 7.6 / 7.9 | 6.7 / 7.3 | 0 | By hue, hatch and name |
| **Function** | 8.6 / 4.8 | 3.1 / 3.5 | 1 pair | No |
| **Function, layer border** | 8.6 / 4.8 | 3.1 / 3.5 | 1 pair | By the border |
| **Layer, function pattern** | By pattern only | By pattern only | By pattern only | By hue and name |

One finding is independent of the mode: the pastels of Top and Inner 1 differ by only 0.4 (light) and 0.2 (dark) in the worst colour vision, because the product's copper colours for these two are close there and do not change with the colour vision setting (`apps/sparksmith/src/palette.rs`). Hence the hatch. Changing the copper colours is a separate question and not part of this design.

### 3.6 Hulls from the enclosure

Optional. With a linked design (an enclosure), each area of an outer layer can be shown as a **hull** in 3D: the area's outline raised to the room the linked design leaves above the board there. It shows where a tall part even fits. A rough part that is higher than the room under its footprint is marked, with the two heights.

- The room is derived from the linked design, through the [assembly link](../architecture/board-interface.md#the-assembly-link-slice-b4), and is never stored in the floorplan. Without a link there is no hull and no limit.
- Heights of parts come from their packages. A part without one is `not estimated`.
- Evaluating the room above a board in a linked design is not built; it needs its own short design with the link's owners (decision 36).

### 3.7 External agents

Every stage is catalog entries behind the six verbs, so Claude Code, Codex or OpenCode drive the same process. The process itself is a workflow as data (`library/assist/workflows/plan-and-route/`): model steps with typed outputs checked by `assist.step_result`, tool steps for the solvers, and a person's answer only for proposals and questions. In the chat that is the proposal card; for an MCP client it is `input_required`; unattended, a grant can let the agent decide its own proposals (decision 18), which is what the bench and an autonomous build use. An open proposal does not stop the other work.

## 4. The solver layers

These layers are stage 4 of the process in section 3. One loop, the harness's: the model proposes intent, the engine validates, solves, rehearses on a fork, verifies, and answers in typed lines. Seven layers, all reading the same four sources: the **physical inventory** (`layout/physical.rs`: every land and hole), the **copper graph** (what is joined, what is still open), the **rules snapshot** (profile, overrides, net classes, class pairs) and the **incremental checks**.

```
intent (LLM) ─► validate ─► space model ─► global (topology) ─► escape ─► detail (existing router)
     ▲                         │ capacity proofs      │ corridors            │ route_apply checks
     └──── typed feedback ◄────┴──── repair ◄── verify (copper graph, issues, lengths, ties) ◄─┘
```

### The space model (shared by every layer)

Per copper layer, a constrained Delaunay triangulation of the routable area. Its fixed edges come from the board edge, cutouts, keepouts, every land and hole of the physical inventory, and locked or already committed copper. Fills are not obstacles, as today. Each free edge between two obstacles is a **portal** with an exact width. A portal's **capacity** for a set of nets is a pure function of its width and the rules snapshot: the sum of track widths plus the class and class-pair clearances between neighbours and to both obstacles must fit. Via sites are candidate points inside triangles that pass the drill and ring rules on every layer of the span.

This is the one new model in the design, and it is derived, never stored: built once per revision beside `RouteIndex`, from the same inventory and the same `BoardRules`. It decides nothing about connectivity (the copper graph does) and nothing about clearance on committed copper (`check_committed` and the board checks do). It is a planning index, like the hulls: every piece of copper it leads to is still checked against the exact copper. `spade` and `robust` (triangulation and exact predicates) are already in `Cargo.lock`; whether they suit the wasm core is to be checked.

Why this model: it is any-angle and has no pitch, a board has a few thousand triangles, an unroutable request shows up as a cut whose capacity is too small, and its portals are named by the objects on each side. So "USB_DP passes between C3 and R1" is at once the model's vocabulary, the solver's variable and the person's picture. That closes the "guided" level of [llm-geometry](../llm-geometry.md).

### The layers

| Layer | Algorithm | Why | Cost | Deterministic | Sources of truth | Feedback to the model |
|---|---|---|---|---|---|---|
| **1. Intent and planning** | The floorplan (section 3), as far as there is one, is the plan; the model adds a typed **routing brief** of per-run preferences; the engine validates it | Roles, priorities, star points and "keep this short" are judgement. Everything exact stays out of the model's hands | One model call per plan or repair round | No (recorded and replayed, section 6) | Reads `view.context`, the net list with classes, the airwires, `layout.route_space` | Refusals per field: unknown net or portal, a layer the class forbids, with `instead` |
| **2. Global routing** | Topological routing on the space model. Each connection is an A* over (triangle, portal position) states, across layers through via sites. All connections negotiate: overuse is allowed, overused portals get a rising history price, repeat until no portal is over capacity. A search over net priorities (priority-based search) runs when negotiation stalls | Removes the order dependence that PCBRouteAgent and the practitioner study both hit. Works on few, exact resources | Triangulation O(n log n) in obstacle vertices; one net O(m log m) in portal states; a small number of rounds. Budgets are counted, not timed | Yes: ties broken by object id | Connections are the copper graph's open islands; capacities from the rules snapshot; obstacles from the inventory | Per net: layer, corridor as portals, estimated length, vias. Per failure: the **cut**: "VBUS, USB_DP, USB_DN all need J1.A4–J1.A5 on top; it fits 1" |
| **3. Escape** | Min-cost flow from a part's pads to exit slots on its boundary and to via sites, with exact capacities between pads; one flow per part and layer set | Fine-pitch parts are where order and capacity decide everything, and flow answers both exactly | Polynomial; a part has tens to hundreds of pads | Yes: fixed augmenting order | Pads and holes from the inventory; pad-to-pad and neck rules from the snapshot; duplicate connector contacts are one net and so one sink | Per part: pads escaped, the pads that cannot escape on the allowed layers and the capacity that is missing |
| **4. Detailed routing** | The existing router. The corridor is pulled tight by the funnel algorithm into waypoints; `router::plan` (Shove, then walkaround) routes waypoint to waypoint in the 45° posture; commits go through `layout.route_apply` | One geometry engine. Every leg is already checked against exact copper, and shove gives rip-up-free local repair | Measured today: 0.29 ms median and 11.6 ms worst per shove step on the 10,000-via board ([push-and-shove](../push-and-shove.md#performance)) | Yes (already tested: "the same edits twice give the same board") | `RouteIndex`, `Ctx::clearance`, `check_committed` with `RouteIndex::near` | Per connection: `ok` with length, vias, detour factor; or `blocked` with the object and the missing clearance, as `route_preview` gives today |
| **5. Pairs and length** | A pair is one object with width 2·w + gap in layers 2 to 4 and is realised by `pair_plan`. Layer and via limits from its class are hard constraints. After routing, skew and group targets are met by `meander::plan` on the longest free stretch; the global layer reserves that room by treating the needed extra length as extra portal demand | Fixes "pairs routed as two nets" by construction. Reuses the as-built pair and meander code | As layer 4, plus a bisection per meander | Yes | Pair width and gap from `impedance::derive`; lengths and skew from the copper graph's path lengths; limits from the class, unchecked when unset | Per pair: coupled and uncoupled length, skew against its limit, impedance against target and tolerance; a tuning that does not fit names the stretch and the room missing |
| **6. Repair** | In order: (a) shove (already inside layer 4); (b) rip up and renegotiate the nets of the failed cut; (c) an exact solve of a small window (a handful of nets, their candidate corridors, capacities as hard constraints, vias and length as cost) by a proof-logging CP solver or CBS; (d) back to the model with the conflict set and typed options | Cheap first. The exact step either fixes the window or proves no fix exists inside it, which is the fact the model needs to choose a different layer, a part move or a question to the person | (c) is exponential in the worst case, so it is bounded by window size and a counted node budget | Yes | Same as layers 2 and 4 | The conflict set, and options: `layer`, `move_part` (names the part and direction that widens the cut), `relax` (a soft preference of the brief), `ask` (a rule only the person may change) |
| **7. Verification in the loop** | After each net: the commit's own checks and the findings difference from the incremental checks. After the run: the copper graph (each net one island, no stubs), `layout.length_report`, the tie checks, full `project.issues` and `fabrication.preflight`. In the bench: an independent verifier on the saved file | The model's own report is not evidence (27 of 27 false "done" reports in the study above) | Incremental checks keep non-copper findings; a copper-only edit rechecks copper families | Yes | The engine's checks, unchanged | `findings: {new, resolved}` one line each, as harness slice H6 defines |

**What the model never does:** state a coordinate that reaches copper unchecked, widen a rule, add a waiver, or mark a net done. **What the solver never does:** guess intent. A constraint that is not in the rules or the brief does not exist for it.

### The routing brief

There are three homes, and each fact has one. Durable electrical constraints are **rules**. Where things go and why is the **floorplan** (section 3): its areas, corridors, fast lanes and power branches reach the router as lower cost inside and an earlier turn, never as walls (decision 37). The **brief** holds only what is specific to one run. So a pair that must stay on the top layer is checked by the board checks whether or not the autorouter drew it, and a route outside its corridor is listed in the route report as information.

- **New net-class fields (rules module), each unchecked when unset:** allowed routing layers, a via limit, "needs unbroken reference copper", and the current a class carries. A minimum width from current needs a named, sourced formula and a temperature rise the user states; without both it is not checked ([no invented limits](../high-speed-routing.md#copper-off-the-signal-path) applies).
- **The brief (a request parameter):** nets or groups to route, whether to prefer the floorplan (the default when there is one), soft preferences beyond it (prefer a layer, prefer a side of a part, extra corridors as portal lists), a via and detour budget per net, priorities, and which locked copper to keep. Without a floorplan the brief alone drives the router, which is how the no-plan arm of the bench runs. Example, in the wire's JSON but shown compactly:

```
route: [USB4_TX1, USB4_RX1, USB2]        prefer: {USB2: {layer: bottom}}
corridor: {USB4_TX1: [J1.A2|J1.A3, C12|U3.4]}   budget: {default: {vias: 2, detour: 1.5}}
keep: locked                             profile: default
```

Star ground and Kelvin sense come from the [ties](net-ties.md) design: a tie names the land a net must reach. The router takes its targets from the copper graph's open islands and the ties' anchor lands, so a Kelvin sense track goes to the anchor, not to the nearest copper of the net. This needs the ties slices.

## 5. Interfaces

### Planning entries

A new module, `plan`, so a grant can let an agent plan without letting it move parts or draw copper (decision 15). It starts at 1.0. The profile query and the confirmed values are in `rules`, a minor raise.

| Entry | Class | Does |
|---|---|---|
| `rules.net_profiles` | read | The net profiles of 3.2: per net, pair or bus, each field with its value, basis and source; the unknowns a stage needs |
| `net_class.edit`, and a per-net form | edit | Stores a confirmed value (voltage, current, edge rate, interface) with its source. New optional fields |
| `plan.get` | read, `link` | The floorplan: per layer its corners, edges with locks, areas and corridors; the power plan, intents, rough positions, open proposals, decisions; takes `encoding` |
| `plan.area_add` / `_edit` / `_delete` | edit | Areas, with layer, uses, function, members, purpose and rationale. Adding one splits an area or free space along given points; deleting one joins it to a neighbour or frees it |
| `plan.edge_reshape`, `plan.corner_move`, `plan.edge_lock` | edit | Moves the bends of one edge or one corner, or locks an edge. Every area on it follows. A shape that would cross another edge is refused, as any invalid geometry is |
| `plan.corridor_add` / `_reshape` / `_edit` / `_delete` | edit | Corridors and fast lanes |
| `plan.power_set` | edit | A rail's tree, carriers and ties |
| `plan.intent_set` | edit | Placement intents |
| `plan.rough_place` | edit, `short` | Rough positions: given ones, or computed from the intents and areas for named parts |
| `plan.place` | edit, `short` | Places parts on the board near their rough positions with the constraint placer; `dry_run` previews; places what it can and lists what it could not meet |
| `plan.notes` | read | The notes of 3.4 at this revision; with `proposal`, before and after on a fork |
| `plan.propose` | edit | Records proposals: kind, operations, the notes they answer, the rationale. The engine fills in the numbers |
| `plan.decide` | edit, `review` | Accepts (applying the operations in the turn), declines or replaces a proposal |
| `plan.diff` | read | What changed between two revisions of the floorplan, as rows |

A person's drag in Plan mode sends the same operations.

### Routing entries

All in `layout` unless noted; each raises `layout` by a minor version with `since` set, and `project` by a minor because of the wrappers. The rules fields raise `rules` by a minor (new optional fields).

| Entry | Class | Does |
|---|---|---|
| `layout.route_space` | read, `link` | The space model for a window and layers: regions, portals (the two objects, width, free capacity per class), via sites, congestion. Takes the `encoding` option of the harness's `encode` slot |
| `layout.route_connect` | edit | Routes one connection, net or pair at goal or guided level: terminals or "the net's open islands", optional layers, corridor, via and detour budget. One undo step. With the envelope's `dry_run` it is the preview |
| `layout.fanout` | edit | Escapes one part's pads; `dry_run` previews |
| `layout.autoroute` | edit, `long` | Takes a brief; runs layers 2 to 7 on a fork; applies the result as one undo step; returns the report. `dry_run` keeps nothing |
| `layout.unroute` | edit | Removes the routed copper of named nets or connections, keeping locked copper. (Today a caller has to list tracks for `layout.track_delete`.) |
| `layout.route_report` | read | Per net: status, layers, length, vias, detour, and the grade for its kind from [llm-geometry](../llm-geometry.md#grading); for unrouted nets, the cut or blocker |
| `layout.tune_pairs` | edit | Brings named pairs and groups inside their limits with the existing meander planner, or says which cannot be met and why |

`layout.autoroute` is long work. Progress and cancel need the envelope's `job`, which is refused until slice S4b. Until then it runs to a counted budget and returns, and a caller routes in batches.

### Report shape (one line per net in the compact form)

```
ok   USB4_TX1 top 14.2mm vias0 detour1.04 skew0.02/—  z85.3/85±10%
ok   VBUS top+bot 31.0mm vias2
open SENSE_P: cut J3.2|R14.1 on top fits 0 of 1 (needs 0.50mm, has 0.31) options: layer=inner1, move R14 +y
new  1: clearance R3.1–GND 0.12 < 0.20mm   resolved 2
```

A dash marks a limit the class does not set.

### Strategy slots

The harness's slots gain a `solve` family. Each is JSON to JSON with no hidden state.

| Slot | Decides | May be WASM later |
|---|---|---|
| `plan.order` | How profiles become planning priority and layer preference (the table in 3.2) | Data profile first; WASM later (effect `none`) |
| `plan.notes_view` | The text form of the floorplan and the notes | Yes (effect `none`) |
| `plan.house_rules` | Extra notes ("connectors only on the short edges") | Yes (effect `read`) |
| `route.order` | Initial priorities and rip-up order | Yes (effect `none`) |
| `route.cost` | Weights and schedules: via, layer change, detour, plane crossing, history growth | Data profile first; a learned policy as WASM later |
| `route.brief_check` | House rules on a brief ("no signal on the plane layer") | Yes (effect `read`) |
| `route.encode` | The text form of `route_space` and the report | Yes (effect `none`) |
| `route.repair_options` | Which options a conflict set offers | Yes (effect `read`) |
| Floorplan map, plan notes, space model, constraint placer, global and detailed routers, escape flow, commit checks, copper graph | | **No.** One implementation in the engine. A third-party router may run later as an effect-`edit` plugin on a fork, and its copper still passes `route_apply` |

**No hard-coded data.** Cost weights and counted budgets live in `library/routing/NAME.json` (a routing profile, chosen in the brief and recorded in the trace). The existing router's constants (depth 8, 96 pushes, 1,500 queries, 24 obstacles) should move there in the same slice. Limits come from the rules snapshot only.

## 6. Determinism and reproducibility

No document describing the autonomous-build protocol was found in the repository; this is what the router contributes to it.

- **The solvers are pure functions** of (document at revision R, brief, routing profile, engine version). No clocks, no thread-order dependence, no hash-map iteration order in decisions, ties broken by object id, budgets counted. Parallel work is allowed only over independent items with an ordered merge.
- **No randomness by default.** A strategy that uses random restarts takes its seed in the brief; the seed is in the trace.
- **The model is the only non-deterministic part**, so its outputs are recorded (`model::record`). Three levels of reproducibility, each tested:
  1. *Solver replay:* the same document, brief and profile give a byte-identical edit set. A CI test, on every bench board.
  2. *Run replay:* a recorded run replayed against the same engine version gives the same saved file (same SHA-256).
  3. *Fresh run:* another model run need not match byte for byte, but must pass the same independent verifier.
- **The floorplan is part of the record.** Its operations, proposals and decisions are in the document's history and the trace, so a run can be replayed step by step, and a later reader sees why a connector sits where it does.
- **The trace** (one format with the harness's run trace) holds per round: the stage, its input digest and the plan notes; the brief and its digest, the profile digest, the revision before and after, each solver call's request digest and result digest, the report, the findings difference, tokens and time. An exact repair adds its proof file's digest.
- **Open point:** bit-identical results across native and wasm builds need the same floating-point results. Basic arithmetic is IEEE-exact on both; library functions (trigonometry) may not be. The router and geometry code call such functions in about 20 places (a grep count, not an audit). Either the decisions avoid them or both builds share one implementation. To be audited in slice A1.

## 7. Performance targets

All of these are **proposed targets, not measurements**, for the founder to confirm. Measured facts today: route index build 4.4 ms and shove step 0.29 ms median on the 10,000-via board.

| What | Target | How to measure |
|---|---|---|
| Space model build, 10,000-via board | Under 100 ms, in the background, never on the UI thread | An ignored release test beside `route_plan_timings` |
| Space model update after one committed route | Under 5 ms (local re-triangulation) | Same test, per commit |
| `plan.notes`, small board | Under 50 ms, so overlays follow a dragged edge or part | Bench and an ignored release test |
| `layout.route_connect`, one connection on a small board | Under 50 ms solver time | Bench: server time per call |
| `layout.autoroute`, the multi-tool board, solver time excluding the model | Under 10 s | Bench: server time; board size unknown until its spec exists |
| Model rounds for the multi-tool board | Report, no cap (the harness sets no default limits) | Bench: calls, tokens, replans |
| A window repair | A counted node budget from the routing profile; report how often it is hit | Trace |

For scale: PCBRouteAgent reports averages of 27 s, 146 s and 516 s per board on its small, medium and large splits with its smaller model, model calls included **[summary]**.

## 8. Evaluation plan

On `cargo xtask agent-bench` (harness slice H3), with its independent verifier over `sparksmith-io`.

**Tasks**

1. **Our boards:** the ESP32 module board (its existing route is the reference), the USB-C multi-tool once its spec exists, and the showcase.
2. **Constraint tasks no public benchmark has:** a pair that must stay on one layer with no via; a net through a tie's anchor; a wide power path past a fine-pitch connector; a deliberately unroutable board, where the pass is the correct cut and no copper.
3. **Graded synthetic boards, in the style of PCBWorld and CADET:** our own generator of original boards by pad count (to 30, to 100, over 100), 2 and 4 layers, built by adding one routable connection at a time so every board has a known solution (the published idea, our code).
4. **Public boards: only with a founder decision** (decision 9). PCBWorld's real boards come from open-hardware projects with their own licences and may embed footprints from libraries our clean-room rule excludes. Their published numbers are context, not a comparison on equal boards.

**Metrics per run:** clean pass (all connections, zero error findings, every class constraint and tie check met); routability; wirelength against the reference and against the spanning-tree lower bound; vias; the per-kind grades; solver time and wall time; tokens in, out and cached; calls and repair rounds; cuts reported; the replay digest check.

**Plan quality, measured apart from route completion**

| Question | Metric |
|---|---|
| Did the notes tell the truth? | Per `fits` note: was it routed inside its corridor (false-fit rate). Per `short` note: was it routed inside all the same (false alarm). Planned against used width |
| Was the plan followed? | Share of routed length inside its corridor; nets that left it and why; fast-lane nets with a layer change. Information, not a score to raise: a better route may leave the plan |
| Was enough planned? | Nets ripped up per board, and the share of them in corridors that were changed |
| Did planning help? | The same board routed with and without the floorplan: clean pass, wirelength, vias, repair rounds, solver time, tokens |
| Is the intent met? | Per kind, from the final copper: fast lanes (vias, length over the lower bound, reference breaks once that rule exists), power (narrowest neck against the planned width; track-only drop), ties (star and Kelvin checks), isolation distances |
| Was characterization right? | On boards whose interfaces and currents are declared, hidden from the model: inferred roles right and wrong; questions asked that were derivable; values needed and never asked |
| Were revisions good? | Seeded-fault boards with a known bottleneck (a connector on the wrong edge, a twisted bus): was the known fix proposed. Proposal rounds, parts moved more than once, tokens per stage |
| Is the plan stable? | Move one part slightly and re-plan: how many edges and corridors change |
| Does it look right? | Pictures of each run's floorplan and route for the founder; the multi-tool's floorplan against one drawn by a person |

**Arms and baselines**

- **No plan:** the router with a brief only, on a placement given in the task. Against the full process this shows what the planning stages add.

- **No model:** the scripted pipeline with a default brief. This is the baseline that tells us what the model adds (53.5 % against 94.9 % in PCBRouteAgent's ablation).
- **Step level:** a model driving today's `layout.route_step`, as the ESP32 run was scripted.
- **Full:** a model driving the whole process of section 3, from characterization to the routed board.
- **Models:** one hosted and one local in every comparison, paired fresh contexts, N repeats, median and spread, as the harness prescribes.
- **Human reference:** the hand-routed ESP32 board.

Pass bar for calling the first target done: starting from the schematic and the board outline, the full process reaches clean pass on the multi-tool board in every one of N fresh runs (N for the founder to set), and solver replay is byte-identical.

## 9. Learning track: can we train models on PCBs?

Yes, and it is worth doing, in one role only: **learned models guide the exact search; they never draw final copper.** This is the AlphaZero split (Silver et al., 2017, **[memory]**): a network proposes moves and judges positions, a search with exact rules decides. A wrong network then costs time or quality, never a wrong board, because every output still passes the validator, the router and the checks.

**The technique follows the problem.** For each sub-problem the simplest technique that measures best on our bench is used. The classical default is the baseline, a newer method has to beat it, and no method is used because it is new. Where several methods below could fill a slot, the bench decides.

### What the evidence says, by model family

| Family | Evidence | Verdict for us |
|---|---|---|
| **Diffusion, for placement** | ChipDiffusion ([arXiv 2407.12282](https://arxiv.org/abs/2407.12282), ICML 2025, **[read]**): denoises object coordinates conditioned on the netlist graph; trained **only on synthetic circuits** (40,000 in its v1 set; 100,000 generated in a day on 32 CPUs) on a single A5000 GPU, and transfers zero-shot to real benchmarks. Legality is 0.8835 unguided and 0.9970 guided, so not every sample is legal. EGGPlacer ([arXiv 2610.33099](https://arxiv.org/abs/2610.33099), **[read]**) adds energy guidance and still runs a greedy legaliser afterwards; in its mixed-size table the classical analytic placer keeps the lowest wirelength (5.64 against 6.08). MacroDiff+ ([IJCAI 2026](https://www.ijcai.org/proceedings/2026/735), search summary) combines a GNN and a Transformer in the denoiser and reports 6.1–6.2 % less wirelength | **Use as a proposal source** for whole-board placement, followed by our constraint placer as legaliser and the plan notes as judge. All of this evidence is chip placement; none is PCB |
| **Flow matching, for placement** | FlowPlace ([arXiv 2604.23658](https://arxiv.org/abs/2604.23658), DAC 2026, search summary): flow-based training, mask-guided synthetic data, hard-constraint sampling for overlap-free layouts; 10–50× faster sampling is reported by secondary summaries, unverified | **Prefer over diffusion** if the first experiment confirms the speed; same role |
| **Diffusion, for routing** | RouteDiff (section 2): images turned into traces, no numbers on its page. CLDRoute ([arXiv 2607.16674](https://arxiv.org/abs/2607.16674), **[read]**): despite the name it predicts congestion and DRC maps for chips, not routes; DRC map SSIM 0.9678, and its congestion maps are not the best on the standard metrics | **No** as a router. A map predictor with a per-pixel uncertainty is a useful idea beside the plan notes |
| **GNN / CNN routability prediction** | RouteNet (2018) was the first CNN to predict DRC hotspots from placement, and later work moves to graph models; one 2026 benchmark finds the graph representation matters more than the architecture ([arXiv 2604.08810](https://arxiv.org/abs/2604.08810), search summary; accuracy figures not verified) | **Use beside the plan notes**, and only if it predicts better than they do |
| **Transformers, autoregressive** | The LLMs of section 2 are general autoregressive transformers, good as planners. A domain policy that emits routing actions one by one is CADET's: 67.2 % against 65.6 % for a classical autorouter, with more vias | **No** for copper. A small GNN or transformer as policy and value inside the search is the use |
| **GANs, VAEs** | No recent result was found, in the sources read, where a GAN leads on placement or routing; this family was not searched in depth. VAEs appear as the latent compressor inside diffusion models (CLDRoute) | **Skip** GANs. A VAE only as a part of another model |
| **RL** | End to end it matches a classical router at best (PCBWorld's PPO: 0.45 clean pass on medium boards; CADET). Commercial RL (Quilter, DeepPCB) searches candidates for hours and still needs a person (section 2). Tuning a symbolic router's cost weights by offline RL is the result that holds up: 1.56× faster on average, and 92 % fewer violations on dense layouts (section 2) | **Use** for cost schedules and for self-play training of the policy and value |

### What to learn, and where it plugs in

Each learned part fills a harness strategy slot that already has a classical default, so it can be switched per profile and removed without changing behaviour elsewhere.

| Learned part | Model | Slot | What it may do | What it may not do |
|---|---|---|---|---|
| **Placement proposals** | Flow matching or diffusion over part positions, rotations and sides, conditioned on the netlist graph, the net profiles and the floorplan's areas | `plan.propose_placement` | Hand N candidate placements to the constraint placer, which legalises each; the plan notes rank them | Place a part. Skip the legaliser |
| **Policy** | GNN or small transformer over the space model's graph (portals as nodes, nets as queries) | `route.order`, `route.portal_prior` | Order the nets, order the portals the A* tries first, pick which net to rip up | Remove an option from the search. It reorders only, so the search stays as complete as the classical one |
| **Value** | The same network's second head | `route.value` | Estimate whether a partial topology will complete and at what cost, to order the priority search | Prune, in the first version (decision 27) |
| **Routability predictor** | GNN on the floorplan's areas and corridors, with an uncertainty | `plan.predict` | Add a note, marked "learned" with its uncertainty | Stop or change anything |
| **Cost schedule** | Offline RL over the routing profile's weights | `route.cost` | Choose weights per round | Change a rule or a limit |

**Determinism.** A network's output is treated like the language model's: it is a recorded proposal. The trace stores the weights' hash, the sampling seed and the output, and solver replay stays byte-identical given the recorded output. Fresh inference need not be bit-identical across machines.

**Weights are library data,** pinned by hash with a provenance record (training code revision, data generator revision and seed, bench results), loaded like a routing profile. The product works without them.

### Data

1. **A synthetic graded board generator** (slice L0). Boards are built by adding one routable connection at a time, so each has a known solution; parts come from our own generators and the interface catalog, so nothing from an excluded library enters. Both ChipDiffusion and CADET trained on synthetic data alone and transferred to real designs, which is the reason to trust this route. The generator's distribution is fitted to statistics of our own boards.
2. **Self-play.** Our deterministic router and exact verifier are the environment. Runs with varied orders, costs and placements yield labelled outcomes: completed or not, rounds, wirelength, vias, the cuts that failed. Search results become the policy's and the value's training targets, as in AlphaZero. Every label is machine-checked, so there is no label noise; few learned-EDA projects have that.
3. **Opt-in real-use data, later.** Traces with floorplans, proposals and the person's accept, decline and alternative decisions are preference data nobody else has. Off by default, local by default, never from a private board without an explicit choice per project (decision 26).

Not used: open-hardware boards collected from the web, for the clean-room and licence reasons of decision 9.

Throughput is unmeasured for our engine. Reference points: 100,000 synthetic circuits a day on 32 CPUs (ChipDiffusion), and 100,000 routed boards in about 146 hours on a 56-core server through a full EDA engine (CADET).

### Infrastructure and cost

| Option | Known | Unknown |
|---|---|---|
| **The Linux build server** | A 32-thread desktop-class CPU with an integrated GPU and 125 GB RAM (checked 16 November 2026). Enough for data generation, self-play, evaluation and CPU inference | Whether PyTorch trains on this GPU under Linux. AMD lists these APUs as preview support, one page says "no ML training support" under a Windows heading, and a community report describes a memory fault ([ROCm docs](https://rocm.docs.amd.com/projects/radeon-ryzen/en/latest/docs/limitations/limitationsryz.html), search summary). A 2–3 hour spike settles it (L2) |
| **The local model server** | A local machine already serves models through vLLM | Its GPU, memory and whether it may be used for training |
| **Cloud** | On-demand medians in autumn 2026: about $1.60–1.64 per GPU-hour for an L40S and $3.79–3.92 for an H100, with providers differing several-fold ([AIMultiple index](https://aimultiple.com/gpu-index), [getdeploying](https://getdeploying.com/gpus/nvidia-h100-vs-nvidia-l40s), search summaries) | Our GPU-hours |

No GPU-hour figure is estimated here, because nothing comparable has been measured on PCB-sized graphs. The models are small (a board has hundreds to a few thousand nodes; ChipDiffusion trained on one workstation GPU). The proposal is a **cap, not an estimate**: at most 100 GPU-hours for the first experiment, which is about $160 on an L40S or about $390 on an H100 at the medians above, to be set by the founder (decision 28). Training code is dev tooling outside the product; inference in the product runs on the CPU in Rust (the crate choice and its licence go through `cargo xtask licences`).

### The gate: when a learned strategy may replace or augment a classical one

On the bench of section 8, with boards from generator seeds the training never saw plus our real boards, paired runs, N repeats, median and spread:

1. **No harm:** clean pass is not lower on any split, and no real board regresses.
2. **A measured gain** on the slot's own metric (repair rounds, solver time, wirelength, vias, false-fit rate), with spreads that do not overlap.
3. **Runs for everyone:** within the performance targets on a CPU with no GPU.
4. **Fallback proven:** with the weights removed, the result equals the classical strategy's.
5. **Replay holds** with the recorded outputs.

All five: it may become the default. The first, third, fourth and fifth, with a gain on some splits only: it ships as an option. Otherwise it is deleted, as the harness says of any strategy that loses.

### Slices

| # | Slice | Delivers | Acceptance | Hours |
|---|---|---|---|---|
| **L0** | Synthetic board generator | Graded boards with known solutions from our own parts and the catalog | Every generated board passes the verifier with its own solution; splits by pad count and layers | 10–15 |
| **L1** | Trace to dataset, and self-play | An exporter from run traces to training records; a runner that varies orders, costs and placements on generated boards | A dataset with machine-checked labels; boards per hour measured on the build server | 8–12 |
| **L2** | Training spike on the build server | A small model trained on its GPU, or the reason it cannot | A yes or no, with timings | 2–3 |
| **L3** | Policy and value for net order, portal choice and rip-up | A GNN, trained by self-play; inference in Rust; the slots | The gate, on the bench | 20–30, plus compute under the cap |
| **L4** | Routability predictor for floorplans | A GNN on the areas and corridors with uncertainty | A lower false-fit rate than the plan notes on held-out boards | 15–25 |
| **L5** | Placement proposals | Flow matching conditioned on netlist, profiles and areas; legalised by the placer | The gate: fewer short notes after routing or fewer revision rounds than the placer alone | 30–50, research risk |
| **L6** | Learned cost schedule | Offline RL over routing profiles from accumulated traces | The gate | 15–25 |

L0 and L1 are cheap and should start as soon as the global router exists (after A4a), so data accumulates while the rest is built. L3 to L6 need the router of waves 3 and 4 as their environment.

## 10. Risks and open research questions

- **Topology that has no 45° geometry.** A corridor can be wide enough for any-angle copper and still fail in the posture. Mitigation: the capacity test uses the posture's worst-case width, and a failed realisation is fed back as a reduced capacity for the next round. Whether that converges quickly is unmeasured.
- **Capacity with mixed widths and clearances** is exact for parallel tracks through one portal, and approximate round pad corners (the diagonal problem Yan and Wong fixed for arrays). Too optimistic wastes rounds; too pessimistic refuses routable boards. Needs tests against the exact checker.
- **Walkaround's convex hulls** do not thread narrow gaps (6 of 400 arrivals on the dense grid). The funnel waypoints should remove most of this; if not, the visibility search named in push-and-shove's follow-ups is needed.
- **Pours and return paths.** Fills are not obstacles and refill round new copper. A track across a plane layer can cut the return path of another net. Pricing plane crossings is easy; checking "unbroken reference" needs a definition the founder accepts (decision 5).
- **Power.** A 5 A path is often a pour, not a track. The first version routes wide tracks from the class width; planning pours and proving their current capacity needs the resistance solver that llm-geometry parked as a future idea.
- **The first board's limits are unknown.** USB4 skew, length and via limits must come from its spec, with sources. None is assumed here.
- **Plans that look reasoned and are wrong.** A model writes fluent rationales. Mitigation: a rationale names its facts and one without says so, numbers are computed by the engine, and inferred values cannot drive a hard check.
- **Too many questions.** A planner that asks for every unknown is unusable; one that asks nothing guesses. Mitigation: ask only for a value a short or unestimated note depends on, in one grouped card, and measure both errors on the bench.
- **The floorplan is rough, and some of it will be wrong.** A corridor that fits on paper can fail in routing. The answer is a cheap repair (change that corridor, route its nets again), not more checking beforehand. The false-fit rate and the nets ripped up are bench metrics.
- **Too many areas to read.** Corridors and areas on every layer can hide the board. The per-layer switches, the colour mode and one layer in view at a time are the mitigation; pictures of real boards decide.
- **Sparse pin data.** Parts made before the three extensions of 3.2, and parts whose datasheets do not state a value, have no electrical characteristics or interface reference. Characterization then asks. The process still works, with more questions.
- **Small models.** The brief and the report are short, but choosing among repair options needs judgement. The no-model arm bounds the damage: the pipeline still routes what a default brief can.
- **Dependencies:** the three pin-data extensions of 3.2 (for declared profiles), the pair catalog reaching `main` from `integ-1`, the constraint placer (harness H10, for rough placement), ties (for star and Kelvin targets), harness H3 (bench) and H6 (findings difference), S4b (jobs) for progress and cancel.
- **Scale beyond the first boards** (BGAs, 6 or more layers, hundreds of nets) is untested by everyone: the public LLM results stop at two layers, and the best commercial result needed hand escape.
- **Open questions:** is a learned cost schedule worth it at PCB scale; can the cut certificates be made minimal cheaply; does any-angle realisation (arcs) with a later octilinear option beat 45° first.

## 11. Founder decisions

1. **The model plans and repairs; it never draws copper.** Its output is a typed brief, and copper comes only from the engine's router. *Recommendation: yes.* Every 2026 result points this way.
2. **One topological space model (a triangulation per layer) as the shared planning index** for the model, global routing, escape and repair, derived per revision from the physical inventory and the rules snapshot. *Recommendation: yes.* It decides neither connectivity nor committed clearance, and an xtask gate should hold that.
3. **Detailed routing stays the existing push-and-shove router**, fed with waypoints; no second geometry generator. *Recommendation: yes.*
4. **Durable routing constraints are net-class rules** (allowed layers, via limit, current), checked by the board checks; the brief holds only preferences for one run. *Recommendation: yes.*
5. **"Needs unbroken reference copper" as a net-class rule**, defined as: no stretch of the net's track lies over a gap in the fill or plane of its reference layer. Unchecked when unset. *Recommendation: yes, after a short definition note*, since the fill's exact outline and the tolerance at pads need care.
6. **Minimum width from current only with a named formula and a user-stated temperature rise.** *Recommendation: yes; until both exist, the class's own width is used and nothing is checked.* Please say which standard's formula you want sourced.
7. **Routing profiles as data** (`library/routing/`), including the existing router's budgets. *Recommendation: yes.*
8. **An exact, proof-logging solver for window repair**, as a new dependency (candidate: Pumpkin, MIT OR Apache-2.0, pure Rust). *Recommendation: decide after slice A4b*, when the bench shows how many boards negotiation leaves unsolved. On the first board it may not be needed.
9. **Public benchmark boards.** *Recommendation: no import for now.* Build our own graded generator and report on our own boards. If you want a like-for-like number later, CADET's synthetic set is the candidate (reported as BSD-3-Clause; unverified), after a licence check.
10. **No GPU, diffusion, differentiable or end-to-end RL routing in release 1.** Learned models come in only as guides inside the exact search (section 9). *Recommendation: yes.*
11. **Pin and gate swap** only where the library declares the pins interchangeable. *Recommendation: yes, and later*; nothing is inferred from names.
12. **The pass bar:** N fresh runs clean on the multi-tool board plus byte-identical solver replay. *Recommendation: N = 5; please confirm or set it.*
13. **Performance targets** of section 7. *Please confirm or change them.*

Raised by the staged plan (section 3):

14. **The name "floorplan"** for the persisted plan, with a glossary entry, since "board plan" and "route plan" are taken. *Recommendation: yes*, or name it yourself.
15. **The floorplan is stored in the project file and changed only by typed operations in a new `plan` module.** *Recommendation: yes.* It follows undo and revisions, and a grant can allow planning without copper.
16. **Net profile fields carry a basis, and an inferred value never drives a hard check or a width**; it is confirmed by the person or sourced from a datasheet first. *Recommendation: yes.*
17. **Net profiles build on the existing pin model, with three optional extensions and no new pin structure** (3.2): an `electrical` field on `Function` (voltage, current, rate or edge, each sourced), a typed `interface` on `Resource` with `interface_role` on `Role`, and the pair catalog extended in place into the interface catalog. `Resource.kind`, `Role.signal`, labels, mux codes and speed grades stay free text. *Recommendation: yes.* One choice inside it: keep the file name `differential-pairs.json` or rename it `interfaces.json` now, before release. *Recommendation: rename*, since it will hold more than pairs.
18. **Who accepts in an unattended run.** A grant setting lets an agent decide its own proposals (used by the bench and autonomous builds); by default a person does. *Recommendation: yes, off by default.*
19. **No invented thresholds in the plan notes.** A note reports need, have and margin. It is never an error and stops nothing.
20. **Placing uses the harness's constraint placer (H10), pulled forward,** with its own short design. *Recommendation: yes.* Without it the placing stage has no solver and the model would place by coordinates, which the evidence says fails.
21. **Overlays in 2D and in the peeling view,** with a switch per layer. Hulls are optional (decision 36).
22. **The process is a workflow as data** (`plan-and-route`), the same for the chat and external agents. *Recommendation: yes.*
23. **Voltage drop is estimated for track branches only** until the resistance solver exists; pours are marked not estimated. *Recommendation: yes.* Say if the solver should move up for the 48 V / 5 A path.
24. **Area uses are a closed set in code** (solvers treat each differently); functions with their letters and colours, and the profile-to-plan defaults, are data. *Recommendation: yes.*

Raised by the learning track (section 9):

25. **Learned models are policy, value, proposal and predictor inside the exact search, and never draw final copper or place a part unchecked.** *Recommendation: yes.*
26. **Data: synthetic boards and self-play first; real-use traces later, opt-in per project, local by default; no boards collected from the web.** *Recommendation: yes.* Say whether opt-in data may ever leave the user's machine.
27. **The value network orders the search and does not prune** in the first version, so the learned router is as complete as the classical one. *Recommendation: yes.*
28. **Compute:** try the build server first (L2); otherwise cloud under a cap you set. *Recommendation: a cap of 100 GPU-hours for the first experiment (about $160 to $390 at autumn 2026 medians).* Also say whether the local model server may train.
29. **The gate of section 9** decides default, option or deletion for every learned strategy. *Recommendation: yes; please set N with decision 12.*
30. **Weights are optional library data pinned by hash**; the product routes without them. *Recommendation: yes.*
31. **Timing:** L0 and L1 start once the global router exists, so data accumulates; L3 to L6 after wave 4. *Recommendation: yes*, with L3 (net order and rip-up) first, since net order is the failure the public agents report.

Raised by the floorplan (section 3), with the [mock-up](floorplan-mockup.html):

32. **Colour of the overlays.** *Recommendation: the layer pastel, with lightness steps between neighbours, letters and names for the function, a hatch per layer in the peeling view, and a key for function colours on the layer in view.* It is the only mode that both says the layer and keeps every pair of neighbours apart in all colour visions measured.
33. **Where a rough position lives.** *Recommendation: in the floorplan, and only while the part is not on the board.* The alternative, a rough flag on a placed part, would make rough parts copper obstacles.
34. **A drag that reaches another edge.** *Recommendation: it stops there.* Pushing the neighbour along would move edges the person did not touch.
35. **Areas of one layer do not overlap;** corridors lie over them, and an area may have several uses. *Recommendation: yes.* Shared edges need it. The price: "quiet inside the processor's area" is a separate small area, not a second one on top.
36. **Hulls.** *Recommendation: yes, as their own later slice (P7)*, after the 2D plan, with the linked design's real shape above the board rather than one height per area. It starts with a short design, because reading that room from a linked design does not exist.
37. **How strongly routing prefers the plan.** *Recommendation: a cost weight in the routing profile (data), chosen by the bench*, with fast lanes routed first. No hard wall at any setting.

## 12. Slice plan

At most 5 agents at once. Each slice is useful alone. Planning slices (P) come first where they decide routing quality: after wave 2 a person or an agent can already draw, drag and discuss a floorplan on every layer, before any autorouting exists. Everything through wave 4 is what the multi-tool board needs; it also needs the ties slices and the board's spec.

| Wave | # | Slice | Delivers | Acceptance | Hours |
|---|---|---|---|---|---|
| 1 | **A1** | Space model and `layout.route_space` | Triangulation per layer from the inventory; portals with width and class capacity; via sites; compact encoding; the float audit of section 6 | Capacity agrees with the exact checker on generated channels (never claims room the checker refuses); 10,000-via board build timed; an xtask gate that the module makes no connectivity or commit decision | 12–18 |
| 1 | **P1** | Net profiles | The three extensions of 3.2 (`Function.electrical`, `Resource.interface` with `Role.interface_role`, the catalog's ids, roles and extra targets), with the pinout import's columns; `rules.net_profiles` from active functions, resources, classes, pairs and stackup, each field with its basis; confirmed values stored in the rules with a source; the grouped question card | Existing parts and files load unchanged; a part declaring an interface profiles its nets as declared with no name matching; on the ESP32 board the USB pair, the rails and their sources and sinks are profiled with no inferred field used by a check; an unknown that a stage needs is listed once | 12–18 |
| 1 | **P2** | Floorplan model and the `plan` module | The stored types of 3.1 with the map per layer, the area, edge, corridor, power and intent operations, locks, `plan.get`, `plan.diff`, file round trip, glossary | One edge operation reshapes both its areas and leaves no gap; a locked edge is moved by nothing; a corner on the board edge follows an outline edit; save and undo keep it; replaying the operations gives the same floorplan; `surface.lock` updated | 16–24 |
| 1 | **P3a** | Plan mode: UI design | Overlays, colour modes, Plan card, proposal card and preview, from the mock-up's reviewed state | Approved | 2–3 |
| 2 | **P4** | Plan notes | `plan.notes`: the rows of 3.4 with need, have and margin, except height | Follow a drag within the target of section 7; a seeded narrow corridor reads `short` with the right numbers; nothing is computed from an unset limit; no note reaches Issues or preflight | 6–9 |
| 2 | **P3b** | Plan mode: build | Overlays with per-layer switches in 2D and the peeling view, the colour modes, the Plan card, rationale on hover, the proposal preview, drag and lock | Headless canvas tests: showing or hovering overlays rebuilds no geometry; an edge drag sends one operation; the preview draws what the diff lists; the colour vision tests cover every mode | 20–30 |
| 2 | **A3** | Routing constraints as rules | Net-class layers and via limit, their checks and Issues wording | A pair held to the top layer is flagged when drawn through a via by hand | 6–10 |
| 2 | **P5** | Rough and legal placement | `plan.rough_place`, and the constraint placer (harness H10; its design first) behind `plan.place` | Rough positions follow the intents at once; placed parts are legal and near them; an intent that cannot be met is listed with the amount and the other parts are placed | 2–3 + 17–28 |
| 3 | **A4a** | Global router | Negotiated topological routing over all open connections, floorplan as a cost, priority search on stall, no copper; routing profile as data | Deterministic; on the ESP32 board's placement it finds a topology for every net; an unroutable fixture returns the right cut; a net leaves its corridor when that is cheaper | 10–15 |
| 3 | **P6** | Revisions and the chat | `plan.propose`, `plan.decide`; solver-made proposals; the proposal card with Accept, Alternative and Why; the `plan-and-route` workflow as data with the planner's questions of 3.3 | On a seeded-fault board the known fix is proposed, accepting it changes the note, and a hand alternative replaces it in the log; an MCP client drives the same stages | 12–18 |
| 3 | **A2** | `layout.route_connect` | Topological A* for one connection or pair, funnel waypoints, realisation by `router::plan` and `pair_plan`, one undo step, cut feedback; `layout.unroute` | The ESP32 board's nets route by goal with no coordinates; a blocked request names the cut; the same request twice gives the same board | 15–25 |
| 4 | **A4b** | `layout.autoroute` and `layout.route_report` | Detailed routing of the global result preferring the floorplan, rip-up of the nets concerned only, the report with the nets that left their corridors, findings difference | The no-model arm routes the ESP32 board clean; the report names every net outside its corridor and why; a widened corridor reroutes only its nets; solver replay byte-identical | 12–18 |
| 4 | **A5** | Bench: tasks, arms, plan quality | The tasks and metrics of section 8, including the no-plan arm and the seeded-fault boards. Needs H3 | The first full report per arm, hosted and local: clean pass, plan-quality table, tokens and time per stage | 12–18 |
| later | **A6** | Pairs to their limits | `layout.tune_pairs`; tuning room reserved by the floorplan's `reserve` areas | Pairs end inside their class limits, or the report names the room missing | 6–10 |
| later | **A7** | Escape by flow | `layout.fanout`; the pin-order check of the layer budget made exact | The USB-C receptacle and a 0.4 mm QFN escape clean | 10–16 |
| later | **A9** | Exact window repair | The CP or CBS step with proofs (decision 8) | Fixes a bench board negotiation could not; an infeasible window returns a checked proof | 15–25 |
| later | **A10** | Reference and power | The unbroken-reference rule (decision 5); current-driven width (decision 6); a design note for pours and the resistance solver | Design first | 3–4 + later |
| later | **P7** | Hulls | The room above the board from the linked design (its design first), hulls in 3D, the height note | A part too high for a test enclosure is marked with both heights; without a link nothing is shown | 2–3 + 12–20, research risk |
| later | **A11** | Strategies as WASM | After S9a | A WASM `plan.order` gives byte-identical results to the data profile | 8–12 + research |

Waves 1 to 4 take about 154–237 agent hours. Wave 1's four slices run in parallel. P4 needs P1 and P2; P3b needs P2 and the approved design; A4a needs A1; P6 needs P4; A4b needs A2 and A4a; A5 needs A4b and H3. The shortest useful cut is waves 1 and 2 (about 93–143 hours): a visible, draggable floorplan on every layer with its notes, driven by hand, by the chat or by an external agent.

The learning track (L0 to L6, section 9) runs beside the later slices: L0 and L1 after A4a, the rest after wave 4.
