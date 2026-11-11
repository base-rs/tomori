# The Farmyard's Trail

A walking trail loops round the outside of the fence: out through the gate, along the front to the right, down the yard's right side along the river's bank, round behind the coop to a lookout over the waterfall's pool, with a log seat, along the edge of the bush on the left, and back to the gate, about 61 m. Five exercise stations stand beside it, each a post with a plaque that says what to do there, standing, with a headset on. It is all in `trail.rs`, laid out from `trail::WAYPOINTS`.

## Where it goes

`trail::centreline()` is the loop, smoothed through `WAYPOINTS`, `STEPS` points to each, the first at the gate; `trail::frame(index)` gives a point of it, the way along it and the way out of the loop. Its track is `WIDTH`, 1.2 m, from crest to crest, all of it walkable. It keeps:

- **within `REACH`**, 11.5 m of the backdrop's middle, so that the whole backdrop stands within the headset's 50 m far plane, 2 m short of it, from anywhere on it, as from the yard; which is why the foothills' last ring is at 36 m. That is also inside the foothills' first ring, so it lies on the level meadow.
- **clear of the river's banks and the pool's rim** (`backdrop::BANKS`, `RIM`), which stand above the water, so that it never climbs them: the river within 7 m of its middle for 12 m and more, the waterfall and pool seen from the lookout.
- **clear of the fence**, its inner edge at least 0.9 m from it, of the pines and of the bush, and out of the views the backdrop keeps open.

## Walking it

It is declared as walkable surfaces, each at the height of what is drawn there, so that participants stand, and the ring where a teleport lands is drawn, on it rather than beneath it: the loop's band in `STRETCHES` polygons, each overlapping the next, since one outline cannot have a hole, on its track at `HEIGHT`; the way out, `GATEWAY`, two rectangles, from the yard's earth between the gate's posts to the fence at `EARTH`, and on to the loop at `HEIGHT`; and the clearings, `clearings()`, discs, one at each station and the bay at the lookout, at `CLEARING_HEIGHT`. `outlines()` gives each outline with its height. All of them are within the centimetre a backstep goes up or down of the yard's earth, so that nothing stops at the gate; keep them so, and keep each at what is drawn, if the track is raised or lowered. The client crate's `locomotion()` declares them with the yard's rectangles. A pair of teleport pads, `pads()`, joins the yard, beside the way to the gate, and the lookout, in its bay, off the trail so that nobody walking or teleporting along it lands on one. The hens keep to `flock::AREA`, well inside the fence and short of the gate, startled or not.

## The stations and places

`STATIONS` are, in order from the gate: 10 squats; 10 shoulder rolls and 10 neck tilts, 5 each side; 20 calf raises; 10 lunges each side; and 30 arm circles and 3 deep breaths, 15 circles each way. Each is a count, never a time, which nobody can keep easily while doing it. None is on the floor or leans on anything, since the posts and rails are not there to lean on. Each has a clearing joined to the trail, on the side with room within `REACH`, its post at the clearing's edge facing it, and its plaque, `plaques()`, which the client crate draws in text from the renderer's glyph atlas.

`places()` are the named anchors Tomori's own server declares (`tomori_server_app::farmyard_trail`), each with room, `ROOM`, for one to eighteen workers: `riverbank` and `pool-lookout`, kind `viewpoint`, and `station-1` to `station-5`, kind `station`, labelled with their exercises, so that the agent can suggest a walk, go to a station and say what to do. A station's anchor stands beside its clearing, opposite its post, a little above head height, facing the clearing, and tells the agent of whoever comes within `STATION_RADIUS`, 1.8 m across the ground (`notify_within`), its clearing and the trail beside it, but never where participants arrive, the gate or a pad, so that arriving, leaving the yard or stepping off a pad tells it nothing. `solids()`, the posts and the seat, and `beside()`, the backdrop's plants and pines within `BESIDE`, 5 m, of the trail, join the yard's, so that the agent, walls and panels keep clear of them: the agent comes 2 m from whoever it follows, which on a 1.2 m trail is often off it, and swings round them rather than standing in a tree fern or a pine.

## Drawing it

`trail::model()` is one model: the track, `LIFT` above the yard's earth, 8 mm, and so 18 mm above the meadow, with edges `EDGE` higher, open where the way out or a clearing meets it; the clearings, 6 mm below the track; a few stones along its edges; the posts and their boards; and the seat. It is counted in the backdrop's budget (see `backdrop.md`).

## Changing it

Move it through `WAYPOINTS`, and the stations by the point of the loop each stands at. Run the farmyard's tests: they check that it stays within reach, outside the fence and clear of the banks, the bush and the views, that it is walkable end to end, through the gate, without a gap or a step that stops a backstep, that landing anywhere on it lands within a millimetre of the top of what is drawn there, that the stations are spaced, beside it and labelled, their radii clear of the arrival, the gate and the pads, that the pads are off the way, and that the hens stay in. Set `TOMORI_FARMYARD_TRAIL_PREVIEW` to a PNG path when running the client crate's tests to save the trail by the river, the gate, the lookout and a plaque, as `-1.png` to `-4.png`. A change to the trail's anchors, or to what an earlier client would not draw, raises the farmyard's version.
