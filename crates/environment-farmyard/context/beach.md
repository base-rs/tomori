# The Farmyard's Beach

Beside the trail where it passes the river, by the `riverbank` viewpoint, a shingle beach slopes from the trail's edge down into the river, with stones on it that participants take with either hand, carry, throw and stack. It is all in `beach.rs`, which the server side (`side/shingle.rs`) simulates with `tomori-physics` and the client crate's `stones.rs` mirrors.

## Its stones

Stones are numbered from 0: the river's first, `RIVER` of them, of four kinds (`Kind::Slab`, `Cobble`, `Long` and `Angular`), half of them `big`; then the building stones, `BUILDING` of them, `building(index)`, five `Kind::Flagstone`s and three `Kind::Block`s, sized by `hewn` and shaped by `dressed`, boxes bevelled and nudged, flat below and level above, for lintels, tables, roofs, pillars and walls. `stone(index)` is any stone's hull, from which it is both drawn and collided, every face of four corners at most, as the physics harness found Rapier needs; `size`, `volume` and `outside` measure it. `homes()` lays them out: the river stones scattered, more towards the water, and the building stones piled and in a row (`building_homes`, `BUILDING_OUT`, `PILE_ALONG`, `BLOCKS_ALONG`), beyond the walked shingle and clear of the trail. `heft(index)` is how much of the hand's speed a stone leaves it with, by its weight, and `fastest(index)` the speed the server side holds a throw to, so that a building stone cannot be flung.

## Its world

`colliders()` are what the stones rest on, `waters()` the river beside it, and `settings()` the world's, which solves each contact against the shingle's own triangle (`Settings::unclustered`), so that a broad stone lying across a bend rests rather than creeping. `bodies()` are the stones as bodies.

## Changing it

Add a stone by raising `RIVER` or `BUILDING`, which raises `STONES`, within `tomori_physics::BODY_LIMIT`, and the farmyard's version, since an earlier client would draw it as another stone or not at all. Run the farmyard's tests: they check each stone's hull, the homes, the budget, that the beach settles and then costs nothing, and that cairns and structures of its stones (a dolmen, a flagstone on a cairn, a short wall and a tower) stand for ten simulated minutes, while a lintel beyond its pillar falls. Set `TOMORI_FARMYARD_BEACH_PREVIEW` to a PNG path when running the client crate's tests to save a little dolmen of the building stones on the beach.
