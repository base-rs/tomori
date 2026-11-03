# Interaction in a Space

This describes how the headset presents a space and what its controls do there. Arrival, before a space is joined, is described in `arrival.md`.

## Requirement

**No control has a hidden destructive meaning.** In a space:

- **B** closes or backs out of what is open: the list of modes, a selected queue entry, or the problem shown, which it dismisses. It never decides an approval, cancels queued input or leaves the space. Leaving takes holding **B** for a second; a shorter hold, or a press, never leaves, and while it is held the status line says that holding it leaves.
- **Deny** is one of an approval's choices, beside *allow once* and each standing option, chosen with the right stick left and right and confirmed with **A**, like any other choice. Nothing is decided without **A**.
- Cancelling a queued entry takes selecting it in the queue strip with the right stick up and down, then **A**; it is sent only after 3 seconds, during which **A** undoes it. An entry that started sending meanwhile is not cancelled, and the panel says so. Any participant may cancel any waiting entry, as `spaces.md` allows.
- **A** never resets the swarm in a space, where it confirms the choice shown, chooses from the list, or cancels or keeps the selected queue entry.

**Controls in a space:**

| Control | Meaning |
|---|---|
| A | confirm the approval choice, choose from the list, or cancel the selected queue entry, or undo that |
| B | close the list, deselect a queue entry, or dismiss the problem; hold for a second to leave |
| X (hold) | push-to-talk to the agent; while a question Tomori can answer is presented, the answer to its current field (see `spaces.md`) |
| Y | open or close the list of the agent's modes, with their descriptions, and the space's speech setting; a space with neither fetches its state again |
| Left stick | page through text (up and down), move between messages (left and right), or move in the list |
| Right stick | choose an approval's choice (left and right), or select a waiting queue entry (up and down) |
| Right stick click | play the latest reply again, or stop it (see `speech.md`) |
| Left stick click | mute or unmute spoken replies locally |
| Left grip | toggle the live microphone (see `live-audio.md`) |

**Modes are chosen from a list.** The list shows every mode the agent offers, by name, with the selected mode's description, and marks the current one; it opens on the current mode. Choosing a mode sends it directly, rather than stepping through the modes in between. The list also offers the space's speech setting, brief or full, which choosing switches for everyone in the space.

**Problems stay visible on their own line.** The panel shows the newest problem, this participant's own error or the space's ongoing problem, on a line of its own of up to two rows, marked `!` with how long ago it appeared. Neither the push-to-talk status, "Sending…", the connection status nor anything else displaces it. It stays until **B** dismisses it or a newer problem replaces it; sending something does not clear it. The space's ongoing problem, dismissed on the headset, shows again if it changes. The status line shows what the microphone is doing, the connection, a notice, a replay, or the counts, and is never used for a problem.

**The queue is always visible.** Entries waiting or being sent are shown in a strip of their own above the footer, oldest first, with the speaker, "You" for the participant's own, their state and their words, on up to two rows; a longer queue shows the selected entry and how many are queued. The queue is not an item reached by paging past the last message. While an approval is presented, its choices take the strip's room.

**The panel stands in the world.** Inside a space the panel is anchored in the world in front of the participant, upright, rather than fixed to their head: it stays where it is while they look around. Only when their head has turned more than 45° from it, or moved more than half a metre from where it was placed, for longer than a second, does it glide back in front of them, over half a second. It is placed in front of them again on entering a space. Arrival keeps its own world-anchored labels and shows no panel.

## Rationale

In November 2026 the user reviewed the headset's interaction and found it barely workable (`docs/proposals/interaction.md`): errors were hidden behind "Sending…", the queue was hard to reach, and the head-locked panel swam against the world. B meant deny, cancel or leave depending on hidden state, so a press meant as "never mind" could deny a tool call not yet read, cancel the wrong entry or leave the space. Modes were cycled blind, several presses away, with their descriptions never shown. The user accepted the proposal's first stage, keeping the live microphone on the left grip. Holding to leave, an explicit deny and an undo for cancelling make each consequential action deliberate. A world-anchored panel that follows lazily is the usual remedy for head-locked text in VR; the thresholds keep a glance from moving it, and are to be tuned on the headset.

## Scope

The headset client's conversation model and controls inside a space, and the placement of its panel. The desktop console mirrors the controls as commands.

## Validation

- Test that B in a space never sends a decision, a cancellation or a leave, that holding it for less than a second does not leave, and that holding it for a second leaves once.
- Test that deny is the last approval choice and is sent only with A, that A cancels a selected queue entry only after the undo time, that A within it keeps the entry, and that an entry sent meanwhile is not cancelled.
- Test that a problem is shown while recording and while sending, stays when something is sent, and is dismissed by B; and that every panel line fits.
- Test that the queue strip shows waiting and sending entries, marks the participant's own, keeps the selected entry in view, and is not reached by paging.
- Test that Y opens the list on the current mode with its description, that A sends the chosen mode directly and the speech setting toggled, and that Y or B closes it.
- Test the panel anchor: it stays put while the head turns less than 45° or nods, a glance away of under a second does not move it, a longer turn or a walk brings it back over half a second, and reset places it in front at once.
- On the headset, confirm the panel stays put while looking at the swarm and follows when turning away.
