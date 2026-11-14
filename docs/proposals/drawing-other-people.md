# Proposal: Drawing Other People

Status: **proposal, decided**, November 2026. The maintainer agreed with the recommendation and added a gaze cue (see **Decisions**). Nothing here is a requirement until a stage brings it into the design documents it changes (listed at the end), which take precedence.

It builds on presence as the protocol carries it (`design/spaces.md`, `design/clients.md`), the others' hands drawn between their updates for what they hold (#417, `tomori-scene`'s `relayed`, `design/physics.md`), the two rays and when each is shown (`docs/proposals/two-handed-pointing.md`, `design/interaction.md`), names and handles (`design/security.md`), the agent's `presence` tool (`design/mcp.md`), the render world and culling (`design/rendering.md`, `docs/proposals/rendering-architecture.md`), and the spectator's frame step (`design/spectator.md`).

## The request

On #417's review the maintainer noticed that nobody is drawn: in a space you can only tell where someone is from what they hold. Asked whether to draw other people's heads, hands and rays, they said:

> Draw other people's heads, hands and rays — yes, it makes sense to me to go in that direction.

On this proposal's review they said:

> I think we can have some fun stylising these — we don't have to be "humans" or trying to make it photo-realistic. Probably seeing that someone is looking at you is helpful for conversations. I basically agree with your proposal.

## Recommendation in brief

> **Revised after stage 1:** a person is now a small swarm of cubes shaped as a head, shoulders, a torso and arms, with no legs, its head a rounded cluster with two eyes that warm as they look at you and an orbit that quickens as they speak. See **The look, revised after stage 1** in section 1 and Decision 8. What follows is the recommendation as first approved.

- **A person is a head, two hands and their rays.** There is no body. Each part is built from the cubes Tomori already draws everything with.
  - **The head** is a rounded block about the size of a head, in the person's colour, with a dark visor band across the front, like a headset. It faces where they look. It is stylised for fun, not a human or photo-realistic likeness, and needs no face.
  - **A gaze cue:** while someone looks at you, their visor band glows towards you, so eye contact reads in conversation. It is worked out from the head pose already relayed, so nothing more is sent.
  - **A tracked hand** is drawn as its joints and bones, tinted, the way the headset already draws your own tracked hands.
  - **A controller** is a small stylised controller in their colour, not the runtime's model of your own hardware.
  - **A ray** uses your own ray's style, in their colour. It is drawn only while it is drawn in full on their headset, ending in a small dot on whatever it meets. A stub or a hidden ray is never drawn.
- **A small name label above the head**, on the same dark plate as a worker's sign. It shows their name, or their handle (`Ada#23`) while two people in the space share the name. It is shown within about 6 m, while they speak, and while you point at them.
- **A colour for each person** from a palette of eight evening-friendly hues, kept clear of the swarm's violet and the state colours. The hue comes from their tag, so it stays the same across reconnects and devices. Colour is never the only thing that tells two people apart.
- **Motion on #417's timeline.** Heads, hands and rays are drawn between updates, one update behind, from the same `Relayed` tracks that what someone holds already follows, so a stick and the hand holding it can never disagree. There is no extrapolation. A jump of more than 1.5 m is cut, not glided. Someone silent for 2 s fades out. Someone who leaves fades out where they stood.
- **Privacy: nothing new about your body is sent.** Head, controllers and hand joints already go to everyone in the space. Only two things would newly be sent: each ray's aim and length while it is drawn in full, and the tag. A per-device setting, *Others see: everything / head only / my name only*, limits what this headset sends. Nothing is recorded.
- **Cheap.** About 110 cube instances per person, in the batch Tomori's cubes already draw, plus one label. That is roughly 1.3 k triangles and two draws per person, culled by the render world, with less detail at a distance.
- **Staged:** (1) heads, the gaze cue and rays; (2) hands and controllers, with the privacy setting; (3) colours, labels and handles; (4) pointing at people, others' ray ends on panels, and level of detail.

## What exists today

| Part | Today | Where |
| --- | --- | --- |
| Presence sent | Every frame (~72 Hz on a Quest 3, unevenly): head pose, left and right controller poses while tracked, both tracked hands' 26 joints with radii, plus `carried` and `rays`, which are the server's alone | `ClientMessage::Presence`, `tomori_scene::scene::Presence` |
| Presence relayed | `source`, head, left, right, hands, to every other member of the space, as sent. `carried` and `rays` are not relayed | `tomori-server`'s `hub.rs`, `ServerMessage::Presence` |
| Controller poses | The headset sends the **grip** poses it draws its own controllers at (`xr.rs` binds `input/grip/pose`). Its rays come from the **aim** poses, which are not sent. `presence::Seen` on the server calls the sent poses aim poses, and the agent's `presence` reads pointing along their −Z, so it points along the grip, not the aim (confirmed; stage 1 fixes it) | `client-quest/src/xr.rs`, `server/src/presence.rs` |
| Size | About 400 B a frame with controllers (≈ 28 KiB/s) and 3.9 KB with both hands tracked (≈ 280 KiB/s) per sender, as JSON, multiplied by each other member | measured on representative messages |
| Drawn | Nobody. Environments' client sides get the others' hands (`Input::hands`), drawn between updates about a frame behind (`relayed`), for what they hold. The spectator draws the participant it follows: their controllers and hands, as theirs | `scene/relayed.rs`, `spectator.md` |
| Names | `SpaceState::participants` gives each person's name, `source` and engagement. Tags are the server's and the agent's and are never sent to clients | `security.md`, **Tags** |
| Pointing at people | The agent's `presence` resolves a ray or gaze meeting another participant's head, within 0.3 m. A participant cannot attend to a person as they speak, since `Resource` has no participant kind | `mcp.md`, `spaces.md` |
| Two people in one place | Expected: both standing at the middle of their own play space arrive at home together | `spaces.md` |

## 1. What others look like

**Options for the head:**

| Option | For | Against |
| --- | --- | --- |
| **A. Visor block** (recommended): a rounded box about 18 × 14 × 16 cm in their colour, with a dark band across its front and a slightly lighter top | Reads at once as a person in a headset and as facing a way. Made of the cube primitive, so it is hazed, lit and culled like the rest of Tomori. No assets | Abstract. Expresses nothing but where they look |
| B. A small flock of cubes | Matches the swarm | Confused with the agent and its workers, the very things people point at |
| C. A face or mask | Warmer | Uncanny without expression data. Picks a face for someone who did not choose one |
| D. A glTF character | Rich | Needs assets and an animator per person, and choices of body and identity. Costs a skinned draw per person. Better left to a later "a build registers a person look" hook, like the agent's avatars (`avatars.md`) |

**Stylised, for fun.** The visor block is the starting point, not a final likeness. People are not drawn as humans or photo-realistically, and need no face. Playful shapes and colours within Tomori's cube style are welcome: a tilted visor, ears or antennae of small cubes, a two-tone top. Whatever the look, its front and the way it faces must read at once, since the gaze cue and conversation depend on it.

**The gaze cue.** Seeing that someone is looking at you helps conversation, so it is shown on their head:

- **When:** another person's head forward is within about 10° of the line from their head to yours, and you are within about 8 m. Hysteresis keeps it steady: the cue starts within 10° and holds until beyond 15°, and it eases in over 0.2 s and out over 0.4 s, so a glance across you does not flicker.
- **How:** their visor band brightens and glows towards you, a little lighter on the side that faces you. In stage 3 their label also lifts slightly while they look at you, as it does while they speak.
- **Only yours:** each headset works it out for itself, from the head poses it already receives, about one update behind like the rest. Nothing new is sent, and nobody else sees that you are being looked at. The spectator shows the cue towards the participant it follows.

**Hands.** A tracked hand is drawn as joints and bones (`BONES`), small cubes tinted towards the person's colour. That is what the renderer already draws for your own hands, and its geometry is already in presence. A controller is drawn as a small rounded body with a tracking ring, in their colour, at its grip pose. The runtime's render model shows *your* hardware, which may not be theirs, and it is a textured mesh for each hand. Where presence carries both joints and a controller pose, as with Meta's simultaneous hands and controllers, the hand is drawn. A hand that loses tracking fades over 0.15 s rather than freezing. Switching between controllers and hands cross-fades.

**Rays.** Another person's ray is drawn only while it is **drawn in full on their own headset**: while it meets something it can act on, while its trigger or grip is in use, while it carries, and for its second after. It is drawn in your own ray's style, in their colour, a little fainter, and ends in a small dot of their colour where it meets something. An idle stub is never drawn to others, so a room of four people is not a room of eight beams. A teleport aim is not drawn either, since it is personal.

**No body.** A floating head and hands is the convention people already read in social VR. A body would have to be guessed from three poses. A faint disc on the floor beneath the head, as the agent's home ring sits, may help judge distance. It is left to stage 4 for a look on the headset.

### The look, revised after stage 1

Seeing stage 1's visor block on the headset's preview, the maintainer found it minimal and asked for something more suited to Tomori: "We use a swarm for you, why not a simpler swarm for humans, roughly showing body and arms at least. Legs are less interesting. Head — maybe we can spend a bit more effort on it." Of three concepts, a torso and arms of glowing cubes dissolving below the chest, three heads, and a person whose eyes warm as they look at you beside the agent's violet cloud, they found all three "very cool, even if we had to use a few less cubes", and chose the second head: a rounded, dense cluster with two soft glowing eyes and a small orbit of tiny cubes. Option B above, a flock of cubes, was set aside for being confused with the agent; shaped as a person, structured where the agent is shapeless, and in a person's colour, far from its violet, it is not.

- **The head** is a rounded, dense cluster of cubes a little larger than a head, facing where they look, with **two eyes** on its face, soft and cool, and a little **orbit** of tiny cubes circling it.
- **The gaze cue is the eyes:** while someone looks at you, by stage 1's rule (within 10°, holding to 15°, within 8 m, easing in over 0.2 s and out over 0.4 s), their eyes warm to a bright gold, grow a little, and the face about them catches it, the eye towards you a little warmer.
- **Speaking is the orbit:** while their voice plays on your headset, which the client knows by source, their orbit quickens and brightens, calming over half a second once it stops. Stage 3's label will show it too.
- **The body** is shoulders and a torso, below and behind the head, turning with its yaw alone, damped, so that nodding and looking down bend nothing. It thins below the chest into a few cubes that drift down and fade: **no legs**, since nothing tracks them.
- **The arms** reach from each shoulder to each hand by two-bone inverse kinematics, the elbows bending out and down, to the controller's grip or a tracked hand's palm; a hand neither tracks hangs at the side. Each hand is a small, brighter cluster; tracked joints may become a few cubes of it in stage 2.
- **Motion:** every cube drifts a little about where it rests, cheaply, with a phase of its own, on stage 1's timeline, and every fade dissolves the cubes where they rest.
- **Light, refined after the first build:** the cubes are the person's own light in their colour, brightest at each part's middle as seen and dimmer and smaller at its outline, shimmering a little, so that a person glows as the concepts did, translucent-looking at the edges by brightness alone, with no blending, sorting or pass. They are small and of many sizes, with air between them, and dissolve strongly below the chest into cubes that fall and fade; the eyes are small, round and soft, never two flat squares. Everyone is a warm coral until stage 3's colours, which the swarm takes whole (`Cues::colour`).
- **Readable at a distance, refined again:** the second build went too sparse to read as a person from 3.6 m, so the third is the middle ground: a head twice as dense, packed round like head B; dense shoulders and upper chest, sparse only at the edges and below the chest; arms of a few slender rods along each bone, as the maintainer suggested, tapering to the wrist, which read as one limb where small cubes broke into dots; the hands small, bright clusters; two crisp, round eyes, each three flat boxes in a socket that frames it, warming to gold with a halo behind; and a tidy orbit. With no bloom pass, glow is a larger, dimmer copy behind the eyes and hands, by brightness and size alone.
- **Budget:** 396 cubes a person in full, 176 beyond 5 m, and the head's 42 alone beyond 15 m, in the batch Tomori's cubes already draw, with no new draw call. On the desktop's RTX 4090, two eyes of 1832 by 1920 with four samples, eight people in view add about 0.07 ms of the GPU's frame and about 38,000 triangles an eye; a Quest 3 would spend roughly 0.75 ms for four people and 1 ms for eight, and about 0.4 and 0.7 ms of its CPU, which its frame log is to confirm. A frame with others in the space allocates nothing.

`design/people.md` describes it as built.

**Not "avatars".** `avatars.md` uses "avatar" for the agent's representation. These would live in a `tomori_scene::people` module and a `Frame::people` element, so the two never mix.

## 2. Identity and colour

- **Colour from the tag.** The tag is a person's stable number (`Ada#23`). The space state would carry it in an optional `Participant::tag`, which older clients ignore. The hue is the tag's place in a fixed palette of eight. Where an earlier participant in the space's participant order already has that hue, the next free one is used, so every client agrees and the earlier person keeps their colour. Without a tag (an older server) the colour comes from `source`, which changes on reconnecting.
- **Labels.** The label shows the name as `SpaceState::participants` gives it, and the handle only while someone else in the space shares the name, the same rule the agent follows. One person on two devices with one token shares a tag and a name; that is the same person, and both are drawn.
- **Speaking.** While someone's voice plays, which the client knows by source, their label shows a small level bar and lifts slightly towards full brightness, so you can tell who is talking where.
- **Engagement.** Someone `away` is drawn at 30% with `away` on the label. Someone `distracted` is drawn as usual.

## 3. Motion

- **One timeline.** `relayed::Relayed` today tracks a `Hand` pose per source. It would become a track of each person's whole presence (head, both controller poses, both joint sets, and the rays added in stage 1), timed by arrival and drawn about one update behind, at most 50 ms (`MOST`). Environments' client sides would read their `Input::hands` from the same tracks, so what someone holds and the hand drawn holding it are the same pose in the same frame.
- **No extrapolation.** Beyond the latest update the person holds still. Interpolation already holds at most 0.8% of frames still with 40 ms of jitter. Extrapolation overshoots on every stop, which shows most on a head.
- **Jumps.** Between two updates more than 1.5 m apart (a teleport), the person fades out over 0.1 s where they were and fades in at the new place, with no glide through walls. This is the threshold signs already use.
- **Silence.** Someone with no update for 2 s fades out over 0.5 s. Their label stays faint where their head was, since they are still in the space's participants. They come back as their next update arrives.
- **Leaving.** Someone gone from the space's participants fades out over 0.3 s where they last were. Their track is dropped. A source number used again is drawn anew, as `Relayed::since` already does.
- **Up close.** Others are drawn fainter within 0.6 m of your eyes, and not at all within 0.3 m, so two people arriving at home together are never inside each other's heads.

## 4. Privacy and consent

**Already sent, to everyone in the space, visitors and the spectator included:** your head pose; your controllers' grip poses; and, while hands are tracked, 26 joints of each hand with their radii. That is enough to see finger poses and estimate hand size. Drawing people only shows on the headset what anyone in the space could already read off the wire.

**Newly sent:**

- each ray's aim pose and length, but only while that ray is drawn in full (stage 1);
- your tag, in the space state (stage 3). The agent already says it aloud where names collide.

**A setting, per device, in the hand menu's People & security section:** *Others see: everything* (the default) / *head only* / *my name only*.

- **Head only** stops this headset sending controllers, joints and rays. Environment objects you hold then follow your head, as they already do for a client with no hands, which `security.md`'s reach checks allow for.
- **My name only** still sends the head, since your voice is placed there and the agent follows you by it. It asks others not to draw you, and they draw only a faint label. This hides you from view, not from the server. The setting should say so in one line.

Neither choice changes your role or what you may do.

**Nothing is recorded.** Presence is held in memory as the latest report, as now, and never written to disk or logs. People are drawn by the spectator, which is not personal (`Frame::elements`), so a spectator's video shows them, as it already shows the followed participant's hands. Capturing it is the operator's choice.

## 5. Network and cost

**Network.** Drawing adds nothing to what is sent in stages 2–4 beyond the tag, a few bytes once in the space state. Stage 1's rays add about 130 B a frame for each ray drawn in full, about 9 KiB/s while pointing, and nothing otherwise. Today's load is the concern, and it is unchanged. Four people with both hands tracked each download about 830 KiB/s, and the server sends about 3.3 MiB/s. If that matters, a later change can send presence in a compact binary form (joints as 16-bit offsets from the wrist cut it about fourfold) or relay hands less often to someone far away. Neither is needed to draw people.

**GPU, per person on a Quest 3** (as first proposed; the swarm, revised after stage 1, is 396 cubes in full and 176 beyond 5 m, about 4.8 k and 2.1 k triangles, as **The look, revised after stage 1** says):

| Part | Instances |
| --- | --- |
| Head | ~4 cubes |
| Two tracked hands | ~100 cubes (joints and bones) |
| Two controllers | ~10 cubes |
| Ray | 2 cubes |

That is about 110 instances and ~1.3 k triangles, in the instanced cube batch already drawn: no new draw calls, hazed and lit as the rest are. The label is a sign face laid out once per name and mesh kept, so two draws a person, one per eye. With eight people that is about 10 k triangles and 16 draws, which should cost well under 0.1 ms of the GPU frame. Stage 1 confirms this with the frame log's GPU line on the headset.

**CPU.** Interpolating a person's head, controllers and 52 joints is a few microseconds a frame. Labels are laid out only as names change.

**The render world.** People submit to `tomori-world` as Tomori's cubes already do: one dynamic item per person in the scene's root, culled whole then piece by piece. Labels are culled too, unlike workers' signs today.

**Level of detail by distance:**

- beyond 5 m, a tracked hand is drawn as one palm block, and the swarm with fewer, larger cubes, 176 of its 396 (built with the swarm, revised after stage 1);
- beyond 15 m, only the head and label are drawn, the swarm's head alone, 42 cubes;
- beyond 40 m, only the label is drawn;
- labels fade out beyond 6 m unless the person speaks or is pointed at.

## 6. Interaction

- **Others' parts are not in your way.** Others' hands, controllers and rays are never hit by your ray. Your grip never takes from someone's hand; the physics server's reach rules already refuse that.
- **Pointing at a person.** Your ray meets another person's drawn head, a 0.15 m sphere, as it meets anything else: the nearest thing wins. Someone standing in front of a panel blocks it, as they would in a room. Pointed at, their label brightens and shows. As X is pressed, they are attended to as a new `Resource::Participant { source }`, which an older server skips. The server describes it as `[Pointing at Ada]`, or by handle where names collide, the way `presence` already names people. The server's 0.3 m head radius for `presence` stays generous for gaze.
- **Others' ray ends on shared panels.** Where someone's ray ends on a panel or wall, a small dot of their colour is drawn on its face (stage 4), so "this line here" can be followed. It never highlights the panel's frame or an item, since those mean *your* hover. A chart's readout stays the pointer's own.
- **Occlusion.** Heads, hands and controllers are opaque cubes. They hide what is behind them and are hidden by what is in front, as `rendering.md` requires of everything.

## 7. Stages

Each stage keeps every test passing, is reviewable alone, and changes nothing for someone alone in a space.

1. **Heads, the gaze cue and rays.**
   - `tomori_scene::people`, with `Relayed` generalised to whole presence; environments' hands read from the same tracks.
   - The visor head, stylised, in a neutral colour, fading on silence, leaving, jumps and up close.
   - The gaze cue on the visor band, from the relayed head poses alone, with its hysteresis. It belongs with the head because it needs nothing else; the label's part of it waits for labels in stage 3.
   - Rays: presence gains an optional `pointing: [Option<Ray>; 2]`, each ray's aim pose and length while drawn in full. The server relays it and resolves `presence`'s pointing from it, rather than from the grip poses.
   - `Frame::people`, which the spectator draws, leaving out the head of whoever it follows.
   - Tests:
     - `relayed` keeps #417's figures, and a held stick and the drawn hand agree each frame;
     - fade timings;
     - the gaze cue starts within 10°, holds to 15°, does not flicker for a head wavering at the edge, and is shown only to the one looked at;
     - protocol round trips with and without `pointing`;
     - the server relays `pointing` and resolves pointing from it;
     - a lavapipe preview, `TOMORI_PEOPLE_PREVIEW`, of two people at a wall;
     - on the headset, with two devices.
2. **Hands and controllers, and the setting.**
   - Tinted joints and bones, the stylised controller, cross-fades.
   - *Others see* in the hand menu, honoured by what is sent.
   - Tests: drawing chooses hand over controller; each setting sends what it says; head-only holding still passes reach checks.
3. **Colours, labels and handles.**
   - `Participant::tag`, the palette with deterministic collision, labels with handles where names collide, speaking, away, and the label's lift while someone looks at you.
   - Tests: every client gives every person the same colour for one participant list; an older server's state falls back to `source`; handles show only while shared.
4. **Pointing at people and polish.**
   - `Resource::Participant` and its description; others' ray-end dots on panels; level of detail and label culling; the floor disc if it earns its place.
   - Tests: attention resolves and describes a person, the handle where shared; dots are drawn on the face they end on and change nothing of it; the frame log on the headset with four people.

## Decisions

The maintainer agreed with the recommendation, November 2026:

1. **The head:** the visor block to start, stylised for fun rather than human or photo-realistic, with no face (**1. What others look like**).
2. **Tags to clients:** yes, each participant's tag is sent in the space state, for stable colours (stage 3).
3. **Others' rays:** the full beam while it is drawn in full on their headset, ending in a dot (stage 1).
4. **Privacy default:** *Others see: everything*, with *head only* and *my name only* as the setting (stage 2).
5. **Pointing at a person** as attention: later, in stage 4.
6. **A build registering a person's look:** later, not staged here.
7. **New, a gaze cue:** someone looking at you is shown on their head, from the head pose already relayed, with hysteresis (stage 1, the label's part in stage 3).
8. **Revised after stage 1, the look:** a simpler swarm than the agent's, shaped as a person, in their colour: a body and arms, by two-bone inverse kinematics to each hand, and no legs, dissolving below the chest; the head the second concept's rounded cluster, its eyes the gaze cue and its orbit quickening while they speak; about 400 cubes a person in full, fewer beyond 5 m and the head alone beyond 15 m (**The look, revised after stage 1**).

## Design documents this would change

- `rendering.md`: people among Tomori's cubes, labels culled.
- `interaction.md`: pointing at people, others' parts never hit, ray-end dots.
- `spaces.md` and `clients.md`: presence's `pointing`, `Participant::tag`, the setting's effect on what is sent.
- `security.md`: what is newly sent, tags to clients, the setting.
- `physics.md` and `environments.md`: one `Relayed` timeline for hands and people.
- `spectator.md`: draws people, not the followed head.
- `hand-menu.md`: *Others see*.
- `mcp.md`: pointing from aim poses.
- A new `design/people.md`.
