# The Farmyard's Backdrop

Beyond the fence lies a New Zealand evening: pasture rising to bush-clad foothills that ring the yard, a waterfall falling down a cliff in the foothills behind the coop into a pool, the river it feeds winding round the yard's right side through grey shingle, native bush, and, beyond the foothills, ranges fading bluer into the dusk, the highest under snow. It is all scenery: nobody lands beyond the fence, and the chickens stay in the yard.

## How it is drawn

The headset's far plane is 50 m from the eyes, so the backdrop is in two parts:

- **Within reach** (`backdrop.rs`), in the yard's coordinates, low-poly in the yard's palette and lit as the yard is, hazed with distance: the foothills' ground, one model of rings about `layout::BACKDROP_CENTRE` from 11.5 m to 37.5 m (`backdrop::RINGS`), whose height is `backdrop::ground`; and the bush, a model for each `backdrop::Kind` (the bush's canopy, podocarps, tree ferns, cabbage trees and flax), instanced where `backdrop::plants` says.
- **Beyond the far plane**, the ranges (`backdrop::ranges`) are directions, not places: the scenery draws them at the sky's depth, after everything else, where nothing nearer has been drawn, the farthest first, each faded into the sky's own colour that way (`horizon` in the client's `sky.wgsl`), so they stay put as you move, as distant mountains do.

The water (`backdrop::water`) is drawn by the client's `water.wgsl`, which moves it by the time alone: ripples carried downstream, streaks down the fall, and foam and mist at its foot. No texture is needed.

Plants are placed deterministically, from `props::scatter`, so every client draws the same bush. `backdrop::clear` says where one may stand: on the foothills' ground, 3 m and more beyond the fence, out of the way in through the gate, out of the water, off the waterfall's cliff, within the far plane, and clear of the sightlines `backdrop::views` keeps open, from the arrival to the waterfall, the pool and the river, and from the workbench's curve to the river. The pines in `props.rs` keep to the same rule (`backdrop::open`).

## Where its sounds come from

`layout::sounds()` names its places for ambient sound: `waterfall`, the middle of the fall (`layout::WATERFALL`, between `WATERFALL_LIP` and `WATERFALL_FOOT`); `pool`, its middle at its water's level (`layout::POOL`); `river`, each point of its middle downstream of the pool (`layout::RIVER`, at the water's level, running downhill); and `bush`, the middle of each patch in `layout::BUSH`, on the ground. `layout::nearest_river` gives the nearest point of the river to any place.

## The Quest budget

About 19,500 triangles in eight draws a frame for each eye, all opaque, so nothing is sorted: the foothills (2,880), the plants (15,222, one draw for each kind with all its instances), the water (197) and the ranges (1,184), under `backdrop::BUDGET`, 20,000, which the tests check with the exact counts. The water's and the ranges' shaders are a few arithmetic operations a pixel, over a small part of the view, and nothing is built or uploaded after the scenery is made but the time. On a desktop GPU, at the Quest 3's eye size with 4× multisampling, the backdrop took the scenery from about 0.17 to 0.24 ms an eye.

## Changing it

Move the river, the waterfall or the bush through `layout.rs`; the ground follows them. Run the farmyard's tests: they check that the backdrop stands beyond the fence and within the far plane, that the views stay open, that the river runs downhill in its valley and the waterfall from its cliff into its pool, that the named places are on the water or among the bush, that the bush is the same every time, and the triangle counts, which change as plants are added or reshaped, and should be updated here too. Set `TOMORI_FARMYARD_BACKDROP_PREVIEW` to a PNG path when running the client crate's tests to save the views from the arrival and from the workbench, as `-1.png` and `-2.png`.
