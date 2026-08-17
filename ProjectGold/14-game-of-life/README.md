# 14 — Conway's Game of Life
**Tests:** grid state, generation stepping, boundary conditions. **Result:** works, all three engines agree.

Two things make this real rather than decorative. A generation must be computed
from a snapshot — updating in place makes the top-left see next-generation
neighbours and the bottom-right see this-generation ones, producing plausible
garbage that survives casual inspection. And the boundary: reading off the grid
is exactly the bounds error the exercise exists to catch.

Checked against known results rather than by eye:

* a block never changes, over five generations;
* a blinker has period 2 — differs at gen 1, identical at gen 2;
* **a glider is its own shape displaced by exactly (1,1) after four
  generations**, with population conserved. That is the strongest check here and
  it passes.

The corner cell has 0 neighbours, not 3 — off-grid is dead, stated once in
`at()` so no caller has to remember it. `0x0` and `1x1` grids step without
dividing by zero.

## Findings
None.
