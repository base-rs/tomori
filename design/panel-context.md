# Panel Context

This proposes making explicit what an environment's walls show: the context their panels depend on, such as a region and a time window, as state on walls and panels that the agent reads and sets, that everyone sees, and that an environment supplies content for. Pinning a panel pulled off its wall, freezing a chart, and a wall locked to one time window are all special cases of it. Only the linked time cursor, which needs none of the model, is built; it is a requirement of `projection.md`. Everything else here is a proposal, phase by phase, for the maintainer to approve.

## Requirement

None yet beyond the linked time cursor (`projection.md`, **The linked time cursor**). It links charts by their structured time axes (`TimeAxis`): live charts only on one wall, by how long before now, since once this proposal is built each wall may have a time window of its own; and fixed windows, absolute instants in UTC, by the instant. It never reads labels and never matches a time of day. `TimeAxis::Fixed` is the window this proposal's fixed time uses.

## Proposal

### The model

- **Parameters.** An environment's server side declares, as its state changes, at most 3 named parameters its panels depend on, each with the value it has now (the environment's own selection, such as the region its bridge has selected) and, optionally, the values it may take: `Space::set_context(vec![Parameter::new("region", "Region", "US-EAST1").with_choices(...)])`. Each parameter has an id like a panel's, a label of at most 16 characters, and values of at most 32 characters, cleaned onto one line; it has at most 16 choices. Tomori adds a parameter of its own, `time`, which it understands: *live*, the last N minutes, ending now, or *fixed* to an absolute range, two instants in milliseconds since the Unix epoch in UTC, as a chart's `TimeAxis::Fixed` says. It is never a time of day or a local time. A zone appears only in what is shown, such as labels and chips, which say `UTC`. Tomori treats every other parameter's values as opaque text. Tomori has no shared selection of its own: the side says what its selection is.
- **Dependence.** A panel says which parameters it depends on: `Panel::depends_on(["region"])`. A chart panel depends on `time` as well.
- **Settings.** A wall and a panel each hold, for each parameter, a setting: *follows*, the environment's current value (for `time`, live over the window the side publishes), or *fixed* to a value. A panel's effective context is its own setting, else the setting of the wall it is on, else *follows*. Settings are kept with the space, across restarts, as walls and where panels rest are.
- **Instances.** One panel id shows one context. A panel whose effective context differs from the one its side publishes under that id becomes an instance of it, under an id the server mints, such as `~7`, which `valid_panel_id` refuses so that no side can mint or forge one. The side is told each instance's effective context as an intent the server makes itself, as it makes a choice or a move (`panel-actions.md`): `Intent::context()`, naming the panel it is an instance of, the instance's id, each parameter's value and the time window. The side publishes the instance's content under that id, and is told again whenever the instance's effective context changes, and when its side starts again. At most 8 instances an environment in a space, counted within its 32 panels.
- **Fixed time, one rule.** While a panel's effective time is fixed, Tomori keeps what it shows and takes only content the side declares for that window. That content is a chart whose `TimeAxis::Fixed` is exactly the window, or a panel published with `Panel::for_window(from_ms, to_ms)`. A frozen chart already holds its window's content, so it never changes and costs nothing. A wall fixed to a window asks its panels' sides for content for that window, which an environment that keeps history, such as Observe, fetches.
- **No data for a window.** An environment without history, or one asked for a window past its retention, publishes a panel saying it has no data for that window, declared for it. A chart's no-data state does this already. It never shows live data under a fixed window, and Tomori never takes live data for one. Until the side answers, the panel says it is waiting for that window's data.
- **Labels are display.** A chart whose time is fixed has its axis labels written by Tomori as clock times in UTC from its `TimeAxis::Fixed`, the last ending ` UTC`, so that the window never scrolls what was seen away. Labels are only shown; linking, inheritance and the rule above use the structured window alone.

### What the existing behaviours become

| Behaviour | In the model |
| --- | --- |
| An environment's wall following its selection, today implicit | the wall's `region` follows |
| Pulling a panel off its wall | an instance, every parameter it depends on that followed fixed to what it showed; the wall shows a fresh following panel at once, in the slot it left |
| The pinned badge | the panel's chip for a parameter fixed |
| Freeze and unfreeze | the panel's `time` fixed to the window it shows, and back to live |
| A wall locked to a time window | the wall's `time` fixed; panels on it take it for whatever they do not fix themselves |

Only a participant's own move pins a panel; the agent and the layout move it as they do today. A side is never told a participant's pin as anything it could make itself, and nothing a side publishes fixes a parameter.

### The agent controls it

Every tool is declared with strict arguments (`mcp.md`):

- **Reading.** `panels`, `presence` and the description of what is pointed at give each wall's and panel's context: each parameter's value, whether it follows or is fixed, and the time window.
- **`set_wall`** takes `context`, such as `{"region": {"fixed": "EU-WEST1"}, "time": {"from": "2026-11-12T02:20:00Z", "to": "2026-11-12T02:50:00Z"}}`, `{"time": {"live_minutes": 30}}` or `{"region": "follow"}`.
- **`set_panel_context`** `{panel, context}` fixes or follows a panel's own parameters. Freezing is `{"time": "shown"}`.
- **`copy_panel`** `{panel, context, wall, slot}` makes an instance with a context, such as the same chart for every region onto one wall.
- **`create_layout`**'s walls each take `context`, so that "show me the incident's two critical windows on two walls" is one call.

"Line up the 02:30 spike across all regions" is then `create_layout` with a wall fixed to 02:20–02:40, and `copy_panel` of the latency chart for each region onto it.

### Everyone sees it

- A wall with any setting of its own shows a slim strip of chips along its top edge, read-only, such as `Region · US-EAST1`, `Time · 02:20–02:50 UTC` or `Time · live 30 min`. A wall that only follows shows none.
- A panel's own settings show as chips in its chrome below its title: `📌 Region · US-EAST1`, `❄ Time · 02:20–02:50 UTC`. That is the pinned badge.
- At most 4 chips on a wall or a panel: its parameters and time. Pointing at one says what it means on the ray's label. Chips are drawn with the face, laid out again only as the context changes.
- Direct control stays small: pulling a panel off pins it, the time chip of a chart toggles freeze with A, Enter or the trigger, and later a chip may offer the environment's declared choices.

### Tangible time tokens (a concept, not to build now)

Walls have sockets; a time window is a token, "last 30 min" or "02:20–02:50", that is plugged into a wall, pulled out, duplicated and frozen, one token for each critical window of an incident, plugged into different walls. A token is a context value. Plugging it in fixes the wall's parameter to it, and pulling it out lets the wall follow again. Duplicating copies the value. Freezing a live token makes it the absolute window it shows now. The chip strip is the token seen from afar. Nothing in the model changes for tokens: they are another way to set the same state.

### Phases

1. **The model, the agent's tools and visibility**, in steps:
   1. Parameters and dependence, and wall and panel settings with their inheritance, kept across restarts, absolute UTC throughout.
   2. **Instances:** the server minting an instance for each effective context that differs from its panel's, its bounds, the side told each instance's context (`Intent::context()`) and republishing its content under the instance's id, again when its side starts again, and a side never able to mint or forge one. Every later step stands on this one.
   3. Content declared for a window and the rule for fixed time, with no data said for an environment without history.
   4. The agent's reading and its `set_wall`, `set_panel_context`, `copy_panel` and `create_layout` contexts.
   5. Read-only chips on walls and panels.

   The linked time cursor came first, needing none of the model beyond a chart's time axis.
2. **Gestures:** pulling off pins, the freeze toggle, and choosing a chip's value.
3. **Tangible tokens.**

## Rationale

In November 2026 the maintainer, in the headset, saw a spike on a downstream operations console's Charts wall, which shows charts for the region its bridge has selected. He wanted to pull the chart off, keep it, switch regions and look for correlated spikes elsewhere, but a chart pulled off kept following the selection. The environment's System wall already pins a panel dragged off it, on the move intent (`panel-actions.md`), and he asked for that as a general mechanism, with a badge saying what a panel is pinned to, a freeze that keeps exactly what is shown with absolute times, and a wall locked to one window so that charts from different regions line up. He then saw that walls could carry a context their panels inherit, made explicit rather than implicit, and asked for pin, freeze and the time-locked wall to be its special cases. He also asked that charts sharing a time share the pointer's time, and that the agent, rather than he, mostly coordinate the arrangement, while everyone can always see what a wall or panel shows. A tangible metaphor of time tokens plugged into walls was asked for as a concept only.

The model holds up because it adds only parameter declaration and the inheritance rule to what pin and freeze need anyway. It turns the time-locked wall and the agent's layouts into settings rather than mechanisms of their own, and gives tokens a meaning in advance. Instances are unavoidable, since one id shows one context. Tomori mints their ids so that a side cannot fake a participant's pin. Time is the one parameter Tomori interprets, because it rewrites axes, links cursors and enforces a freeze; every other parameter is the environment's.

## Scope

When built, phase 1 would cover: `tomori-environment`'s `Parameter`, `Space::set_context`, `Panel::depends_on` and `for_window`, and `Intent::context`; `tomori-protocol`'s context of a `Wall` and a `Panel`; `tomori-server`'s stage (settings, instances, the rule for fixed time, absolute labels), hub (telling the side), `mcp.rs` and the agent's tools; and `tomori-scene`'s chips. The linked time cursor's scope is in `projection.md`.

## Validation

When built: tests of `tomori-environment` that a context is an intent apart from any payload, never a client's; of `tomori-server`'s stage that a panel's effective context is its own over its wall's, that a pull-off by a participant makes an instance and refills the slot while the agent's move does not, that a side cannot publish to an id it was not given or fix a parameter, that a fixed time keeps what is shown and takes only content for its window, that labels are written in UTC clock times, that bounds refuse a ninth instance, and that settings and frozen instances survive a restart; of its hub that the side is told each instance's context, again as it starts; through the whole server, that the agent's tools set and read contexts under strict arguments; of `tomori-scene` that chips show for walls and panels with settings of their own and none otherwise, and a time chip toggles freeze; and a render on lavapipe of a wall's chip strip and a frozen chart in clock times.

## Open questions

- Whether a parameter's choices should be offered by a chip's chooser in phase 2, or only by the agent.
- Whether a wall's `time` setting should apply to panels that do not depend on time, as a list of events in that window.
- Whether instances should outlive their side's absence, shown frozen, or rest until it runs again.
- Whether a live window shorter or longer than the side publishes, such as `live 60 min` on a 15-minute chart, should be asked of the side, as a fixed window is.
