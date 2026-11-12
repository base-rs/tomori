# The Farmyard's Upper Terrace

A private place at the top of the waterfall, on the mountain behind it, which participants reach only by pointing at the falling water from the waterfall rock, and leave by pointing at the cairn by the lip, which takes them back onto the rock. It is the farmyard's second level, a walkable region of its own 7 m above the rest, which the agent's following across levels is tried on.

## What is there

`terrace.rs` holds it, said along the waterfall's way from the backdrop's middle and across it, to the right as one faces the fall from the yard (`terrace::on_fall`, `backdrop::on_the_fall`):

- **Its floor**, moss and stone, flat at `layout::TERRACE`, 6.9 m, 7.2 m above the pool, within `terrace::OUTLINE`, about 8 m across the cliff's top and 4 to 5 m deep, about 33 m², from a little behind where the cliff falls away to short of the upper cliff's foot, and from the stream's bank to the mountain's right side. Its faces are even triangles about 0.6 m across (`terrace::SPACING`, `terrace::faces`), a constrained Delaunay triangulation of a lattice within its outline and of its outline cut into pieces no longer (`terrace::edge`), refined so that no angle is under 25°, no corner shared by more than eight faces, each shaded moss or stone as the floor is about it; it was a fan from one point, which the maintainer saw on the headset as a star before them. Its sides, cut along the same pieces, reach `terrace::SIDES`, 0.8 m, down into the mountain, so that no gap shows beneath its edge where the cliff's top falls away a little sooner. Its top is drawn as ground (`terrace::ground`), so the farmyard's walkable ground has it, less what stands on it, about 25 m².
- **The water**: the upper falls drop 2.9 m from the upper cliff's lip (`layout::UPPER_FALLS_LIP`) into the upper pool at its foot (`layout::UPPER_POOL`), whose overflow runs as a stream straight to the waterfall's lip, at the level the waterfall leaves it, so that the two falls are one stream (`backdrop::upper_water`, drawn with the rest of the water).
- **A flat rock to sit on** (`terrace::SEAT`), at its front, looking out over the yard; **the cairn** (`terrace::CAIRN`), four flat stones by the lip, the way down; eight of the river's stones at its edges (`terrace::stones`), drawn with the river's; and ferns (`terrace::ferns`), a ponga at its back corner, its crown high over the floor, a mamaku across the stream over the waterfall's lip, seen from the rock below, and three ground ferns at its corners, drawn with the ferns. All but the mamaku are `terrace::solids`, which the walkable ground keeps clear of and Tomori's own server adds to the farmyard's solids.

## Getting there and back

Two teleport targets (see Tomori's `design/locomotion.md`), `terrace::targets`, which the client's teleport profile declares after the trail's:

- **Upper terrace**: the waterfall, an upright cylinder of 1 m about the lower 4 m of its fall, as tall as a target may be, pointed at only from within `terrace::FALL_REACH`, 5 m, of its foot (`Target::within`), which the rock's top is and nowhere else participants stand, the terrace included; it takes one onto the terrace at `terrace::ARRIVAL`, facing the upper falls.
- **Waterfall rock**: the cairn, pointed at only from within `terrace::CAIRN_POINTED`, 4.5 m, of its foot, from most of the terrace and nowhere below; it takes one back onto the rock's top, which is a floor because the rock's own target arrives on it, facing the waterfall.

Nothing else joins the terrace to the rest: the nearest other walkable ground is 13 m away, a backstep on it steps along it or nowhere, and a teleport aimed with the stick from below never lands on it, since its floor is above every eye there and a ray lands only on a surface it meets from above. From the terrace, a teleport aimed at the trail below, within reach, lands there, as anywhere.

Its anchor, `upper-terrace`, a viewpoint (`terrace::place`, among the trail's `places`), stands 1.6 m behind whoever arrives there, back from the upper falls they face, 1.8 m above the floor, facing them, with the trail's room, so that whatever gathers there is beside them rather than before the falls, as the rock's anchor is.

## Within the space's bounds

The top of the falls is 24 m from where participants arrive, on the way they face, beyond the 20 m the space's bounds were; they are 30 m now (Tomori's `anchors::REACH`, and the server's), which takes in the terrace, its anchor, its targets and where the agent comes to anyone on it, 28 m away at most. In the farmyard's own coordinates the terrace is beyond 30 m of its origin; only carried to where participants arrive is it within them.

## Changing it

Move its corners in `OUTLINE`, keeping its front behind where the cliff falls away and its back short of the upper cliff, and its stones, ferns, rock and cairn at its edges, so that its walkable ground stays about 25 m². Run the farmyard's tests: they check that it is a floor of its own at its height, on the mountain, covered once by its faces, even and well shaped, its edge theirs, the mountain beneath it, joined to no other ground and with no drop within 0.6 m of anywhere on it, behind the lip and clear of the water; within the space's bounds; that its stones, ferns, rock and cairn stand on it and are kept clear of; that the waterfall takes one up from the rock alone and the cairn back down from the terrace alone; that its anchor fits one to eighteen workers; and that the upper falls fall into the upper pool, whose stream is the waterfall's lip. The client's tests check the same through the teleport profile as carried into a space, and, on lavapipe, draw the waterfall from the rock, the upper falls from where one arrives and the yard from the flat rock; set `TOMORI_FARMYARD_UPPER_TERRACE_PREVIEW` to a PNG path to save them as `-1.png`, `-2.png` and `-3.png`.
