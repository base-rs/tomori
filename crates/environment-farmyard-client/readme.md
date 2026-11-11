# `tomori-environment-farmyard-client`

The client half of [Tomori](https://github.com/ioquatix/tomori)'s farmyard: the scenery Tomori's clients draw, an evening sky behind the yard and its red chicken coop, built in code and drawn with `tomori-assets`, with its chickens, and the New Zealand backdrop beyond the fence, its foothills and bush, its moving river and waterfall, and its snowy ranges; the client side that shows the chickens as the farmyard's server side shares them, lets participants point at them and plays their clucks; its ambience, wind, bush, the river, the waterfall and native birds, from sounds dropped into `assets/sounds`; and the teleport profile that lets participants walk about the yard. What its server and clients share is [`tomori-environment-farmyard`](https://crates.io/crates/tomori-environment-farmyard).

## Usage

Tomori's own app and spectator register it; a build of another project's registers it the same way, at the identity in `tomori-environment-farmyard`, which its server registers too:

```rust
let headset = tomori::Builder::default().register(tomori_environment_farmyard_client::register);
```

A headset build carries the chicken in its APK as `farmyard/`, from `tomori-environment-farmyard`'s `assets/models`, and the sounds as `farmyard/sounds/`, from this crate's `assets/sounds`, as [`context/usage.md`](context/usage.md) says. The chicken is by Hinoki Maple Williams, under the [Creative Commons Attribution 4.0 International licence](https://creativecommons.org/licenses/by/4.0/) (CC BY 4.0); `register` adds its notice to the build's licences, which `tomori --licenses` shows.

The interface is 0.x. Its requirements are in Tomori's [`design/farmyard.md`](https://github.com/ioquatix/tomori/blob/main/design/farmyard.md).

## See Also

- [Tomori](https://github.com/ioquatix/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-render`](https://crates.io/crates/tomori-render), whose `Scenery` it implements.
