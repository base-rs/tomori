# The Farmyard's Backdrop

Beyond the fence lies a New Zealand evening: pasture rising to bush-clad foothills that ring the yard, a waterfall falling down a cliff in the foothills behind the coop into a pool, the river it feeds winding round the yard's right side through grey shingle between its banks, river stones along it, native bush, and, beyond the foothills, ranges fading bluer into the dusk, the highest under snow. It is all scenery, but for the trail round the outside of the fence (see `trail.md`), and the chickens stay in the yard.

## How it is drawn

The headset's far plane is 50 m from the eyes, so the backdrop is in two parts:

- **Within reach** (`backdrop.rs`), in the yard's coordinates, low-poly in the yard's palette and lit as the yard is, hazed with distance: the foothills' ground, one model of rings about `layout::BACKDROP_CENTRE` from 11.5 m to 36 m (`backdrop::RINGS`), whose height is `backdrop::ground`, and which `backdrop::drawn_ground` gives as its triangles draw it; and the bush, a model for each `backdrop::Kind` (the bush's canopy, podocarps, tree ferns, cabbage trees and flax), instanced where `backdrop::plants` says; and the river's stones, one model, `backdrop::stone`, instanced where `backdrop::stones` says.
- **Beyond the far plane**, the ranges (`backdrop::ranges`) are directions, not places: the scenery draws them at the sky's depth, after everything else, where nothing nearer has been drawn, the farthest first, each faded into the sky's own colour that way (`horizon` in the client's `sky.wgsl`), so they stay put as you move, as distant mountains do.

The water (`backdrop::water`) is drawn by the client's `water.wgsl`, which moves it by the time alone: ripples carried downstream, streaks down the fall, and foam and mist at its foot. No texture is needed.

## Its water and banks

The river's and the pool's surfaces are flat at their levels and reach beneath their banks (`backdrop::SPREAD`), so that the shore is where the ground rises out of them. `backdrop::ground` carves the river's bed and the pool's basin below their water, and raises their banks, `backdrop::BANKS` from the river's middle and `RIM` from the pool's, to `LIP` above the water beyond where it is drawn, easing back into the land, but where the river leaves the pool over its lip (`layout::RIVER[1]`, just beyond the pool and just below it). The pool lies at 0.6 m, below the pasture between it and the yard. Since the foothills' ground is triangles 1.5 m across, the tests check the ground as drawn, not only `ground`: everywhere the water's surface ends, the drawn ground stands above it. Flax stands beyond the water's spread, its leaves over the banks, and `backdrop::stones` sets stones of every size along the river's banks where they rise out of the water, in its shallows, standing out of it, and about the pool's shore, on the ground as drawn.

Plants are placed deterministically, from `props::scatter`, so every client draws the same bush. `backdrop::clear` says where one may stand: on the foothills' ground, 3 m and more beyond the fence, out of the way in through the gate, out of the water, off the waterfall's cliff, within the far plane, and clear of the sightlines `backdrop::views` keeps open, from the arrival to the waterfall, the pool and the river, and from the workbench's curve to the river. The pines in `props.rs` keep to the same rule (`backdrop::open`).

## Where its sounds come from

`layout::sounds()` names its places for ambient sound: `waterfall`, the middle of the fall (`layout::WATERFALL`, between `WATERFALL_LIP` and `WATERFALL_FOOT`); `pool`, its middle at its water's level (`layout::POOL`); `river`, each point of its middle downstream of the pool (`layout::RIVER`, at the water's level, running downhill); and `bush`, the middle of each patch in `layout::BUSH`, on the ground. `layout::nearest_river` gives the nearest point of the river to any place.

## The Quest budget

About 21,400 triangles in ten draws a frame for each eye, all opaque, so nothing is sorted: the foothills (2,880), the plants (15,222, one draw for each kind with all its instances), the river's stones (810, 54 of one model in one draw), the trail (1,144, see `trail.md`), the water (197) and the ranges (1,184), under `backdrop::BUDGET`, 22,000, which the tests check with the exact counts; and the stations' five plaques, small panels of text drawn after the scene. The budget was 20,000 until the trail and the stones added about 2,000 triangles in two draws. The water's and the ranges' shaders are a few arithmetic operations a pixel, over a small part of the view, and nothing is built or uploaded after the scenery is made but the time. On a desktop GPU, at the Quest 3's eye size with 4× multisampling, the backdrop took the scenery from about 0.17 to 0.24 ms an eye.

## Changing it

Move the river, the waterfall or the bush through `layout.rs`; the ground follows them. Run the farmyard's tests: they check that the backdrop stands beyond the fence and within the far plane from the yard and the trail, that the views stay open, that the river runs downhill in its valley between banks above its water and the waterfall from its cliff into its pool, that the flax stands on the banks and the stones by the water, that the named places are on the water or among the bush, that the bush is the same every time, and the triangle counts, which change as plants are added or reshaped, and should be updated here too. Set `TOMORI_FARMYARD_BACKDROP_PREVIEW` to a PNG path when running the client crate's tests to save the views from the arrival and from the workbench, as `-1.png` and `-2.png`.
