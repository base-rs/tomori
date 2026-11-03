# Hand Menu

This describes the headset's general menu, held on the participant's hand. It is the place for people and security now, and for settings and other controls as they move off the heads-up panel.

## Requirement

**Showing it.** Turning the left hand, or the left controller, so that the palm faces the participant's head shows the hand menu, anchored just above the wrist and facing the head; turning it away hides it. It works with hand tracking and with controllers, in arrival and inside a space. Showing and hiding have hysteresis, in angle and in time, so that the menu neither flickers at the boundary nor appears when the hand passes through the pose: it appears once the palm has faced the head, within about 35°, for about a fifth of a second, and hides once it has turned more than about 55° away. It moves with the hand while shown. It is in the world, attached to the hand, never to the view, in keeping with `arrival.md`.

**Choosing in it.** Items in the hand menu are pointed at with the right hand's ray and chosen with the trigger, as arrival objects are (see `arrival.md`), with the same hover feedback. While the menu is shown, the right trigger chooses in the menu rather than in the world behind it. Labels are anchored to their items.

**What it holds.** The menu shows what applies to the participant at that moment, and nothing else:

- their display name and role (see `security.md`);
- inside a space, the participants there and how many visitors are watching;
- for a visitor, knocking on the space they are watching, and whether the knock is pending, declined or expired;
- for a participant, the space's pending knocks, each admitted or declined from the menu;
- the pairing code, while the device is pairing;
- security notices, such as that the server's identity has changed or that the device is no longer admitted.

**Cues.** Something waiting for the participant, such as a pending knock, shows a small cue on the left wrist, such as a glow, without the menu being open, so that they know to look. The cue clears once nothing is waiting.

**The general menu.** New settings and controls are added to the hand menu rather than to the heads-up panel, and existing ones move to it over time. The menu's contents come from the client's state and the server's messages; it adds no protocol of its own beyond the actions it offers, which are ordinary client messages.

**Without hands or controllers.** Clients without tracked hands or controllers, such as the desktop window and the console, offer the same items another way, such as a key that opens them in a panel.

## Rationale

The user found that the hand metaphor fits Tomori: in some headset apps, the participant turns their hand to look at their palm or wrist to reach more of the interface and its settings. Knocking, seeing who is connected and handling security need an interface in the world, and a menu that is always at hand but out of sight until wanted keeps the view clear. The user chose it as the general menu, not only for security. Stated by the user in the November 2026 authentication discussion.

## Scope

The headset and desktop client (`crates/client-quest`): showing the hand menu from hand and controller poses, its items, pointing in it, and the wrist cue; and the equivalent for the desktop window and console.

## Validation

- With hand tracking and with controllers, turn the left palm towards the head: the menu appears above the wrist after a brief hold and follows the hand; turning it away hides it. Test the visibility rule as a function of hand pose and time: it appears only after the hold within the inner angle, stays shown until the outer angle, and does not appear for a hand passing through the pose.
- Confirm that the menu stays attached to the hand when the head turns, and that while it is shown the right trigger chooses an item in it and nothing behind it.
- Test the menu's items against client states: a visitor sees knocking and its state; a participant sees pending knocks with admit and decline; everyone in a space sees its participants and visitor count; the pairing code and security notices appear only while they apply.
- Confirm that a pending knock shows the wrist cue with the menu closed, and that the cue clears once the knock is resolved.
- On the desktop and console, confirm that the same items are reachable without hands or controllers.
