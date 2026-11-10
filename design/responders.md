# Responders

This describes how Tomori routes its controls to what the participant points at, and proposes generalising it into a **responder chain** in the spirit of AppKit's first responder, so that an environment's 3D objects, such as a globe, take the controls that Tomori's panels take. Its first step is built and required, as below and in `environments.md` and `interaction.md`; the later steps are proposals, not requirements, until the maintainer accepts each.

## Requirement

**Objects that take the right stick.** An environment's client side may mark an object it picks as taking the right stick, with `Picked::handles(Actions::SCROLL)`. A push of the right stick that leaves the middle while the ray points at such an object, as the frame before found it, when no rung of Tomori's own takes it, is that object's until the stick returns to the middle, wherever the ray goes and whatever appears meanwhile, and its client side is told it as `Input::scroll`, naming the object, with the stick's axis and its phase, `Began` once, `Changed` while held and `Ended` once, as it returns to the middle or focus is lost, while `Input::right_stick` is zero. It is the right stick's rung `Object`, after every rung of Tomori's and before the environment's client side. A client side that marks nothing is told exactly what it was. The desktop window's keys for the right stick do the same, and its wheel pushes such an object as far as it would scroll a panel, at most at the stick's rate, keeping at most two notches. `environments.md` (**Scroll**), `interaction.md` (the controls table and **Whose each control is**) and `desktop.md` (**Pushing what an environment shows**) state it in full; the server side is not involved, learning of it only through the client side's intents.

**The ray's hint stays a panel's.** The ray shows `Stick ↕  scroll` on a panel that scrolls, from the panel's label; an environment's object has no label on the ray, only its short name in the recording indicator, so showing a hint for one would mean a new label for every object an environment shows, which is a decision of its own (see Open questions). Until then an object that takes the stick shows no hint.

Under **Proposal**, the interface for environments and the first increment are built; the rest is direction, not requirement.

## Status

Asked for in November 2026 by the maintainer, speaking from the headset: "I wonder if we could copy into Tomori something like Objective-C's first responder model, which would allow a kind of generic event handling mechanism, and we map the control inputs to that handling mechanism, so that instead of two-dimensional UI elements where focus follows what you're pointing at, we could do things like: right now I want to point at the globe and then use the right thumbstick to rotate it. I don't know if we have a mechanism to do that. In Tomori with panels we've done something like that for scrolling, so we do have some features like that, but I wonder if they're generic enough to use in different UI elements or 3D elements."

Before the first step, the answer was **partly**: an environment's client side could turn a globe with the right stick by reading `Input::pointed` itself as a push began, but it was told the stick whenever the ray was on nothing of Tomori's, not only on the globe, was not told which object the push began on, and had to infer when a push began and ended from the axis leaving and returning to zero. The first step, built, closes exactly that gap, additively. Generalising the rest into a responder chain is possible and worth doing in steps, but a pure AppKit chain cannot reproduce today's orders; what fits Tomori is responders filling the rungs of one declared order per action.

The sections up to **Proposal** describe the routing as it was when the proposal was written, before the first step, with one later change noted where it applies: A, which then had no declared order and never reached an environment, has since had its own order, `precedence::A`, and selects an environment's selectable object as the trigger does, where nothing before it in A's order takes the press (#291). Since then, teleport pads (#292) are a rung of the trigger's order, `Pad`, after `Tomori` and before `Environment`, and the last of A's conversation's meanings, and a pad pointed at is `Aim::Pad`, which the sticks treat as Tomori's. The first step adds only the rung `Object` to the right stick's order, `Input::scroll` and the desktop wheel's push.

## What exists today

### One frame step for every interactive client

The headset and the desktop window run the same frame step, `tomori-scene`'s `Step`, with the keyboard and mouse standing in for the controllers (`desktop.md`: "The window runs the frame step it shares with the headset, with the keyboard and mouse standing in for its controls, not a separate interface"). `Step::buttons` (`crates/scene/src/scene.rs`) acts on the buttons and sticks each frame, and `Step::update` (`scene.rs`) on the trigger, the grip, the ray and everything drawn. The spectator takes no input and runs no client side (`environments.md`; `crates/client-spectator` uses no `Step`), so nothing here changes it. Hand tracking alone has no stick, A or B (`wrist-transcript.md`), and a pinch does not act as the trigger yet (`environments.md`).

### What the ray is on: already unified

Each frame, `Step::update` resolves what the ray meets, nearest first, among cards, workers, the environment's picked object and projected panels (`scene.rs`), the ray ending on the nearest of them through `pointing::end` (`scene.rs`; `interaction.md`, "Each kind reports how far along the ray it meets it, and the ray ends on the nearest"). It then classifies the result into one `precedence::Aim` (`crates/scene/src/scene/precedence.rs`; set at `scene.rs`):

- `Aim::Tomori` while the hand menu shows, a wall's board, bar or handles is nearer than the picked object (`on_wall`, `scene.rs`), the right wrist's transcript shows, a teleport is aimed, an arrival object or menu row is highlighted, or a panel, card or worker is pointed at or carried;
- `Aim::Environment` when the client side's `pick` is nearer than all of those;
- `Aim::Nothing` otherwise.

The client side's `focus`, the selectable object the trigger and B act on next frame, is the picked object only if it is `selectable` (`scene.rs`). `Input::pointed` names whatever object was picked, selectable or not (`scene.rs`). Which object was pointed at when something began is decided from the frame before, as `client_side.rs` says of a select.

So "focus follows the pointer" exists: the frame computes one focus per frame, Tomori's own interface first. What is not unified is which of the things pointed at **takes** each control: `interaction.md` says so plainly, "Which of the things it meets takes the grip or the words is decided as before; only where it ends is unified."

### One order per control, implemented per control

`interaction.md` sets the rule: "Each control has one order of rungs, and the first rung that claims it as it begins takes it", with "Tomori's own interface first", and "A new use of a control is a new rung, put where it belongs in its order". The orders live in three places (A's, since #291, in the first):

- `precedence::Right`, `precedence::Left` and `precedence::Trigger`, and since #291 `precedence::A` (`precedence.rs`), each a `Ladder` whose `first` picks the first rung that claims (`precedence.rs`), with tests that each rung wins over the ones after it and that Tomori's come before the environment's (`precedence.rs`);
- B's order, `conversation::Back` (`crates/scene/src/conversation.rs`), declared as an ordered enum, but carried out by an `if` chain split in two: the first five rungs in `Step::buttons` (`scene.rs`) and the rest in `Conversation::back_or_environment` (`conversation.rs`);
- A's order, which, before #291, had no declared type at all: the hand menu's row and "anything else while the hand menu shows" in `Step::buttons` (`scene.rs`), the transcript in `Wrist::input` (`scene.rs`), and the list, the request's grace, the checklist item, "turn the right wrist", the approval and the queue in `Conversation::accept` (`conversation.rs`). Since #291 it is `precedence::A`: MenuRow, Menu, Transcript, Conversation, Tomori, Environment, Nothing, which `Step::buttons` asks in turn, the conversation's rung being the conversation's `accept` still.

### The current routing

What each control does in each situation, as the frame before found it, before #291 and #292; since #292, a teleport pad pointed at takes the trigger and A, to go to the pad paired with it, after everything else of Tomori's and before the environment's, and the sticks as Tomori's. "Swarm" is attracting the swarm; "summons" adds summoning it after 0.5 s held. Since stage 1 of the agent following the speaker (`docs/proposals/agent-follows-the-speaker.md`), the trigger in a space neither attracts the swarm nor summons: those cells read "nothing" in a space, and the swarm rung is outside a space alone. Cells read left to right as the first column that applies wins, except where the cell says otherwise.

| Control | Hand menu shows | Transcript shows | Panel or wall carried | Panel pointed at | Card, worker, wall board | Environment object | Nothing |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Trigger (left button) | chooses the row it began on, or nothing | nothing | wall: keeps it from snapping, no swarm; panel: swarm, no summons | swarm, no summons | swarm, no summons | selectable: `Select` to the client side, no swarm; otherwise swarm, summons | swarm, summons |
| A (Enter) | chooses the row pointed at, else nothing | expand, confirm a choice, remove an entry | the list, then nothing or "turn the right wrist" | ticks the checklist item pointed at, after the list and a request's grace | the list, else nothing | never the environment's; since #291, selectable: `Select` to the client side, after every rung of Tomori's | the list, else nothing |
| B (Backspace) | pinned: lets go; shown by pose: falls through | backs out of the transcript if it has something to | panel: a quick press throws it away; wall: removes it, for an owner; neither starts holding to leave | after the conversation's rungs, says how to throw it away | wall held: lets go; else the conversation's rungs | selectable: `Back` to the client side, after the conversation's rungs | the conversation's rungs, then "hold B to leave" |
| Right stick (PgUp, PgDn, `[`, `]`) | nothing (Tomori's) | moves its focus | pushes, pulls, turns it | scrolls it if it has more, else nothing | nothing (Tomori's) | `right_stick` to the client side | `right_stick` to the client side, or result cards without one |
| Right stick click (R) | replay | replay | turns it square | replay | replay | replay | replay |
| Left stick (arrows) | travel's, inactive | travel's, inactive | travel, the panel along | travel where it travels, else nothing | travel where it travels, else nothing | travel where it travels, else `left_stick` to the client side | travel where it travels, else `left_stick` to the client side |
| Right grip (G) | — | detaches or reattaches it | carrying | pulls it off | takes the card, or takes hold of the wall | never the environment's | nothing |
| Wheel (desktop) | — | nothing | nothing | scrolls it | nothing | nothing | nothing |
| X, Y, left stick click, left grip | talk, pin the menu, mute, live microphone, whatever is pointed at |  |  |  |  |  |  |

Hold B for a second to leave is not a rung: every press but one on a panel or wall carried starts it, whichever rung took the press (`interaction.md`; `Step::buttons`).

Where each is decided, and how its ownership is held through a gesture:

| Control | Order | Where decided | Ownership through the gesture |
| --- | --- | --- | --- |
| Right stick | `precedence::Right`: Carried, Transcript, Scroll, Tomori, Environment, Cards | `scene.rs` | `RightStick`, latched as it leaves the middle (`scene.rs`); `Transcript`, `Tomori` and `Cards` all latch as `Other`, and the transcript and the cards each check `wrist_active` again (`scene.rs`, `817`–`822`) |
| Left stick | `precedence::Left`: Travel, Tomori, Environment, Nothing | `scene.rs` | `LeftStick`, the same latch again (`scene.rs`); travel latches a second time by the direction a push first went (`locomotion.md`) |
| Trigger | `precedence::Trigger`: Menu, Transcript, CarriedWall, Tomori, Environment, Swarm; since #292 Pad between Tomori and Environment | `scene.rs`, every frame | no one latch: `Selector` latches the environment's pull (`client_side.rs`), `Pointer` the arrival or menu row a pull began on, and `summoning` whether the pull is the swarm's own (`scene.rs`) |
| B | `conversation::Back`, 15 rungs | `scene.rs` and `conversation.rs` | a press, except a panel or wall carried, latched in `throwing` and `removing` until B is let go (`scene.rs`) |
| A | none declared; since #291 `precedence::A`: MenuRow, Menu, Transcript, Conversation, Tomori, Environment, Nothing, a teleport pad pointed at being, since #292, the last of the conversation's meanings | `Step::buttons`, `Wrist::input`, `Conversation::accept` | a press; the request's grace refuses A for 0.6 s (`interaction.md`); since #291, a press the environment takes is a select until A is let go, one select at a time with the trigger (`Selector`) |
| Right grip | none declared | each consumer filters on the others: the projector's grip unless a card is held, the board's unless a panel is carried, the transcript's while it shows (`Step::update`) | each consumer's own carry state |
| Wheel | inline | `scene.rs` | none: each notch goes to the panel pointed at |
| Right stick click | inline | `scene.rs` | a press |

### Generic and ad hoc

Generic already:

- **Where the ray ends** is one rule for every kind (`pointing::end`), and **what it is on** is one classification per frame (`Aim`), Tomori's own first.
- **The orders** are declared lists for three controls, with one `Ladder` trait and tests over every ladder alike.
- **Gesture ownership** is one rule in prose for every control: "a push of a stick as it leaves the middle, until it returns there; a pull of the trigger as it begins, until it is let go; a press of B as it is pressed" (`interaction.md`).

Ad hoc:

- The two stick latches are the same code twice (`RightStick::update`, `LeftStick::update`), and the trigger's ownership is spread over three state machines. B's latches for a carried panel and wall are two more.
- B's order is declared but run as an `if` chain in two files; A had no declared order until #291; the grip and the wheel have none.
- Each rung's claim is a hand-written closure over the frame step's fields (`carrying`, `wrist_active`, `scrolls`, `aim`, `environment`, `focus`), so a new kind of thing to point at is a new field threaded through `Step`, as `pointed_panel`, `card_reach` and `Client::focus` each were.
- An environment's client side gets each control as **one** recipient: it is told `select` and `back` for its object, but the sticks without saying which object, if any, they were meant for (`client_side.rs`).

## Can an environment turn its globe today?

### The exact rule

The right stick reaches a client side under `precedence::Right::Environment`, which claims whenever the space has a client side (`scene.rs`: `precedence::Right::Environment => environment`, with `environment = self.client.is_some()`, `scene.rs`). It is reached only after `Carried`, `Transcript`, `Scroll` and `Tomori` decline, and `Tomori` claims whenever `Aim::Tomori`. So, as `environments.md` says, a push is the client side's when, as it leaves the middle, no panel is carried, the transcript does not show, and the ray is on something of the environment's **or on nothing of Tomori's**; selectable or not (the test `the_right_stick_reaches_the_environment_only_where_nothing_else_takes_it`, `scene.rs`, checks both). It stays the client side's until the stick returns within 0.2 of the middle, wherever the ray goes (`scene.rs`).

So the stick is **not** delivered only when the ray is on something of the environment's: it is delivered whenever nothing of Tomori's wants it, including pointing at empty sky. And the client side is told the axis, not to whom it is addressed.

### What a client side can do now

It can reconstruct the missing parts itself, because `Input::pointed` names the object picked this frame, selectable or not (`scene.rs`), and `right_stick` is zero whenever the push is not its own or the stick is in the middle:

```rust
// Today, inside the environment's ClientSide::update.
let pushing = input.right_stick != Vec2::ZERO;
match (&self.turning, pushing) {
    (None, true) if input.pointed == Some("globe") => self.turning = Some(self.yaw),
    (None, true) => {}                                  // a push begun elsewhere: ignore it all the way
    (Some(_), true) => self.yaw += input.right_stick.x * RATE * dt,
    (Some(_), false) => { output = output.with_intent(turn_globe(self.yaw)); self.turning = None; }
    (None, false) => {}
}
```

This works, with caveats a downstream must know:

- **It must latch "began elsewhere" itself.** A push begun on nothing and drifting onto the globe is delivered too; ignoring it needs its own state.
- **It sees this frame's pick, while ownership was decided from the frame before's `Aim`.** One frame apart; harmless in practice.
- **Zero means three things**: at the middle, not its own, and focus lost (`Step::buttons` and `Step::blur`). A push that a panel carried took is zero throughout, so it is not misread, but the client side cannot tell an end from a loss of focus.
- **Nothing shows that the globe takes the stick.** A panel that scrolls says `Stick ↕  scroll` on the ray's label (`projection.md`); an environment object has only its `label`, which the recording indicator shows.
- **The left stick is travel's** wherever the environment travels (`precedence::Left::Travel`), so turning on two axes with both sticks is not available there; the right stick's two axes are.
- **A never reached an environment** (`interaction.md`), so "confirm" on the globe was the trigger's `Select` or nothing; since #291, A selects a selectable object as the trigger does, but an object that only takes the stick, not selectable, still gets no A.

### What Observe would need

Observe, a private downstream environment whose globe of regions and pods (`environments.md`) has its rotation as shared state decided by its server side, would today:

1. Pick the globe in `ClientSide::pick` (it may also mark it `selectable` to choose regions with the trigger; turning does not need it).
2. Latch pushes as above, turning a **local offset** for smooth feedback on the participant's own device.
3. Send its server side an intent such as `{"turn_globe": {"yaw": …}}`, not every frame: intents are dropped beyond 8 at once and 4 a second (`environments.md`), so a client side sends the final yaw as the push ends, and, if others should see the turn live, at most two or three a second meanwhile.
4. Keep the offset until the server side's next state reflects the turn, then drop it, so that the globe does not jump back.

Nothing was missing that blocked it. What was missing was that the frame step says which object a push is for, only for objects that ask, with a clear beginning and end: the first increment, now built. With it, Observe marks the globe `handles(Actions::SCROLL)` in `pick`, reads `Input::scroll` rather than latching pushes itself, and keeps steps 3 and 4 as they are (see the worked example below).

## Is it generic enough?

### What the uses share

| Use | Who decides | Focus | Gesture | Fallback |
| --- | --- | --- | --- | --- |
| Scroll a panel (right stick, wheel) | the panel pointed at | the pointer, nearest | push, latched | nothing (Tomori's), never the environment |
| Select an environment object (trigger) | the object pointed at, if selectable | the pointer, nearest | pull, latched | the swarm, never summoning from Tomori's |
| Back out of an environment (B) | the environment, if its selectable object is pointed at | the pointer | press | "hold B to leave" |
| Tick a checklist item (A) | the panel's item pointed at | the pointer | press | nothing |
| Carry a panel or wall (grip, right stick, click, B) | what is carried | modal: held, not pointed | grip held | travel for the left stick |
| Move the transcript's focus (right stick, A, B) | the transcript | modal: shown on the wrist, not pointed | push or press | — |
| Choose a menu row (trigger, A) | the hand menu | modal while shown, then the row pointed at | pull or press | nothing while it shows |
| Travel (left stick) | travel, **before** anything pointed at | none, or the ray for teleport | push, latched by direction | the client side, where there is no travel |
| Summon (trigger) | the swarm | nothing pointed at | pull held 0.5 s | — |

Three ideas recur in all of them: **focus** (a modal thing if one is up, otherwise the nearest thing pointed at), **a declared order** of who is asked, and **ownership latched as a gesture begins**. Those are what a responder chain captures.

### Where they genuinely differ

- **Most of B's and A's rungs are not pointed at.** The list of modes, a selected queue entry, waiting for a name, the card saying no speech was heard, the two problems, a request's grace and approvals are the conversation's state, and they come **before** the panel or environment object pointed at. In chain terms they are modal responders, a stack above focus, not containers of what is pointed at.
- **Travel comes before focus.** The left stick is travel's in an environment that travels, even pointing at a panel, and a panel carried passes it on to travel. That is AppKit's key equivalent, consulted before the first responder, not a link in the chain after it.
- **The same modal things are in different orders for different controls.** For the trigger, the hand menu comes before a wall carried (`precedence::Trigger`: Menu, Transcript, CarriedWall); for B, a panel or wall carried comes before the pinned hand menu (`conversation::Back`: Carried, CarriedWall, HeldWall, Pinned). With the left palm up while the right grip carries a wall, both are up at once, and one stack cannot give both orders. A single AppKit chain, the same for every action, would change behaviour here.
- **Falling through from Tomori's is not falling through to what is behind.** A trigger pulled on a panel attracts the swarm, never selects the environment's object behind it, and never summons (`interaction.md`). In a chain this is natural, since the environment object is not the panel's container, but the swarm must still be told the pull began on something of Tomori's.
- **Carrying is modal and spans controls.** A panel carried takes the right stick, its click and B, and leaves the left stick to travel and the trigger to the swarm; a wall carried also takes the trigger.
- **The transcript is wrist-anchored, not pointed at.** While it shows, the ray is hidden and points at nothing (`scene.rs`); its focus is moved by the stick, not by pointing.
- **Holding B to leave watches every press**, whoever took it; it is not something any responder handles.
- **X, Y, the left stick's click and the left grip** mean the same wherever the participant points (`interaction.md`): global commands, not routed.

## Proposal: a responder chain for Tomori

### Actions

Map the physical controls once, per device, to abstract actions, and route actions rather than buttons. The headset's controls and the desktop's keys already stand in for each other in `Buttons` (`desktop.md`); the mapping makes that explicit.

| Action | Kind | Headset | Desktop | Hand tracking | Modal for |
| --- | --- | --- | --- | --- | --- |
| `Choose` | pull: begins, held, ends | right trigger | left mouse button | pinch, later | the hand menu, a wall carried |
| `Confirm` | press | A | Enter | — | a request's 0.6 s grace refuses it |
| `Back` | press | B | Backspace | — | a panel or wall carried latches it until let go |
| `Scroll` | 2D axis, push latched | right stick | Page Up, Page Down, `[`, `]`; the wheel, for an environment's object (built) and a panel | — | a panel or wall carried, the transcript |
| `Move` | 2D axis, push latched | left stick | arrow keys | — | travel, where the environment travels |
| `Grab` | hold | right grip | G | — | the transcript (detach) |
| `Square` | press | right stick click | R | — | a panel or wall carried |

`Scroll` borrows AppKit's `scrollWheel:`, which goes to the view under the pointer rather than to the first responder, as Tomori's stick does; a panel reads it as scrolling, a panel carried as pushing, pulling and turning, a globe as turning. Not actions, and never routed: talking (X), pinning the menu (Y), muting (left stick click), the live microphone (left grip) and holding B to leave, which observes every `Back` press.

### Responders

A **responder** is anything that can take an action: it says which actions it handles, and, for each, whether it accepts it now. They come in three tiers, which is where Tomori differs from a single AppKit chain:

1. **Modal responders**, which take focus while they are up: the hand menu while it shows (and while pinned), the right wrist's transcript while it shows, a panel, wall or card carried, a teleport being aimed, and the conversation's own state for `Back` and `Confirm`: the list of modes, a selected queue entry, waiting for a name, the card saying no speech was heard, the participant's and the space's problems, a request in its grace.
2. **The pointed responder** and its **containers**: the nearest thing under the ray, as today's `Aim` resolution finds it, then what contains it. A panel's container is its wall; a menu row's, the hand menu; an environment object's, the environment's client side; and nothing pointed at is the client side, where there is one. The chain from a panel never reaches the environment, whose object is not its container.
3. **Global handlers**, after the chain: the swarm for `Choose` (attracting, and summoning only for a pull begun on nothing of Tomori's or the environment's), result cards for `Scroll` outside a space, the lobby for `Move` outside a space, and the notice that holding B leaves. One global handler comes **before** focus, as a key equivalent: travel, for `Move`.

| Responder | Handles | Accepts when |
| --- | --- | --- |
| Hand menu | `Choose`, `Confirm`; `Back` while pinned | shown; `Choose` and `Confirm` act on the row pointed at, and are absorbed elsewhere |
| Transcript | `Scroll`, `Confirm`, `Back`, `Grab`; absorbs `Choose` | shown; `Back` with something to back out of |
| Panel carried | `Scroll`, `Square`, `Back` | carried; `Back` latched until let go |
| Wall carried | `Scroll`, `Square`, `Back`, `Choose` | carried; `Back` for an owner |
| Wall held, not carried | `Back` | held |
| Conversation's state | `Back`, `Confirm` | a list open, an entry selected, a name awaited, a problem shown, a request waiting |
| Panel pointed at | `Scroll`, `Confirm`, `Back`, `Grab` | `Scroll` with more than it shows; `Confirm` on a checklist item, for a participant; `Back` says how to throw it away |
| Wall board, handles | `Grab` | where walls may be taken hold of |
| Card | `Grab` | — |
| Worker | none (attention only) | — |
| Environment object | as it declares: `Choose` and `Back` (`selectable`), `Scroll` (first increment), more later | it says so in `Picked` |
| Environment client side | `Scroll`, `Move` (ambient) | the ray on nothing of Tomori's |
| Travel | `Move`, before focus | the environment travels on this headset |
| Swarm | `Choose` | always; summons only from nothing |

### Each action keeps its declared order

Because the same modal responders come in different orders for different actions (above), the order stays **per action**, as `interaction.md` already requires: each action has a list of **slots**, and each slot is filled, this frame, by whichever responder occupies it, or by none. Today's ladders are those lists. `precedence::Right` becomes `Scroll`'s: carried, transcript, pointed (Tomori's), pointed (environment object), environment client side, cards. `precedence::Trigger` becomes `Choose`'s, `conversation::Back` `Back`'s, and a new `precedence::Confirm` declares A's, which has no declared order today.

The generic part is how a slot claims: the pointed slot asks the focused responder `handles(action)` and then its container, rather than a hand-written closure over fields of `Step`, so that a new kind of thing to point at is a new responder, not a new field and a new rung in several ladders. This is AppKit's `-respondsToSelector:` and `nextResponder`, applied within each action's order, rather than one chain for every action.

### One gesture, one owner

Every action that lasts has the same ownership rule, which today is written five times: the owner is decided as the gesture begins (a push leaving the middle, a pull passing its threshold, a press, a grip squeezed), held until it ends (back to the middle, let go), and then free. A gesture whose owner goes away, such as a client side replaced as the space changes, is **nobody's** until it ends, as `Selector::abandon` does today (`client_side.rs`). Losing focus ends every gesture. One generic `Gesture<Owner>` type replaces `RightStick`, `LeftStick`, `Selector`'s pull, `throwing` and `removing`, so that no gesture is split between responders.

### The interface for environments

As built, in `tomori-scene`'s `client_side.rs`, whose `Picked`, `Input` and `Select` were already `#[non_exhaustive]`:

```rust
/// Which actions an object a client side picks takes, beyond selecting it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Actions(u8);

impl Actions {
    pub const NONE: Self;
    /// The right stick's push, and the desktop's keys and wheel that stand for it: `Input::scroll`.
    pub const SCROLL: Self;
    pub const fn contains(self, other: Self) -> bool;
}
impl std::ops::BitOr for Actions { /* … */ }

#[non_exhaustive]
pub struct Picked {
    pub object: String,
    pub label: String,
    pub distance: f32,
    pub selectable: bool,
    pub actions: Actions,
}

impl Picked {
    pub fn selectable(self) -> Self;                   // as before
    pub fn handles(self, actions: Actions) -> Self;    // adds to `actions`
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Scroll {
    pub object: String,
    pub axis: Vec2,
    pub phase: Phase,
}
impl Scroll { pub fn new(object: impl Into<String>, axis: Vec2, phase: Phase) -> Self; }
// The wheel's axis may be small, or zero while its push lingers: a client side that applies a dead zone of its own
// drops a slow scroll of a touchpad.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Phase { Began, Changed, Ended }

#[non_exhaustive]
pub struct Input<'a> {
    // … as before …
    pub scroll: Option<Scroll>,
}
impl Input<'_> { pub fn with_scroll(self, scroll: Option<Scroll>) -> Self; }
```

Selecting stays `selectable`, apart from `actions`, rather than becoming `SELECT | BACK` constants as the proposal first sketched, so that the first step changes nothing about selecting; folding selecting into `Actions`, and splitting the trigger from B, is left for when an environment needs it.

The frame step keeps, beside `Client::focus`, the object of the frame before that takes `SCROLL` (`Client::scrollable`); `precedence::Right` has `Object` between `Tomori` and `Environment`, claimed by that object as a push leaves the middle; the push's owner is `RightStick::Object`, with a number no other push shares, and a push whose object's client side went, or that focus left, is `RightStick::Abandoned`, nobody's, until the middle; a wheel's turn is a push of its own, numbered likewise, for as long as it lasts; and `client_side::Scroller` tells each push `Began` once and `Ended` once, a push that replaces another beginning on the frame after the other ends, so that a frame tells one.

The server side is not involved. Input is local to the participant's device; the client side turns what it is told into what it draws at once, and into intents, which its server side validates against its own state and shares (`environments.md`, `65`).

### Worked example: Observe's globe

```rust
impl ClientSide for Observe {
    fn pick(&self, ray: &Ray) -> Option<Picked> {
        let distance = self.globe.hit(ray)?;
        Some(Picked::new("globe", "the globe", distance).handles(Actions::SCROLL).selectable())
    }

    fn update(&mut self, input: Input<'_>) -> Output {
        let mut output = Output::new();
        self.apply(&input.updates); // the server side's state: the globe's shared yaw
        if let Some(Scroll { object, axis, phase, .. }) = &input.scroll
            && object == "globe"
        {
            let dt = input.now.saturating_duration_since(self.last).as_secs_f32();
            self.offset += axis.x * TURN_RATE * dt; // local, smooth, this participant's at once
            let live = input.now.saturating_duration_since(self.sent) >= Duration::from_millis(400);
            if *phase == Phase::Ended || live {
                output = output.with_intent(json!({"turn_globe": {"yaw": self.shared_yaw + self.offset}}));
                self.sent = input.now;
            }
        }
        self.last = input.now;
        output.drawing(Arc::new(self.drawn()))
    }
}
```

Intents are dropped beyond 8 at once and 4 a second after that (`environments.md`), so the globe turns locally every frame and sends `turn_globe` at most two or three times a second while turning, here every 0.4 s, and once as the push ends. The server side takes `turn_globe`, clamps or snaps it as it decides, shares the new yaw in its state, and every participant's client side draws it; the turning participant's client side drops its offset once the shared yaw has caught up. An environment that wants the turn to be each participant's own, as a panel's scroll is, sends nothing and keeps the offset. A push begun on empty sky still reaches `right_stick`, so an environment that pans its whole view with the stick keeps doing so, and one that does not ignores it.

### Migration, in increments

Each increment keeps every existing test passing, changes no documented behaviour unless it says so, and is reviewable alone.

1. **Objects that take the right stick: built** (see Requirement). `Actions`, `Picked::handles`, `Input::scroll`, `Scroll`, `Phase`, the `Object` rung, and the desktop's keys and wheel. Additive: an environment that marks nothing is unchanged, and the farmyard needed nothing.
2. **One gesture type.** Replace `RightStick`, `LeftStick`, `Selector`'s pull and B's `throwing` and `removing` with one `Gesture<Owner>`. No behaviour change; the existing tests of each are the safety net, and the type gets its own.
3. **Declare A's and the grip's orders: half done.** A's is declared, `precedence::A` (#291); the grip's, `precedence::Grab`, is not, nor is B run from its declared order. Declare the grip's as a list like the others, and run B's `conversation::Back` from its declared order rather than an `if` chain in two files. An **equivalence test** enumerates every combination of the situations that decide each control (menu shown or pinned, transcript shown with or without something to back out of, panel or wall carried, wall held, `Aim`, scrollable, selectable, takes the stick, travels, client side present, each of the conversation's states, request in its grace), about 2^15 cases, and checks that the declared order picks the same rung as the code it replaces, before that code is removed; A's order, declared by #291, is in it too.
4. **Responders for what is pointed at.** One `Pointed` value per frame (a panel, a wall part, a card, a worker, an environment object, or nothing), each with `handles(Action)`, replacing the parallel `card`, `worker`, `object` and `panel` options and `Aim`; the pointed slots of every order ask it. The equivalence test of increment 3 stays and must still pass.
5. **More for environments**, each its own decision for the maintainer: `Move` to an object that takes it where there is no travel; `Confirm` (A) for an object that takes the stick without being selectable, since A reaches only selectable objects (#291); the ray during a `Choose` pull, for dragging; a hint on the ray naming what an environment object takes, as `Stick ↕  scroll` does for panels; and `Grab` for environment objects, which `environments.md` names as missing for the GitHub cards.

Not worth generalising: holding B to leave, which is a watch over every press rather than an action; X, Y, the left stick's click and the left grip, which mean the same everywhere; the console, whose letters are commands, not pointing; and the transcript's own focus model, which is a list moved by the stick rather than pointed at. A retained-mode widget tree is not needed: what is pointed at is already resolved once a frame.

### Costs and risks

- **The orders are carefully tuned.** B's and A's orders went through many reviews (`interaction.md`), and the trigger's summons depends on exactly which pulls are the swarm's own. A chain that reorders anything silently would undo those decisions; hence orders stay per action, and increment 3's table-driven equivalence test, old against new over every combination, comes before any code is removed.
- **Per frame**, the work is a handful of comparisons for each action, once a frame, less than today's resolution of what the ray meets; no allocation is needed but the object's identity, copied once as a push begins, as `Client::focus` is copied once a frame.
- **Hand tracking** has no stick, A or B, so nothing changes for it; a pinch as `Choose` becomes one mapping when it is designed.
- **The desktop window** gets each action from its keys, as now; its wheel scrolls a panel, and, since the first increment, pushes an environment's object that takes the stick as far as it would scroll a panel, at most at the stick's rate.
- **Focus stays visible.** The ray ends on the responder under it, highlighted, with a tick as it moves onto a new one (`interaction.md`, `feedback.md`); an environment object taking the stick shows no hint yet (see Requirement and Open questions).
- **Modality** is where a single chain would go wrong: the hand menu and the transcript each pre-empt different actions, and carrying pre-empts some actions but passes `Move` to travel and, for a panel, `Choose` to the swarm. Writing that as each responder's `handles` plus each action's order keeps it explicit.
- **Two ways to read the stick.** After increment 1, a client side may see a push as `scroll` or as `right_stick`, never both; a downstream that reads only `right_stick` and marks an object `SCROLL` loses those pushes from `right_stick`, which is why marking is opt-in.

### Alternatives considered

- **Only an axis for the pointed object, with no chain.** Increment 1 alone. It answers the globe at once and costs little. It leaves A, the grip and B's split as they are, and each further use, such as `Move` or the wheel for objects, would be another special case like it. Recommended as the first step, not the last.
- **Keep per-control ladders as they are.** They work and are tested, and the rule that a new use is a new rung has held. But each new kind of thing to point at threads new fields through `Step` and new closures through every ladder, and the grip has no declared order to test (A's was declared by #291).
- **One AppKit responder chain for every action.** Elegant, and familiar. It cannot reproduce today's orders, since the trigger and B order the hand menu and carrying differently, and travel comes before focus; it would change tuned behaviour.
- **A retained-mode UI toolkit**, with widgets, hit testing and event bubbling. Far more than Tomori needs: pointing is already resolved once a frame, the responders are few, and the environment's objects are not widgets but whatever its scenery draws.

**Recommendation:** increment 1 is built; next, increments 2 and 3, which change no behaviour and give the grip a declared order, beside A's (#291), with an equivalence test; and decide increment 4 once 3 shows how much it simplifies. Increment 5's items are each the maintainer's call.

## Rationale

The maintainer asked whether Tomori could copy the first responder model, so that, for example, pointing at a globe and pushing the right stick turns it, and whether what panels do for scrolling is generic enough (see Status). Reading the frame step shows that focus following the pointer is already generic, that the orders of who takes each control were declared for three controls of seven, four since #291 declared A's, and that ownership through a gesture is one rule implemented five times. Before the first increment, an environment could turn a globe only by reconstructing, in its own client side, which object a push began on, and it was given the stick on empty sky too.

Telling the object a push began on, only for objects that say they take it, mirrors how `Picked::selectable` made the trigger and B opt-in for each object (`environments.md`), and keeps the rule that whatever the participant points at gets the input, Tomori's own interface first (`interaction.md`): the new rung sits after every rung of Tomori's, so a panel in front of the globe still takes the push. A beginning and an end are told, rather than inferred from zero, so that a client side can send one intent as a turn ends, within the intent limits. Exactly one end is told for each beginning, however the push ends, and the push never moves to another object or to anything of Tomori's that appears meanwhile, as every push belongs to what took it, so that a client side never sees a turn split or left open. A push whose client side has gone cannot be told its end; the new client side is told nothing of it.

The desktop's wheel pushes an object as the stick would, at an axis that scrolls a panel as far as the wheel turned, rather than telling notches apart from the stick, so that a client side written for the stick works with the wheel unchanged: integrating its axis at the stick's rate, 15 rows a second at ±1, it goes as far as a panel scrolls for the same turn. The rows the wheel turned are kept and pushed at most at the stick's rate, the last frame only as far as is left, so that a touchpad's fraction of a notch is a small push rather than a whole frame all the way; at most two notches are kept, so that a fast spin ends within half a second of the wheel stopping; and the push lingers a tenth of a second once it has pushed all it was turned, so that a slow, steady scroll is one push rather than a beginning and an end each frame, which would have sent an intent on each end far beyond their limit. The first version pushed all the way for 0.2 seconds a notch with no limit, which the review found turned a slow touchpad into alternating beginnings and ends and fifty quick notches into ten seconds of spinning. Telling notches as an axis beyond 1 would break the stick's range, and a separate field would be a second API for one gesture.

A hint on the ray for an object that takes the stick was considered and left open: panels have a label on the ray, which says how to scroll them, while an environment's objects have none, and adding one, with what it says and who writes it, is more than the first increment.

A single chain for every action, as AppKit's, was considered and set aside because the trigger and B already order the hand menu and carrying differently, and travel comes before what is pointed at; responders filling the slots of each action's own declared order capture what is shared without moving any tuned decision.

## Scope

`tomori-scene`'s `client_side.rs` (`Picked`, `Input`, `Actions`, `Scroll`, `Phase` and `Scroller`), the frame step's routing of the right stick and the wheel in `scene.rs`, its order in `scene/precedence.rs`, and its tests in `scene/tests/scroll.rs`, with the text of `environments.md`, `interaction.md` and `desktop.md` that describes them, for the first increment. The later increments would cover `scene.rs`'s gesture ownership, `conversation.rs`'s `Back` and `accept`, and the frame step's resolution of what is pointed at. The spectator, the console, the server and the protocol are untouched.

## Validation

For the first increment:

- Tests of `client_side.rs` check that `Picked::handles` adds what it is given and leaves selecting apart, that nothing takes the stick unless it says so, that an `Input` is told no `scroll` unless built with one, and, over short sequences of frames, that `Scroller` tells each push `Began` once, `Changed` while held and `Ended` once, ends one push before beginning the next, and follows a push by its number rather than its name.
- Tests of the frame step, in `scene/tests/scroll.rs`, with a client side of their own whose globe takes the stick, or not, check, as a table of situations, that a push reaches the rung of `precedence::Right` each names: a panel carried, the transcript, a panel in front that scrolls, one that shows everything, the hand menu, the globe, nothing, and without focus nobody; that a client side that marks nothing, or a globe not pointed at, is told the same `right_stick`, frame by frame, as the rule before, through pushes, rests and rolls of the stick, in every situation; that a push on the globe is `Began`, `Changed` and `Ended` with `right_stick` zero throughout, stays the globe's as the ray leaves it, and a push begun elsewhere is `right_stick` even once the ray comes to it; that the push ends once however it ends, by blurring or buttons without focus, the next push beginning anew, and never moves to a panel carried, the transcript or the hand menu appearing meanwhile, nor ends early as the globe goes; that a push outliving its client side tells the next nothing until the middle; that a notch of the wheel pushes the globe all the way until it has gone three rows, the last frame as far as is left, and another notch meanwhile goes on with the same push; that a touchpad's twentieth of a notch a frame for 60 frames is one push, told about three quarters of the way each frame, going as far as it was turned within 5%; that fifty notches in 0.8 seconds keep at most two and end within half a second of the last; that the wheel pushes nothing for notches that cancel, without focus, at a globe that does not take the stick or at a panel in front; that it gives way to the stick, and waits for a push of the stick to be told its end without losing any of its turn; that buttons read several times between frames tell each push the client side sees once; and, through the desktop window, that Page Up, Page Down, the brackets and the wheel push the globe the mouse points at.
- `precedence.rs`'s tests, which check every ladder, cover the new rung, and that it comes after every rung of Tomori's and just before the environment's client side.
- Region coverage of everything added or changed is complete (`design/testing.md`), and the farmyard's and the GitHub prototype's tests pass unchanged, since neither marks an object.

For any later increment, the equivalence test (increment 3) over every combination of the situations that decide each control must pass against the code it replaces before that code is removed.

## Open questions

- Whether to give the left stick, as `Move`, to an object that takes it where the environment allows no travel, so that a globe can be tilted with one stick and turned with the other.
- Whether an environment's object should have a label on the ray, as a panel has, and with it a hint for the stick, as `Stick ↕  scroll` does for panels, and who writes it: the environment, through `Picked`, or Tomori, from the actions it declares.
- Whether A, which since #291 selects an environment's selectable object, should also reach an object that takes only the stick, or be told apart from the trigger, which `Input::select` does not do.
- Whether `selectable` should be split into the trigger and B separately.
- Whether `Scroll` is the right name for the right stick's action, after AppKit's `scrollWheel:`, or whether `Turn` or `Adjust` reads better for what a globe does with it.
- Whether a client side should be given the ray during a `Choose` pull, for dragging, as it is given only `pointed` today, since `pick` takes the ray by shared reference.
