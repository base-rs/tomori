# The Farmyard's Trail

A walking trail loops round the outside of the fence: out through the gate, along the front to the right, down the yard's right side along the river's bank, round behind the coop to a lookout over the waterfall's pool, with a log seat, along the edge of the bush on the left, and back to the gate, about 61 m. Five exercise stations stand beside it, each a post with a plaque that says what to do there, standing, with a headset on. It is all in `trail.rs`, laid out from `trail::WAYPOINTS`.

## Where it goes

`trail::centreline()` is the loop, smoothed through `WAYPOINTS`, `STEPS` points to each, the first at the gate; `trail::frame(index)` gives a point of it, the way along it and the way out of the loop. Its track is `WIDTH`, 1.2 m, from crest to crest, all of it walkable. It keeps:

- **within `REACH`**, 11.5 m of the backdrop's middle, so that the whole backdrop stands within the headset's 50 m far plane, 2 m short of it, from anywhere on it, as from the yard; which is why the foothills' last ring is at 36 m. That is also inside the foothills' first ring, so it lies on the level meadow.
- **clear of the river's banks and the pool's rim** (`backdrop::BANKS`, `RIM`), which stand above the water, so that it never climbs them: the river within 7 m of its middle for 12 m and more, the waterfall and pool seen from the lookout.
- **clear of the fence**, its inner edge at least 0.9 m from it, of the pines and of the bush, and out of the views the backdrop keeps open.

## Walking it

It is declared as walkable surfaces at the yard's height, `HEIGHT`, so that nothing steps at the gate: the loop's band in `STRETCHES` polygons, each overlapping the next, since one outline cannot have a hole; the way out, `GATEWAY`, a rectangle from the yard's earth between the gate's posts to the loop; and the clearings, `clearings()`, discs, one at each station and the bay at the lookout. The client crate's `locomotion()` declares them with the yard's rectangles. A pair of teleport pads, `pads()`, joins the yard, beside the way to the gate, and the lookout, in its bay, off the trail so that nobody walking or teleporting along it lands on one. The hens keep to `flock::AREA`, well inside the fence and short of the gate, startled or not.

## The stations and places

`STATIONS` are, in order from the gate: 10 squats; shoulder and neck stretches, 30 seconds; 20 calf raises; 10 lunges each side; and arm circles and a deep breath, 30 seconds. None is on the floor or leans on anything, since the posts and rails are not there to lean on. Each has a clearing joined to the trail, on the side with room within `REACH`, its post at the clearing's edge facing it, and its plaque, `plaques()`, which the client crate draws in text from the renderer's glyph atlas.

`places()` are the named anchors Tomori's own server declares (`tomori_server_app::farmyard_trail`): `riverbank` and `pool-lookout`, kind `viewpoint`, and `station-1` to `station-5`, kind `station`, labelled with their exercises, so that the agent can suggest a walk, go to a station and say what to do. A station's anchor stands behind its post, facing its clearing. Anchors take no radius yet; once they do, for the agent to be told as someone arrives, a station's is `STATION_RADIUS`, 2 m. `solids()`, the posts and the seat, join the yard's, so that the agent, walls and panels keep clear of them.

## Drawing it

`trail::model()` is one model: the track, `LIFT` above the meadow, with edges `EDGE` higher, open where the way out or a clearing meets it; the clearings; a few stones along its edges; the posts and their boards; and the seat. It is counted in the backdrop's budget (see `backdrop.md`).

## Changing it

Move it through `WAYPOINTS`, and the stations by the point of the loop each stands at. Run the farmyard's tests: they check that it stays within reach, outside the fence and clear of the banks, the bush and the views, that it is walkable end to end, through the gate, without a gap or a step, that the stations are spaced, beside it and labelled, that the pads are off the way, and that the hens stay in. Set `TOMORI_FARMYARD_TRAIL_PREVIEW` to a PNG path when running the client crate's tests to save the trail by the river, the gate, the lookout and a plaque, as `-1.png` to `-4.png`. A change to the trail's anchors, or to what an earlier client would not draw, raises the farmyard's version.
