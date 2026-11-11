# The Farmyard's Campsite

Beside the lookout's log seat, a step west of its end, between the trail and the tree ferns on the pool's bank, a campsite stands on a clearing of packed earth: a ring of stones round a fire pit, a bonfire of leaning logs, a second log seat across the fire, and four marshmallow sticks leaning over that seat towards the fire. It is all in `campsite.rs`, laid out from `campsite::FIRE`, and drawn by the client crate.

## Where it is

`FIRE` is the pit's middle; `seat()` the second seat, as `trail::seat()` gives the lookout's; `standing()` four discs at the trail's height on the fire's trail and lookout sides, which the client crate declares walkable, joined to the trail; `solids()` the pit with its flames and the second seat with its sticks; and `place()` the `campsite` anchor, kind `area`, high above the fire, facing the trail, with `ROOM` for one to eighteen workers, which `tomori_server_app::farmyard_campsite` declares. Where participants stand is within `REACH`, 12.6 m, of the backdrop's middle, so that the backdrop stays within the far plane a metre short of it. `clearing()` is the edge of its earth, drawn just above the ground where it rises towards the pool.

## The sticks

`Pose` is where a stick is: its butt and the way along it to its tip. `Pose::racked(index)` leans it on its log, touching it; `Pose::held(hand, turn)` runs it forward through the fist, `HOLD` from its butt; `Pose::lying` lays it level. `heat(point)` is how hot it is for a marshmallow, and `TOAST_RATE` how fast it toasts at full heat, through `GOLDEN`, `BROWN` and `BLACK`, drawn in `SHADES`, `shade` and `colour`; black in the heat, it is `burning`.

What every client is told: each `Stick`, its `epoch`, `toast`, who `held` it and where it is `lying`, in the farmyard's state beside the flock's (`Shared`, read from the same JSON) and as each changes (`Event`). What a participant asks, `Intent`: `Grab` at the epoch they saw, `Toast` as they hold it, and `Release` with where they let it go. `Campsite` is the server side's: `ask` takes the first grab of a free stick, one a participant, and only its holder's toasting, bounded by the fire's rate, and letting go; `rest` and `beneath` decide where a stick let go lies, straight down on the clearing, the trail, the meadow or the lookout's seat, or back on its log, fresh, by its log, in the fire, beyond `DROP_REACH` or inside the fence; and `sweep`, given who is present, puts back what a leaver held and tidies every stick once the space has been empty for `TIDY`, an hour.

Whoever holds a stick controls it, as the physics proposal's first stage has it: the holder's client draws it from their hand, toasts it and says so; the others draw it from the holder's hand in presence; the server side owns it at rest. The client crate's `campsite.rs` is that client.

## Drawing it

`model()` is the clearing, stones, ash, logs and second seat, lit, and the embers, glowing; `stick()` a stick with a marshmallow in each shade, of which an instance shows one. The client crate draws the flames, embers and burning marshmallows' flames with its `fire.wgsl`, and lights the campsite with a flickering lamp. `BUDGET` bounds their triangles together, which the tests count.

## Changing it

Move it through `FIRE`; the seat, sticks, standing and anchor follow. Run the farmyard's tests: they check that it stays beside the lookout's seat, off the trail, clear of the pool, the river, the bush, the pines, the yard and the views, within its reach, its clearing above the ground, the sticks touching their log and out of the heat, where let-go sticks rest, the grabs, toasting and tidying, and the budget; and the server program's, that one to eighteen workers fit its anchor clear of its solids. Set `TOMORI_FARMYARD_CAMPSITE_PREVIEW` to a PNG path when running the client crate's tests to save the campsite from the lookout's seat. A change to its anchor, or to what an earlier client would not draw, raises the farmyard's version.
