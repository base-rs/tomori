# Using Tomori's Assets

`tomori-assets` loads an environment's models at runtime, from bytes bundled with the app, and draws them. It has layers, each usable without those after it.

## Assets

- `AssetRef::new(environment, version, name, digest)` names an asset: the environment it belongs to, at its protocol version, its name, which is also its path beneath a folder of assets (`ENVIRONMENT/NAME`), and the SHA-256 `Digest` of its bytes, which is what it is. Write the digest beside the code that expects the asset, so that changing the file without its digest fails to load it.
- A `Source` reads an asset's bytes: `Embedded`, bytes compiled in with `include_bytes!`; `Directory`, a folder holding each asset at its path; or one of the app's own, such as the APK's `assets` folder on Android. A source never needs to check the digest.
- `Library::shared()` is the program's library; `Library::new(limits)` makes another. `add` appends a source; `bytes` returns the first source's bytes that match the digest, or an `Error` of kind `Missing` or `Mismatch`; `model` reads the bytes as glTF, once, and shares the `Arc<Model>` by digest with everything that asks.

## Reading glTF

`read(bytes, &limits)` reads a binary glTF 2.0 file into a `Model`, or returns why not. It reads triangle meshes with positions, normals, texture coordinates (floats or normalized integers), colours and up to eight joint influences; materials' base colour factor and texture, emissive factor and alpha mask; PNG and JPEG images embedded in the file; skins; and animations of nodes' translations, rotations and scales, stepped or linear. It refuses anything else rather than guessing: external or data URIs, sparse accessors, cubic splines, morph targets, required extensions. `Limits` bounds the file, its JSON, and every count and texture size before anything is allocated from it.

## Models, poses and instances

- `Model` is checked data: `nodes` (each with a `name`, `parent`, `children`, `rest` `Transform`, `mesh` and `skin`), `roots`, `order` (parents before children), `meshes` of `Primitive`s, `materials`, decoded `textures`, `skins` and `animations` of `Channel`s. `Model::built(nodes, roots, meshes, materials, &limits)` checks geometry built in code the same way, so a procedural model can stand in for a file, or be replaced by one. `node(name)` and `animation(name)` find them.
- `Pose::rest(&model)` and `Pose::sampled(&model, animation, seconds)` give each node's local transform; `blend` mixes two, translation and scale linearly and rotation along the shorter arc; `set_local` lets a controller move a node beyond the animation; `globals` composes them. `joint_matrices` gives a skin's matrices, and `deformed` skins a primitive's vertices on the CPU exactly as the GPU does, for tests and for measuring motion.
- `Instance::new(model)` places a model with a `transform`, a pose (`set_pose`) and nodes shown or hidden by name (`set_shown`, as for a lantern's glass); `draws` lists what it draws.

## Animation

`animation` is a small animator, the same on a server and its clients. `Graph::builder()` declares states, each a `Clip` (a span of an animation, `looping()` or once, `at_speed`, `with_event(offset, name)`, `with_motion(RootMotion)`), parameters (`bool`, `float`, `trigger`) and transitions (`transition(from, to, conditions, fade)`, `from_any(to, conditions, fade)`), whose `Condition`s are `Is`, `Above`, `Below`, `Triggered` and `Finished`. `Animator::new(graph, start)` starts in the first state; `set(name, Value, at)` changes a parameter at a moment; `advance(to)` moves to a moment, returning the events passed, each once, and how far root motion `moved`; `state`, `position` and `fading` say where it is; and `pose(&model)` samples the pose, blended while it fades. However its time is divided, the same inputs give the same results. `RootMotion::planted(model, animation, (start, end), feet, forward, rate, contact)` measures how far a clip carries what it animates from its planted feet, each a point in a foot's node, those within `contact` of the lowest counting as on the ground. An environment's controller, its own code, sets the parameters from the state its server side shares, and moves its instance as root motion says; see Tomori's `design/animation.md`.

## Drawing (`gpu` feature)

`gpu::Renderer::new(device, queue, format, samples)` makes the pipeline for an eye's pass with a `Depth32Float` depth buffer. `upload(device, queue, &model)` uploads a model once, as a `GpuModel`, with its textures' mipmaps. Each frame, `place(device, queue, &gpu_model, &mut batch, &instances)` writes every instance's placements and joint matrices into a `Batch` (from `batch(device)`), `set_lighting` sets the scenery's `Lighting` (`Lighting::new(sun_direction, sun, sky, ground)`, `with_haze`, `with_lamp`, up to four), `set_reveal` limits drawing to a sphere while a scenery grows, `prepare` sets an eye's camera, and `draw` draws, one draw for every instance of each primitive.
