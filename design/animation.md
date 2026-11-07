# Animation

This describes how Tomori animates an environment's models: the split between model, view and controller, and the small animator that is the general part, which an environment's controllers drive.

## Requirement

**Model, view and controller.** An animated object in an environment is split three ways, as a model, a view and a controller in an application are:

- **The model** is the asset and its state: the `.glb` a `Library` loads (see `assets.md`), and the high-level state the environment's server side owns and shares over the state channel (see `environments.md`), such as what a creature is doing and since when. The server side decides it; clients never do.
- **The view** is an `Instance` of the model, posed and drawn: the scenery draws it with `tomori-assets`' GPU renderer, skinned, every instance of a primitive in one draw.
- **The controller** maps the model's state onto the view: it sets an animator's parameters at the moments the state says, places the instance where the animator's root motion takes it, and turns what it hears from the animator, its events, into effects such as sounds. A controller is concrete: it belongs to its environment's crate, and knows the one kind of object it controls.

**General now, specific for now.** Only what is clearly reusable is general and lives in Tomori's crates: loading and validating assets, model instances drawn with GPU skinning, and the animator below. Controllers, the behaviour of what they control, their state's encoding and their animator graphs are each environment's own, in its own crate; no controller framework or trait is built ahead of the shapes the first environments show, and these are generalised as they are learnt. The first environments' controllers and server sides are deliberately specific to them.

**The animator.** `tomori_assets::animation` is pure, deterministic logic, without the GPU, which runs the same on the server and on clients:

- **Clips** are spans of a model's animations, played looping or once, at a speed, sampled at a moment into a pose.
- **States** are named, each a clip; an animator starts in the first.
- **Parameters** are bools, floats and triggers, each set at a moment, which applies when the animator's time reaches it, or at once if it has passed. Changes at one moment apply in the order they were set.
- **Transitions** go from a state, or from any other, to another, when all their conditions hold: a bool is a value, a float is above or below a limit, a trigger is set, which taking the transition clears, or the state's clip, played once, has finished. They are tried in the order declared, and at most 8 are taken at one moment, so that transitions that lead back to each other cannot loop. A transition cross-fades from the pose the animator was in, frozen as it was, over its fade; no transition is taken until a fade has finished, and then the conditions are tried again.
- **Events** are named moments on a clip's timeline, such as a footstep or a call's cue, each within its clip. Each fires once each time the playhead passes it, on every loop, and an event at a clip's start also as its state is entered, with the moment it happened, however the animator's time is divided into frames. An advance reports at most the latest 64 (`EVENT_LIMIT`), so that a client joining late, which advances a long way in one step, costs no more than one advancing a frame, and is told only what it would have heard last.
- **Root motion** is how far a clip carries what it animates, a table of distance along the clip, so that a controller moves its object as the animation does, and its feet do not slide. `RootMotion::planted` measures it from the model's planted feet, each a point on a foot's node, such as its sole: from each sample of the clip to the next, as far as the foot on the ground sweeps back, the feet within a given height of the lowest counting as on the ground and of those the one sweeping back the most, so that a walk (a small height: its planted foot) and a scrambling sprint (any height: its pushing foot) are measured alike. A sample at which no foot is on the ground carries nothing. An animator reports how far its clips moved over each advance, across loops.
- **Determinism.** Where an animator is at a moment, and what it reported on the way, depend only on its graph, when it started, and the parameters set at given moments, never on how its time was divided: it advances in steps that end at every change of parameter, every clip's end and every fade's end, and each loop's moment of an event is computed the same way in every step. A moment that is not finite is never reached. The graph builder refuses what would break this: a clip without end, an event outside its clip and a fade that never ends. So the server and every client that are given the same state compute the same animation without streaming poses, and a client that joins late computes it in one step.
- **Declared in code.** An environment declares an animator's graph with `Graph::builder()`; a file format can come later. Its types are `#[non_exhaustive]`, made with constructors, so that they can grow.

**Not yet.** Blend spaces, layers and masks, additive animation, inverse kinematics and a file format for graphs: none is needed by the first environments.

## Rationale

In November 2026 the maintainer asked that Tomori standardise on a model and animation controller system, because the rendering engine will grow in complexity and an environment's animated creatures should not get bespoke animation code: general layers, assets, model instances and a small animator, which environments' controllers use, with the server side owning behaviour and sending high-level state and start times, and each client's animator mapping that state onto clips deterministically. The maintainer then clarified that the controller layer is MVC for 3D objects: a controller specific to its object is right, and no generic controller framework or trait hierarchy is to be built up front; the animator is to stay minimal, without blend spaces or data formats unless an environment needs them. Measuring how far a walk and a sprint carry a creature from its planted feet was first done in a bake; root motion makes that a property of any clip. Fades freeze the pose they leave rather than play both clips, so that a transition costs one more sampled pose, and holding transitions until a fade ends keeps every blend whole.

## Scope

`crates/assets/src/animation.rs`, and the controllers that use it, in each environment's own crate.

## Validation

- Tests check the graph builder's refusals: no states, a state or parameter named twice, a transition or condition naming what the graph does not have or a parameter of another kind, a clip that does not play or never ends, an event outside its clip and a fade that never ends.
- Tests set parameters ahead of time, at the moment, and in the past, and check transitions on bools, floats, triggers (taken once) and finished clips, from a state and from any, the limit on transitions at one moment, and that nothing is taken during a fade and the conditions are tried again as it ends.
- Tests check a cross-fade's pose: frozen as the last state left it, blended by how much of the fade is left.
- Tests check that events fire exactly once, with their moments, on every loop and at a clip's end, whether the animator advances in one step or in frames that fall on and across them; that root motion accumulates over loops and stops with a clip played once; that random divisions of the same span give the same state, position, fade and motion; that planted feet give the distance a walk's feet sweep back, and nothing without a foot on the ground; and that an advance over a billion seconds returns at once with the latest 64 events, as advancing in frames reports them last, and one to a moment that is not finite changes nothing.
