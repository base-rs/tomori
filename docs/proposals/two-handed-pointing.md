# Proposal: Two-Handed Pointing and Grabbing

Status: **proposed**, November 2026, for the maintainer's review. Nothing here is a requirement until each stage brings it into the design documents it changes (listed at the end), which take precedence until then.

It builds on the ray and its unified end (`design/interaction.md`, #233), the per-control orders and the responder chain's increments (`design/responders.md`), carrying panels and walls with the grip and editing walls by their handles (`design/projection.md`), the grip's declared order and the campsite's marshmallow sticks (`design/environments.md`, #346), travel and its teleport aim (`design/locomotion.md`), the hand menu and the right wrist's transcript (`design/hand-menu.md`, `design/wrist-transcript.md`), feedback by hand (`design/feedback.md`), presence and the agent's `presence` tool (`design/mcp.md`), and the physics proposal (#339), whose held stones are a hand each.

## The request

Spoken by the maintainer from the headset, November 2026:

> I wonder if we should do ray casting from both hands equally. The right hand controller has a ray that shoots out of it; maybe the left should do the same. It might suit people who are ambidextrous better if we can have the same, at least for the first two finger buttons, behave the same way in terms of grabbing and moving things, including panels.

The moment suits it. `live-on-stick-click` moves the live microphone from the left grip to the left stick click, and the left trigger already does nothing in a space, so both triggers and both grips are free to mean the same thing. The physics proposal (#339) and the campsite's marshmallow sticks (#346) need grabbing with either hand, and #339 already allows two held stones for each participant, one in each hand.

## Recommendation in brief

- **Each hand has its own ray, trigger and grip, and they behave the same.** The left ray points at whatever the right ray can point at. The left trigger chooses what the right trigger chooses, and the left grip takes and carries what the right grip takes. That covers Tomori's own things (panels, walls and their handles, cards, workers, timers, list rows, hand menu rows, arrival objects, pads and targets) and the environment's (selectable objects, objects that take the grip). Each hand finds its own pointed responder each frame, and each hand's gestures are latched separately.
- **The asymmetric buttons keep their roles.** X talks, Y pins the menu, A confirms, B backs out and, held, leaves. The left stick travels, the right stick scrolls and moves what is carried, and the stick clicks keep their meanings. Where one of these acts on something pointed at, it asks **its own hand's ray first, then the other's**. So A ticks the item the right ray is on, or else the one the left ray is on. X captures what both rays point at.
- **A ray is drawn only while it has something to say.** That means while it meets something, while its trigger or grip is in use, or for a second after either. Otherwise it is a short stub, and hidden altogether while its hand hangs down or is turned palm up. Two idle 1 m beams are clutter.
- **Shared attention lists both hands.** As X is pressed, the utterance's `attention` names what each shown ray is on, the most recently moved onto first, within the existing limit of four. The agent's `presence` tool reports pointing for each hand. This needs one optional field in presence: which rays are shown.
- **The teleport aims along the left ray**, the hand whose stick aims it. The right ray stays free to point and carry while aiming.
- **Both hands on one thing is a two-handed carry**, for panels and walls. The thing follows the midpoint of the two grips and turns with the line between the hands. For a wall, the spread between the hands sizes its slots, as the corner handle does. Letting go of either hand leaves it carried by the other, without a jump.
- **A per-device *dominant hand* setting mirrors the asymmetric roles wholesale.** The buttons, sticks and wrists all swap together, so that a left-handed participant has the same layout reflected. Rays, triggers and grips need no setting, since they are already the same on both hands.
- **Staged:** (1) the left ray, trigger and grip mirror the right for Tomori's own things, with per-hand attention and the left-ray teleport aim; (2) the environment's objects with either hand, alongside the physics stages; (3) two-handed carry and resize; (4) the dominant-hand setting.

## What exists today

| Part | Today | Where |
| --- | --- | --- |
| The ray | The right controller's aim ray, ending on the nearest thing it meets, otherwise plain and 1 m long. While the hand menu shows, the ray is the menu's; while a teleport is aimed, it ends at the landing | `interaction.md`, **The ray ends on what it meets** |
| What the ray is on | One `precedence::Aim` a frame, from one ray: Tomori's, a pad, the environment's or nothing | `responders.md`, `scene/precedence.rs` |
| Right trigger | `precedence::Trigger`: menu, transcript, wall carried, an environment's list row, Tomori's, pad, the environment's select. In a space it never summons | `interaction.md` |
| Right grip | `precedence::Grab`: transcript, carried, Tomori's (panel, wall, card), the environment's object within reach or pointed at, nothing | `interaction.md`, `environments.md` |
| Left trigger | Nothing in a space | `interaction.md` |
| Left grip | The live microphone; moving to the left stick click in `live-on-stick-click` | `live-audio.md` |
| A, B, right stick | Act on what the right ray is on, after their modal rungs | `interaction.md` |
| Teleport | The left stick forward aims along the **right** ray | `locomotion.md` |
| Hand menu | Left palm up (or Y pins it); pointed at by the right ray | `hand-menu.md` |
| Transcript | Right palm up; worked by the right stick, A, B and grip, with the right ray hidden | `wrist-transcript.md` |
| Presence | Head, both controllers' poses and both tracked hands, every frame; `carrying` names one panel | `tomori-protocol`'s `ClientMessage::Presence` |
| Agent's `presence` | What each head looks at and what the **right** aim ray points at, resolved by the server | `mcp.md` |
| Speech context | `UtteranceBegin::attention`, up to four resources, most relevant first; the client sends what the right ray is on | `interaction.md`, `spaces.md` |
| Environment input | One `Input::pointed`, one select stream for the trigger and A, one `Input::grab`, and the others' **right** hands | `environments.md` |
| Feedback | Pointing, choosing, requests and leaving felt on the right; the voice and knocks on the left | `feedback.md` |

Almost all of this assumes one ray. The data for a second ray is already there: presence carries the left pose to the server and to every client.

## 1. The model

**A hand is a pointer.** The frame step keeps, for each hand, what the single pointer has today:

- its ray, from the controller's aim pose, or later a tracked hand's;
- what the ray meets, ending on the nearest thing, as `pointing::end` decides now;
- its `Aim`, as the frame before found it;
- its trigger pull, its grip squeeze and what each was latched to as it began;
- what it carries.

Each hand's ray is resolved against the same things, and each hand's trigger and grip run the same ladder against that hand's own `Aim`. A pull of the left trigger on a pad goes to the pad's partner. A squeeze of the left grip on a panel pulls it off and carries it along the left ray. A squeeze of the left grip on a wall's corner handle drags it with the left hand's movement. The knob and hinge read the left hand's movement left, right, up and down, as they read the right's.

**What does not become per hand.** These are buttons on one controller, so they keep their roles:

| Control | Role, unchanged | Which ray, where it acts on something pointed at |
| --- | --- | --- |
| A | Confirm, choose, tick, select | its own (the right), then the left |
| B | Back out; a press throws away what is carried; hold to leave | its own (the right), then the left. Throwing away acts on the right hand's carry, then the left's, and the carry hint shows `B  throw away` only beside the one it would throw |
| Right stick | Scroll, and push, pull and turn what is carried | its own, then the left; what is carried in the right hand, then the left's |
| Right stick click | Square what is carried, or replay | as the right stick |
| X | Talk; captures attention as it is pressed | both rays (below) |
| Y | Pin the hand menu | none |
| Left stick | Travel; the teleport aims along the left ray (below) | the left, for the aim and for the environment's left stick |
| Left stick click | The live microphone, after `live-on-stick-click` | none |

"Its own, then the other's" keeps a button predictable: the hand pressing it is the hand it serves, unless that hand points at nothing that takes it. The rule needs no memory of which hand moved last, unlike a single "active ray". It still serves the person who points with the left hand and confirms with the right thumb, as a left-handed or ambidextrous participant will.

**Arrival and the lobby.** Arrival's objects are chosen by either trigger. Where the trigger still draws the swarm, outside a space, either trigger draws it towards its own hand, and a pull of both draws it towards the first.

**Hand tracking.** A tracked hand gets a ray where the runtime gives an aim pose, the same for both hands. It chooses nothing until a pinch acts as the trigger (`environments.md`). When that is designed, it is the same pinch on either hand.

**The desktop window** has one mouse, which stays the right hand. It gains no second ray, and two-handed carry is the headset's alone; the window keeps the corner handle and its keys for resizing.

## 2. Rays

**When each ray is shown.** A ray is drawn in full while it meets something it can point at or take hold of, while its trigger or grip is held, while it carries something, while it aims a teleport, and for one second after any of these. Otherwise it is drawn as a 15 cm stub from the controller, so that the participant knows it is there without two beams sweeping the view. It is hidden altogether:

- while its hand points more than 50° below the horizontal, as an arm resting at the side does;
- while its palm is up showing the hand menu or the transcript;
- while the controller is not tracked.

The right ray is subject to the same rule. Its plain 1 m beam becomes the stub, which also tidies the single-handed view. If that proves too quiet on the headset, the right ray could stay at 1 m, but the symmetric rule is recommended.

**Highlight and hover.** Each ray highlights what it meets. Two rays on one thing highlight it once. The hover tick is felt on the hand whose ray moved onto it, not always the right (`feedback.md` changes to say "the hand that pointed").

**Shared attention.** There is no single "the ray" any longer, so both are listed:

- **As X is pressed**, the client names in `UtteranceBegin::attention` what each **shown** ray is on: first the one that moved onto its target most recently, then the other, then what follows today, within `ATTENTION_LIMIT`. Two rays on one thing name it once. The server describes each as it does now. A note of which hand pointed is not needed: "this one and that one" reads in the order given. Older servers already take a list.
- **The agent's `presence` tool** reports `pointing` for each hand, `left` and `right`, each as it reports the right ray today, and only for a ray the client says is shown. A hand that hangs at someone's side should not be reported as pointing at the wall in front of their feet.
- **Presence** gains one optional field, `rays`, two flags for whether each ray is shown. An older client sends none, and the server takes it as the right ray alone, which is today's behaviour. Older servers ignore it. Nothing else in presence changes, since both controllers' poses are already sent.

**The teleport aim and the left ray.** The left stick aims the teleport, so it aims along the left ray. The stick and the hand that holds it work together, as in most headset applications. While the left stick aims:

- the left ray serves only the aim, ending at the landing with its ring;
- the right ray goes on pointing, choosing and carrying, so a panel carried in the right hand stays in the hand through the aim, as it does now.

There is one gain: a card carried in the right hand no longer blocks travel, since the aim no longer takes its ray. There is one change: participants used to aiming with the right controller now aim with the left. The dominant-hand setting swaps both together. Whether the aim becomes an arc rather than a straight ray is a question of its own (see the open questions).

## 3. Conflicts

**Two hands, two things.** Each hand may point at, choose and carry its own thing at once. Carry a panel in the right hand while the left points at a checklist item and A ticks it. Hold a marshmallow stick in the left hand while the right takes hold of a wall. Each gesture belongs to the hand that began it, until that hand lets go, whatever the other hand does.

**Two hands, one thing: a two-handed carry.** A squeeze of the second grip on something the first hand carries joins the carry rather than taking it:

- **Panels and walls.** Each hand keeps the point it took along its own ray, and the thing follows the midpoint of the two points. It turns about the vertical with the line between them. A panel also tilts with that line's rise; a wall stays upright, as carried walls do now. For a **wall**, the distance between the hands, against what it was as the second hand closed, sizes its slots, the same field the corner handle sets, in the same 5% steps and bounds, with the same readout. Scaling a floating **panel** is left open (below), since panels have no shared size of their own. Letting go of either hand leaves the thing carried by the other, re-anchored where it now is, so nothing jumps. The right stick, its click and B act on the two-handed carry as on a one-handed one.
- **Cards**: the second hand takes the card from the first, a hand-off, since a card has nothing to stretch.
- **An environment's object**: the client side is told both grabs, each with its hand, and decides. Telling it nothing new would make the second squeeze nobody's. The farmyard's sticks hand over to the second hand, which is the natural reading of "take it in the other hand".
- **A wall's handle** is held by one hand. A second hand squeezing on another handle of the same wall drags that one too, so two corners can be pulled at once only where the handles do not conflict; otherwise the second squeeze is nobody's until let go.

The alternative, first hand wins, is simpler and is what stage 1 does: until stage 3, a second grip on something carried is nobody's.

**The hand menu.** The menu shows on the left palm, or wherever Y pins it, and is pointed at by **the other hand's** ray. The hand holding it is palm up, so its ray is hidden and its trigger is the menu's, choosing nothing. While the menu shows, the other hand's ray is the menu's and ends on its rows, and its trigger and A choose. With the dominant hand set to left, the menu goes to the right palm and the left ray points at it.

**The transcript.** It stays on the right wrist, worked by the right thumb and the right grip, with the right ray hidden while it shows. The **left** ray, trigger and grip go on working in the world meanwhile. So a participant can read the transcript on one wrist and pull a panel off the wall with the other hand. Only the right hand's ladders have the transcript's rungs. The rule that the transcript does not show while the right trigger is held, or while the right ray is on a menu row, is unchanged. The left trigger and the left ray do not hold it back. With the dominant hand set to left, all of this mirrors.

**Carrying, travel and the hand menu.** A panel carried in either hand travels along with the left stick, as now. If the left hand carries a card, the left stick does not aim, for the same reason the right hand's card blocked travel before: the aim would take its ray.

**One select stream for an environment.** An environment's client side written for one select at a time keeps one. The first of the two triggers and A to begin a select on a selectable object keeps it until it is let go, and the others are nobody's meanwhile, as A and the trigger share one stream today. `Select` gains the hand it came from, `#[non_exhaustive]` already allowing it. A client side that opts in, with a new `ClientSide::two_handed`, is given a select for each hand at once.

## 4. The dominant hand

A per-device setting, **Dominant hand: right** (the default) **or left**, in a new **Controls** section of the hand menu, before Sound, in and out of a space, under "On this headset", and in the console's `controls hand left|right`. Like the Feedback setting, it is kept on the device, not in the space.

It **mirrors every asymmetric role together**:

- X's and A's roles swap, and Y's and B's;
- the left stick scrolls and the right stick travels (and aims along the right ray);
- the hand menu goes to the right palm, and the transcript to the left wrist, worked by the left stick, X (as A) and Y (as B);
- feedback's sides swap, so that the voice is felt on the right;
- every hint names the physical button it means, as `X  tick` rather than `A  tick`.

Swapping only the sticks and wrists was considered. It would leave the transcript on the left wrist worked by the right thumb across the body, or the confirm button on the hand that is not pointing, which is the asymmetry the setting exists to remove. A full mirror has one rule and one test: every role on a hand goes to the other hand. The system's own buttons are the runtime's and cannot move: the Meta button stays on the left and the Oculus button on the right. Neither is Tomori's.

Since rays, triggers and grips are already the same on both hands, an ambidextrous participant needs no setting at all. The setting is for people who want the whole layout reflected.

## 5. The protocol

- **Presence**: an optional `rays: [bool; 2]`, left then right, saying which rays are shown. It is absent from older clients, which the server reads as the right ray only. `carrying` names one panel today. With a panel in each hand, it becomes a list, `carried`, beside it, with `carrying` kept for older servers as the first. An older server then protects only that one panel from the agent moving it, which is acceptable.
- **`UtteranceBegin::attention`** is unchanged in form. It simply lists what both rays are on.
- **The agent's `presence` tool** says `pointing: { left, right }` for each participant, each what that ray meets or absent. The old single `pointing` stays as the right's, for agents' instructions written against it, until a later change drops it.
- **Older clients** send one ray and are described as today. **Older servers** ignore `rays` and `carried`, and describe only the right ray in `presence`.
- **Environments**: `Input::hands` gives the others' **both** hands (which the physics proposal also needs). `Input::grab` becomes one for each hand, with `grab` kept as the right hand's. `Select` gains its hand. `ClientSide::within_reach` is asked for each hand. Each of these is additive behind `#[non_exhaustive]`, and an environment written for the right hand alone keeps working.

## 6. The orders become per hand

`interaction.md`'s rule, "each control has one order of rungs, and the first rung that claims it as it begins takes it", is kept. **A control instance per hand** is introduced: `Trigger(Hand)` and `Grab(Hand)` each run their ladder, `precedence::Trigger` and `precedence::Grab`, against that hand's `Aim` and carry, with their own gesture latch. The rungs are the same for both hands, with these exceptions, all modal:

- `Menu` claims the trigger of the hand **not** holding the menu, choosing the row its ray began on. The holding hand's trigger is absorbed while the menu shows.
- `Transcript` claims only the wrist hand's trigger (absorbing it) and grip (detach and reattach).
- `Carried` claims a grip on what **either** hand carries. For the hand already carrying, it is that carry. For the other hand, in stage 3, it joins the carry as a two-handed one; until then, the squeeze is nobody's.

A, B and the right stick keep one ladder each. Their pointed rungs, such as `precedence::A`'s checklist item and row, `precedence::Right`'s `Scroll` and `Object`, and B's panel pointed at, ask the button's own hand's responder first, then the other's. Their carried rungs ask the right hand's carry, then the left's.

`responders.md`'s "one pointed responder per frame" becomes **one for each hand per frame**. Increment 4's `Pointed` value is then a pair, and increment 2's `Gesture<Owner>` is kept for each control instance. Both increments make this cheaper. They are not prerequisites: stage 1 can hold the per-hand state as `Hands<T>`, a pair indexed by hand, around today's fields (`pointer`, `aim`, `pointed_panel`, `card_reach`, `Client::focus` and the carry state). That may be the moment to do increment 4 rather than doubling those fields. The equivalence test of increment 3 gains the hand as one more dimension, and with the left hand idle, every case must decide as it does today.

## 7. Staged plan

Each stage keeps every test passing, changes nothing for a participant who uses only the right hand except where it says so, and is reviewable alone.

1. **The left hand mirrors the right for Tomori's own things.** This comes after `live-on-stick-click` frees the left grip. It covers:
   - per-hand pointing state, `Aim` and gesture latches;
   - the left ray, drawn by the rule in section 2;
   - the left trigger choosing arrival objects, menu rows (when the menu is pinned or on the other palm), an environment's list rows, pads and targets;
   - the left grip pulling off, carrying and docking panels, taking hold of walls and their handles, and picking up cards;
   - A, B and the right stick asking their own hand's ray first, then the other's;
   - the teleport aiming along the left ray;
   - `attention` listing both rays;
   - `rays` and `carried` in presence;
   - `pointing` for each hand in the agent's `presence`;
   - hover and choose feedback on the hand that acted.

   A second grip on something carried is nobody's. The environment's objects are still the right hand's alone.
2. **The environment's objects with either hand**, alongside the physics proposal's stage 1:
   - selects from either trigger, in one stream, with `Select::hand`;
   - `Input::grab` and `within_reach` for each hand;
   - the others' both hands in `Input::hands`;
   - the farmyard's sticks held in either hand, and handed from one hand to the other.
3. **Two-handed carry and resize**: panels and walls moved and turned between both hands, walls' slots sized by the hands' spread, re-anchoring on letting go of one, cards handed over, and an environment told both grabs.
4. **The dominant-hand setting**: the full mirror, its section in the hand menu, the console's command, and hints that name the physical button.

## 8. Costs and risks

- **Twice the ray casts.** Resolving a second ray against cards, workers, panels, walls, handles, pads and the environment's `pick` doubles that part of the frame. It is small against drawing, and a hidden ray casts nothing.
- **Muscle memory.** Aiming the teleport with the left hand changes something the maintainer uses daily. The setting does not restore the old way; an open question asks whether it should.
- **Two highlights, two hints.** Two rays on two things show two labels. The carry hint and the ray's label must each sit by their own hand's target and never overlap. That needs checking on the headset with a panel carried in each hand.
- **The orders were tuned for one ray.** Making them per hand without changing any single-handed case is what increment 3's equivalence test, extended by the hand, protects.
- **Ambiguity in "this".** With both rays shown, the agent is told two things. A participant who meant one may need to say which, or lower the other hand. The order (most recently pointed first) is the mitigation.

## Open questions for the maintainer

- Should the right ray stay a full 1 m beam when it meets nothing, as now, and only the left ray use the stub? Or should both use the symmetric rule recommended here?
- Should the teleport aim along the left ray, as recommended, or stay on the right? And should the aim become an arc, which reaches far landings with a lower hand?
- Should X's speech context list both rays, as recommended, or only the most recent? Should the agent be told which hand pointed at what?
- Two-handed carry for **floating panels**: should the hands' spread scale them? That would need a shared per-panel size or text scale the projection does not have yet. Or should panels only move and turn with two hands, and walls alone resize?
- Should a second grip on another participant's held object be refused, as #339 recommends for stones, or take it?
- Should the dominant-hand setting mirror everything, as recommended, or only the sticks and the wrists? Should it also keep a way to aim the teleport with the right hand?
- Should a ray be shown to **other** participants, from presence's `rays`? Clients do not draw other participants yet, but the spectator could.
- Should hand tracking's rays come with the pinch, or before it, for pointing at things while speaking?

## Design documents this would change

`interaction.md` (the controls table, the ray, **Whose each control is**, and every "the right controller's ray"), `responders.md` (per-hand pointing and gestures, and the equivalence test), `projection.md` (carrying with either grip, the hint's placement and two-handed carry), `locomotion.md` (the aim along the left ray, and cards and travel), `hand-menu.md` (pointed at by the other hand, and the mirror), `wrist-transcript.md` (the left hand free while it shows, and the mirror), `environments.md` (`Select::hand`, grabs and reach for each hand, both hands in `Input::hands`, `two_handed`), `feedback.md` (felt on the hand that acted, and the mirror), `mcp.md` (`pointing` for each hand), `spaces.md` and `clients.md` (presence's `rays` and `carried`), and `desktop.md` (one mouse, the right hand).
