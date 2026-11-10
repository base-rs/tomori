# The Farmyard's Workbench

The agent's home in the farmyard is its workbench, in the yard's front left corner, where the fence by the gate meets its left side: a wooden bench curving through 100° about the middle of its curve, `layout::WORKBENCH`, its concave side facing out of the corner towards where participants arrive (`layout::workbench_facing`), with a tool rack behind it. The agent's home, `layout::AGENT_HOME`, floats 1.5 m above that middle, and its workers stand in the ring about it, 1.4 m away, over 300° of it (`layout::HOME_SWEEP`), so that none stands straight behind the agent, hidden by the swarm; the two of each row's six behind the agent, as seen from the arrival, hover over the bench either side of its middle, and, compact, four of each row's ten, over the bench and by its ends.

## Where it stands

Everything about it is laid out from `layout.rs`: `on_workbench(radius, turn)` gives the point `radius` from the middle and `turn` radians along the curve from straight behind it, positive towards the gate. The bench's top is `WORKBENCH_TOP` from the middle, at `WORKBENCH_HEIGHT`; nothing on it reaches above `WORKBENCH_CLUTTER`, below the workers hovering over it; the vice stands `VICE` proud of its front; and the rack stands within `RACK`, `RACK_HEIGHT` tall, its back 8 cm inside the fence.

Its solids, which the agent, walls and panels keep clear of, are its top in `WORKBENCH_PIECES` pieces and its rack in `RACK_BOARDS`, one for each of its flat boards, each the box about its corners; its footprints, which nobody lands on and the chickens walk round, are `layout::workbench()`. The scenery draws it in `workbench.rs` from the same constants, within those solids, so that what is drawn and what is kept clear of agree, which its tests check.

## Changing it

Move it, or change its size, through `layout.rs`, and run the farmyard's tests: they check that one to eighteen workers fit the ring about the agent clear of its solids and the fence, that those behind the agent stand over the bench and none straight behind it, that the walkable rectangles stay clear of it, and that everything it draws stands within its solids. The ring's radius is Tomori's, `tomori_protocol::cluster::RING`, so a bench nearer the agent than its rack's reach allows fails them. Something drawn on the bench taller than `WORKBENCH_CLUTTER`, or on the rack before `RACK`, stands where the workers do. A change to where the agent's home is, or to what an earlier client would draw about it, raises the farmyard's version.
