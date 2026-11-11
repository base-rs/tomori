# Replacing the Farmyard's Coop

The farmyard's coop is built in code (`coop::model`), until a coop modelled in Blender replaces it. A replacement is a binary glTF file, `.glb`, loaded at runtime through `tomori-assets` and checked as every asset is.

## Making one

Model the coop in metres, with the ground beneath the middle of its footprint at the origin and its door facing +Z (Blender's −Y, which the glTF exporter turns into +Z). The farmyard places it on `layout::COOP`, so it should fit that footprint, 2.0 m wide and 1.6 m deep, have its floor and door sill at `layout::COOP_FLOOR` (0.6 m), its door where `layout::DOOR` is and its ramp reaching the ground at `layout::RAMP_FOOT`, so that the chickens' paths into it and out of it still fit; and keep its window, if it has one, towards +X, where participants peek in from.

The chickens stand on what the layout says is there, so a replacement keeps it where they stand: the ramp's top runs straight from the sill to the ground at its foot (`layout::ramp_height`), its cleats no more than about 1.5 cm proud of it; the two nest boxes are open at the top and the front, their insides `layout::NESTS`, their bottoms at `NEST_FLOOR` and their low front lips at `NEST_LIP`, room for a hen to walk in, stop and turn round, which the flock's tests check against the hen's own mesh; and the lantern hangs at `layout::LANTERN`, where the scenery's lamp is, clear of the roof. The built coop's perch runs from the far side's lining to the near side's wall, towards the door from the window, above the door's head, on a cleat at each end. Export it as a `.glb`, with Blender in background mode and `--disable-autoexec`, with its textures embedded, as PNG or JPEG, at most 2048 texels on a side.

## Using it

Put the `.glb` in the crate's `assets/models/` folder, which the crate embeds and the APK carries, name it with an `AssetRef` beside the code that uses it, with the SHA-256 of its bytes, add the embedded bytes to `Library::shared()`, and register the farmyard with `register_with(sceneries, Coop::Asset(reference))`, or make it Tomori's own by changing `register`. If the asset cannot be loaded, the built coop stands in and the log says why. A coop that changes the coop's footprint, door or ramp changes the layout, which the farmyard's clients and server share, so it raises the farmyard's version.
