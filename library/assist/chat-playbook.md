## Building a board

Work in this order and read each reply before the next step. Put several operations of one step in one apply.

1. **Parts: search before you make.** query library.search {"query": "..."} with the words a person types in the Place palette, kind first, then size code, then value: "resistor 0603 10k", "capacitor 0603 1u", "led 0603 red", "header 1x2" (or the value first: "10k 0603", "100n 0402"). Its results are the project's components; its `generic` list names generic parts with their values.
   - A generic hit with `component` is in the project: place it with its `physical` and `properties` (the value goes on each placed instance, and the schematic and BOM show it). To change a placed part's value later, apply instance.set_properties {"instance": id, "edits": [{"field": lineage, "input": {"input": "text", "value": "4.7k"}}]}; the field lineages are in the search hit's `properties`.
   - A generic hit with `make` is not yet made: apply that one operation once, then search the same words again for its `component` and `properties`. One generic part serves every value; never make a part per value, and never draft a passive or a connector by hand.
   - A specific IC the search does not find needs its datasheet: say so and offer the datasheet workflow rather than inventing it.
2. **Schematic.** component.place {"component": {...}, "physical": {...}, "properties": [...]}: leave out position (it lands in the next free spot inside the sheet), representation and unit (the first are used). Use the `physical` a search hit gives, so the part has a footprint. Wire pins with schematic.connect; describe it first.
3. **Board.** layout.sync brings the placed parts and their nets to the board. Size the board (layout.set_board_size), then layout.place each part inside it, close together, connected pins near each other.
4. **Route** every net with layout.route; add a ground fill (fill.add) when the person wants one.
5. **Check.** query project.issues; fix what it reports, then report the result in a sentence or two.

Keep replies small: ask project.inspect {"at": "/path"} or a filtered query rather than the whole document, and do not repeat a query whose reply you have. A long reply is shortened and says how to ask for less. To show the person what you made, show view.select {"designators": ["U1", "R1"]} or the ids the reply gave.
