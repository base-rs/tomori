# Proposal: Panel Refresh Policies, Live, Every N Minutes, On View or Manual

Status: **proposed**, November 2026, for the maintainer's review. Nothing here is a requirement until it is brought into the design documents it changes (listed at the end), which take precedence.

It builds on the panel context model (`design/panel-context.md`): parameters a side declares, settings on walls and panels that are *follows* or *fixed*, a panel inheriting its wall's setting, chips (#415), instances (#420), the rule for fixed time (#473), and pin and freeze by hand (#485). It also builds on the panels an environment publishes and their pacing, the 128-panel bound and far panels drawn as titles (`design/projection.md`, #494, #506), proximity zones (`design/environments.md`, #371), environment actions (`design/environment-actions.md`) and the cached Prometheus client (`design/data-sources.md`, #402). It was written against `origin/main` at `59304e0d` (#506).

## The request

From the maintainer, November 2026, about a downstream environment (Shopify's internal Observe) whose agent query charts poll Prometheus:

> Today every environment panel is live. Observe caps its agent charts at 32 for cost. I'd like more panels than that, as long as not all of them are live, with refresh controls like Observe's web UI: auto refresh, refresh every 1, 5 or 15 minutes, or off with a manual refresh button. Maybe this needs a more interactive model for panels, or ties into walls.

## Recommendation in brief

- **A refresh policy for each panel**, which is one of `live` (the side's own cadence, as today), `every N min` (N is 1, 5, 15, 30 or 60), `on view` (live while someone is within reading distance of the panel, otherwise held), or `manual` (only when someone asks). "Paused" and "off" are both `manual`.
- **Modelled beside the context, not as part of it.** Refresh is a setting that a wall and a panel each hold, inherited exactly as a context parameter is: the panel's own setting, else its wall's, else the default its side declares for the panel, else `live`. It is told to the side in the same `Intent::context()`. But it is **not a parameter**, because it changes *when* a panel is refreshed and not *what* the panel shows. So it never makes an instance, never counts towards a panel's three parameters or four chips, and is never part of an instance's identity.
- **The server is the scheduler.** For a non-live panel, the server sends its side `Intent::refresh` when the panel is due: at each `every N` tick (staggered), when someone presses refresh, and when someone comes into view of an `on view` panel. The side's one new rule is: **fetch a non-live panel only when told to**. The server holds back updates to a non-live panel outside the window after a refresh it asked for, keeping the latest to make at the next refresh. The policy therefore stays true on the wire even for a side that ignores it.
- **One chip:** `↻ 3 min ago · every 5 min`, on the panel's chrome after its context chips. Pointing at its first half and pressing it refreshes now. Pressing its second half cycles the policy. A wall shows its own refresh setting in its chip strip, and the agent sets it.
- **Separate budgets.** At most **32 live panels** (`live`, or `on view` while viewed) for an environment in a space, and the 128 panels in all stay as they are. Once the live budget is full, a new panel takes its side's declared overflow policy (Observe's is `every 5 min`), or `manual` if the side declares none. Explicitly asking for a 33rd live panel is refused, and the message says why.
- **Frozen and fixed-window panels have no refresh.** Their window is fixed, so they are filled once. Their chip shows no policy. Pressing refresh on one only asks again for its window when it is waiting or has no data.
- **Smallest useful step:** the policy setting with its inheritance, a side's default and overflow policy, server-scheduled `every N` and `manual`, `Intent::refresh`, a read-only chip, the live budget, and the agent's `set_refresh` and `refresh_panel`. The pressable chip, `on view` and Observe raising its chart cap come after.

## What the system has today

- **Every published panel is live.** A side publishes and updates panels whenever it likes, within the pacing in `projection.md`: about one change a second per panel, 128 panels, and a page of 64 at once. Tomori does not know how often a side polls its source, and nothing tells a side that a panel can rest.
- **The context model** gives each panel an effective binding for every parameter it depends on, plus `time` (`Follows`, `Live { minutes }` or `Window { from_ms, to_ms }`). The binding comes from the panel's own setting, else its wall's, else following. The server makes `Intent::context()` and gives it to the side whenever the binding changes. Settings are kept with the space. Chips show them. Instances exist because one panel id shows one context.
- **Freeze** fixes a copy's time to the window its chart shows. The rule for fixed time then accepts only content declared for that window, so a frozen chart never changes and costs nothing. In effect it is already `manual` with nothing to refresh.
- **Proximity zones** (#371) let a side poll quickly while someone stands at a place it declares and slowly while nobody does. But a zone is about a fixed place. An agent's chart moved wherever the asker looks has no zone, so Observe paces its charts only by whether anyone is in the space at all ("the proximity zones do not apply, since a chart is wherever the asker took it").
- **Far panels are drawn as titles** (#506). The client already knows when a panel is too small to read, but the server and the side do not.
- **Observe's agent charts** (its PR #92 and #101): each `chart` action makes a live panel. Its series are read through the source's one cached client as background range reads, every 30 s (`REFRESH`, the scrape interval) while anyone is in the space and every minute while nobody is. Fixed windows are read once. The charts are capped at 32 (`MOST_CHARTS`), with the 33rd replacing the oldest, on Notices' four slots and two overflow walls of 15. Observe's other walls (Charts, the terminals, Compare) come from the summary poll, which is paced by its zones (#71) and shared through the cache's 15 s buckets (#402).

## 1. The refresh policy

### Policies

| Policy | Meaning | Who refreshes | Counts as live |
| --- | --- | --- | --- |
| `live` | The side's own cadence, as every panel today | the side, freely | yes |
| `every N min`, N ∈ {1, 5, 15, 30, 60} | Refreshed every N minutes, on a schedule the server staggers | the server's `Intent::refresh` | no |
| `on view` | Live while any participant is within reading distance of it and it is drawn whole. Otherwise held, refreshed once on coming into view | the side while viewed, otherwise the server's intent | only while viewed |
| `manual` | Only when someone asks, whether a participant's press, the agent, or the first fill | the server's `Intent::refresh` | no |

A press of refresh on any panel refreshes it once now, whatever its policy. On a `live` panel it asks the side for an immediate read, which is useful after a slow poll.

`on view` is the attention-driven case: it is #371's idea applied to a panel, which moves, rather than to a place. The server knows where every panel is and where every participant's head is, so it decides "viewed" itself. A panel is viewed when a participant who is present and not stale is within the reading distance that `projection.md` uses for its slot scale, and the panel is drawn whole rather than as a title (the 0.09° a row of #506, computed by the server from the same geometry). The 10 s grace that zones keep after someone leaves applies here too. The side is told in the intent whether the panel is viewed, so it never has to work this out.

### Where it lives: three options compared

| | A. A context parameter `refresh`, beside `time` | **B. A setting inherited like a parameter, beside the context (recommended)** | C. Wall-level only |
| --- | --- | --- | --- |
| Inheritance from its wall | yes | yes, the same rule | yes, nothing else |
| A panel set apart from its wall | yes | yes | no: to set it apart you must move it to another wall |
| Makes instances | **yes**: a different value is a different context, so the same chart refreshed differently becomes a copy | no | no |
| Chip budget (4) and parameter budget (3) | uses one of each | its own chip, outside both | wall chip only |
| A side declares it | a fourth "parameter" that every side must understand, with Tomori reserving the name | `Panel::refresh(default)` and an overflow policy, both optional | nothing per panel |
| Agent tools | `set_context` with `parameter: refresh` | `set_refresh` and `refresh_panel` | `set_refresh` on walls only |
| Fit with freeze | odd: a frozen panel would still carry a `refresh` binding | natural: fixed time means no refresh, so its chip is hidden | n/a |
| Fit with Observe | needless instances when the agent slows one chart | each chart separately; the overflow walls slowed as a whole | cannot keep the newest charts live on a wall of older slow ones |

**Recommendation: B.** Refresh has the *shape* of a context setting (wall, then panel, then default, kept with the space, shown as a chip) but not its *meaning*. The context decides what a panel shows, which is why a difference in context is an instance. Refresh decides how fresh it is kept, so making instances for it would multiply panels for no reason. B reuses the storage, the inheritance, the telling and the chip drawing of the context model. It adds no parameter and leaves the context's bounds and its instance rule untouched. C is too coarse for the case that prompted this, where a wall's newest charts stay live while older ones slow down. Its strength, that one setting slows a whole wall, is kept in B through wall-level settings.

### Inheritance and defaults

The effective policy of a panel is:

1. the panel's own setting, made by a participant or the agent; otherwise
2. its wall's setting; otherwise
3. the default its side declared for the panel (`Panel::refresh(Refresh::every(5))`); otherwise
4. `live`, which is today's behaviour, so a side that declares nothing sees nothing change.

A wall's setting overrides the side's per-panel default because a wall setting is a participant's or the agent's explicit choice. Settings are kept with the space and rest with a panel, as context settings do. A panel moved to another wall keeps its own setting and takes the new wall's in place of the old one's.

**Instances.** A copy made by `copy_panel` or a pin takes the source panel's *own* refresh setting, if any. It then inherits from the wall it lands on, or floats with the side's default. A **frozen** copy, and any panel whose effective time is a fixed window, has no effective refresh. Its window's content is filled once by the rule for fixed time. Its chip shows no policy, and a press of refresh asks for its window again only while it is waiting or showing "No data for this window". This gives an honest way to retry a partial answer. Unfreezing restores the setting it had.

**A live window refreshed slowly.** A chart whose time is `Live { 60 min }` and whose policy is `every 15 min` shows the 60 minutes up to its last refresh. Its "now" label would then mislead by up to 15 minutes. The chip's age says how old it is. In addition, a side publishing a non-live chart should label its last axis point with the clock time (`02:35 UTC`) rather than `now`. Whether the server should enforce this, as it labels fixed windows, is an open question.

## 2. Interaction

### The chip

A panel published by an environment whose effective policy is anything but `live` with no setting shows a **refresh chip** after its context chips, with the same backing and tones as `projection/chips.rs`:

```
 p95 queuing, us-central1
 [Region · US-CENTRAL1] [Time · live 60 min] [↻ 3 min ago | every 5 min]
 ─────────────────────────────────────────────────────────────────────
     ╭╮        ╭─╮
 ────╯╰────────╯ ╰───────╮╭────────────
                         ╰╯
 01:35                   02:05                             02:35 UTC
```

(A concept sketch. It was not rendered offscreen, because a cold build of `tomori-render` in a private target directory is not cheap. Phase 1's render test would produce the picture.)

- **Its two halves.** `↻ 3 min ago` is the age: time since the side last answered a refresh, counted by the client from the server's instant as a text panel's age is. `every 5 min`, `on view`, `manual` or `live` is the policy. A policy the panel sets itself is drawn *fixed*, one inherited from its wall *inherited*, and a side's default drawn as following, matching the context chips' three looks.
- **States.** While a refresh is outstanding the age reads `↻ refreshing…`. If the side has not answered within 30 s it reads `↻ no answer · 4 min ago`, in the warning tone. An `on view` panel being viewed reads `↻ live · on view`. A `live` panel shows no chip unless it has a setting of its own, so a wall of live panels looks exactly as it does today.
- **Pressing.** The chip gives two presses through `ContextChip::press`, the way the time chip on a copy gives freeze. The age half gives `ChipPress::Refresh`, and the policy half gives `ChipPress::Cycle`, which steps through `live → every 1 → every 5 → every 15 → manual → live`. (`every 30`, `every 60` and `on view` are left to the agent and a later chooser, keeping the cycle short.) Pointing at either half with either ray or the desktop's mouse, the ray's label says `A  refresh now` or `A  every 15 min`, the next policy, and A, Enter, the trigger or a click presses it, sending `ClientMessage::RefreshPanel { panel }` or `ClientMessage::SetRefresh { panel, policy, revision }`. The latter is refused if the panel changed since the `revision` seen, as `FreezePanel` is. A visitor is told they only watch. A press the client does not know does nothing, and a client that does not know the chip draws nothing.
- **Pointing while speaking.** "Refresh this" or "make this one manual" while pointing at the chip names it, as #415 names chips: `; pointing at its chip: "↻ 3 min ago · every 5 min", its own setting`.
- **Walls.** A wall holding a refresh setting shows it in its chip strip, `↻ every 5 min`. Wall chips are not pressable yet, as no chip of a wall is (`panel-context.md`), so a wall's policy is set by the agent in the first step. A wall's chip with *refresh all* and *cycle* is phase 2, together with pressing any wall chip.

### The agent

- **`set_refresh`** `{ wall | panel, policy: "live" | "every" | "on_view" | "manual", minutes?, follow? }`. `minutes` is required for `every` and must be one of 1, 5, 15, 30 or 60. `follow: true` clears the setting. Malformed combinations, a wall or panel that is not found, the Tomori wall, the agent's own panels and a live budget that is already full are each refused, saying why. A change is announced as the agent's shared changes are (`mcp.md`): a notice of kind `refresh` on the status wall replacing the last one, such as `The agent set the wall "Charts overflow 1" to refresh every 5 min.` It is logged at info.
- **`refresh_panel`** `{ panel | wall }` refreshes one panel now, or every non-live panel on a wall. A wall counts as one request per panel against the rate limits below. It is not announced, since refreshing changes no shared setting, but the result says how many panels were asked and how many were refused for rate.
- **`contexts`** and **`panels`** list each panel's effective `refresh` (`policy`, `minutes`, `setting`: `own`, `inherited`, `default` or `live`), its `updated` instant in UTC, and whether it is `viewed`. `contexts` also lists each wall's setting.

### Notices

The status wall says only what changes shared state or needs a decision:

- The agent's `set_refresh`, as above.
- The live budget being reached, once each time it fills: `32 panels are live; "Latency, eu-west1" will refresh every 5 min. Ask to make another manual to keep it live.`
- A refused live request from a participant goes to their own ray label, not the status wall.

## 3. The environment protocol

### What a side declares

- `Panel::refresh(Refresh)`: the panel's default policy. If absent, the default is `live`.
- `Space::set_overflow_refresh(Refresh)`: the policy given to a newly published panel that would otherwise be live once the live budget is full. If absent, the overflow policy is `manual`.
- `Outlet::refreshed(panel)`: an acknowledgement that the side answered a refresh with content unchanged, so the age resets though nothing was published. Publishing the panel also counts as an answer.

### What a side is told

- **`Intent::context()`'s `Context` gains `refresh()`**, a `RefreshBinding` with the effective policy and, for `on view`, whether the panel is viewed now. It is told at the same moments as bindings (a setting changes, the panel moves to another wall, the side starts again), plus whenever an `on view` panel becomes viewed or stops being viewed. A side bound anew is told every panel whose policy is not `live`. A side that ignores it is unaffected apart from the holding below.
- **`Intent::refresh(panel, reason)`**, made only by the server, never from a client's message. `reason` is `Scheduled`, `Requested { participant }` (0 for the agent) or `Viewed`. It asks the side to fetch the panel now and publish it, or call `refreshed`, within 30 s.

### What a side must do

1. **Live panels:** as today.
2. **Non-live panels:** fetch only on `Intent::refresh`, and answer within 30 s. Stop any timers you keep for them. A side with a shared poll, such as Observe's summary, which feeds many panels at no extra cost, may keep publishing them. The server holds those updates, as below, so declaring them `live` is the honest choice and costs nothing.
3. **On view:** poll at the side's live cadence while `viewed` is true, otherwise as non-live.
4. **Fixed windows (frozen or wall-fixed):** as the rule for fixed time already requires. `Intent::refresh` for one arrives only after a press while it is waiting or has no data, and asks for the window again.

### What the server enforces

- **Holding.** An update to a non-live panel is applied only within 45 s of a refresh the server asked for (the same 45 s the rule for fixed time waits). Outside that window it is kept as the waiting update for that panel, replacing any earlier one, as `projection.md` already keeps one per panel. It is made at the next refresh, or dropped if the side answers that refresh with a newer one. The policy therefore holds on the wire whatever a side does. An older side that publishes every second shows its panels at the policy's cadence, and their age is honest.
- **Scheduling.** `every N` ticks are staggered by a hash of the panel's id over the period, so 96 charts at 5 minutes become about one request every 3 s rather than a burst. A tick for a panel with a refresh still outstanding is skipped. The first tick after a policy changes to `every N` comes at once.
- **Rate limits.** A participant's `RefreshPanel` is accepted at most once per panel every 10 s, with a burst of 4 and 1 a second for each participant across panels; visitors are refused. The agent's `refresh_panel` is limited to 32 panels in a call and 64 a minute in a space. Across all reasons, a side is sent at most 4 `Intent::refresh` a second in a space with a burst of 32. The excess is queued oldest first, and a press past the limits is refused with the time until it may be pressed again. These sit inside the side's existing intent rates.

## 4. Limits and cost

- **Live budget: 32 panels for an environment in a space** (`stage::refresh::LIVE`). A panel is live if its policy is `live`, or `on view` while viewed. An instance counts like any panel. The total stays at **128** panels, of which at most 8 are instances. A space then has at most 32 panels that change freely and 96 more that change on a schedule or on request.
- **At the cap:**
  - A panel newly published as live past the budget takes the side's overflow policy.
  - A live setting asked for by a participant or the agent is refused with `32 panels are live already: set another to every N or manual first.`, naming the oldest-refreshed live panel as a candidate.
  - `on view` panels coming into view past the budget are viewed at `every 1 min` instead. The chip says `↻ every 1 min · on view (live budget full)`, and the side is told `viewed: false`.
  - A side's own default of `live` past the budget is treated as its overflow policy. The server never demotes a panel that is live already.
- **The cost model**, in what a refresh costs the side's source, per panel per hour:

  | Policy | Refreshes an hour | Observe chart of 4 series, 30 s step |
  | --- | --- | --- |
  | `live` (Observe: 30 s while anyone is in) | 120 | 480 range reads, each only the tail through the cache |
  | `every 1 min` | 60 | 240 |
  | `every 5 min` | 12 | 48 |
  | `every 15 min` | 4 | 16 |
  | `manual` | presses only | about 0 |
  | `on view` | 120 while viewed, 1 per approach | usually tens |

  32 live and 96 at `every 5 min` cost 32 × 120 + 96 × 12 = 4,992 refreshes an hour, against 3,840 for today's 32 live, which is 1.3× today's cost for 4× the panels. The wire cost is unchanged or lower: a non-live update is one `chart_append` or panel per refresh.
- **Client limits.** The headset reads 180 panels and draws far ones as titles (#506). 128 panels from one environment fit, as they already do.
- **Whether 128 should grow** is left as an open question. Non-live panels cost the server little but still fill slots and frames.

## 5. Observe's use

- **Charts the agent asks for** declare `Panel::refresh(Refresh::Live)` for the newest 16 and `every 5 min` beyond them. Observe's `MOST_CHARTS` rises from 32 to about **80**, within 128 minus its other panels and reserve (bridge 10, terminals 12, Compare 4, pins 6, copies 8 = 40, leaving 88). A chart the agent marks live past 16 is still within Tomori's 32. Observe declares `every 5 min` as its overflow policy. The instructions tell the agent the policies, that `set_refresh` keeps a chart someone is reading live, and that a slowly refreshed chart's age is on its chip. Overflow walls grow from two walls of 15 to as many as the charts need, drawn as titles when far.
- **The Charts wall, terminals and Compare stay `live`.** They are fed by the summary and the zones' pace (#71), not polled per panel, so slowing them saves nothing. They declare nothing, and so stay `live` under the default.
- **Combining with the cached client (#402).** A scheduled or pressed refresh reads through the same cache as a background range read aligned to the step, so it asks the gateway only for the steps since the last read, from a minute before. Charts that share expressions share requests. A press within the same 30 s step is answered from the cache, so the 10 s per-panel press limit costs nothing beyond it. The staggered ticks spread requests across Observe's background slots.
- **Combining with proximity (#71).** `on view` gives the agent's charts the attention-driven pace that zones could not, because a chart moves wherever the asker took it. It is Observe's natural default for charts beyond the live 16, once built. The zones keep pacing the summary, terminals and Compare. Observe's own idle pace, a minute while nobody is in the space, still applies to its live charts. A non-live chart's tick while nobody is in the space may be skipped by the side answering `refreshed`, so an empty space costs nothing.
- **Fixed-window charts** are already read once. With this proposal their chip offers a retry when the gateway answered partially, replacing the automatic re-read of a partial answer.

## 6. Phasing

1. **The policy and the server's schedule (smallest useful step).**
   - `Refresh` (`live`, `every N`, `manual`) as a setting on walls and panels, with the inheritance above, kept with the space, rested with panels.
   - `Panel::refresh`, the overflow policy, `Context::refresh`, `Intent::refresh` and `Outlet::refreshed`.
   - The server's staggered schedule, holding and rate limits, the live budget, and the agent's `set_refresh` and `refresh_panel`, with notices.
   - The refresh chip, read-only, drawn and named by pointing.
   - Observe adopting it for charts past 16.
2. **Pressing.** The chip's `Refresh` and `Cycle` presses on the headset and desktop, `RefreshPanel` and `SetRefresh`, and wall chips pressable (refresh all, cycle), which also gives `panel-context.md`'s wall chips a first press.
3. **`on view`.** The server's viewed test from presence and panel geometry, with grace, the budget while viewed, and Observe's charts defaulting to it.
4. **Later.** A chooser on the chip with every policy, raising 128 if non-live panels make it worthwhile, and a refresh token in the tangible tokens concept (`panel-context.md`).

### Risks

- **A stale panel read as current.** A slowly refreshed live chart's "now" label could be up to 15 minutes old. The chip's age mitigates this, and clock-time labels would too (see the open questions). An `every 15 min` chip must be legible from where its panel is read, which needs a headset check.
- **Older sides.** A side that ignores refresh keeps polling its source. The server's holding protects the wire and the chip stays honest, but the source is not protected. Only a side that implements the rule saves the cost the request is about.
- **Bursts.** Many presses, or a wall refresh on a slow source, could pile up behind the source's background slots. The rate limits and the side's own queues bound this, and `↻ no answer` makes it visible.
- **Complexity in the context code.** B reuses the context's storage and telling. The risk is that refresh leaks into the instance rule, so tests must show that a refresh difference never makes an instance.
- **`on view` flapping** at reading distance's edge. The grace and the margin that zones use apply.

## Open questions for the maintainer

1. Should a **participant's** policy change be announced on the status wall, as the agent's is, or only shown on the chip, as a participant's freeze is today?
2. Should the server **label a non-live live-window chart's axis** with clock times, as it labels fixed windows, rather than trusting the side's "now"?
3. Is the live budget **32 per environment in a space** right, or should it be per space, or set by the server's configuration (`tomori.live_panels`)?
4. Should **`on view`** mean within reading distance (proposed), within a participant's view cone, or pointed at? And should it count spectators?
5. Should a **pinned copy** default to `manual`, being a keepsake, rather than inheriting the source's policy?
6. With non-live panels cheap, should the **128 bound** rise for non-live panels, for example 32 live within 256?
7. Should the cycle on the chip include `on view` once it exists, and drop `every 1 min`?

## Design documents this would change

- `design/panel-context.md`: refresh as a setting beside the context, its inheritance, the chip, instances and freeze.
- `design/projection.md`: holding non-live updates, the live budget, the refresh chip on the wire, `RefreshPanel` and `SetRefresh`.
- `design/environments.md`: `Panel::refresh`, the overflow policy, `Intent::refresh`, `Outlet::refreshed` and the side's obligations.
- `design/mcp.md`: `set_refresh`, `refresh_panel`, and refresh in `contexts` and `panels`.
- `design/interaction.md` and `design/responders.md`: the chip's presses.
- `design/security.md`: rate limits by role for refresh requests.

## Validation, when built

- `tomori-environment`: `Refresh` bounds, `Context::refresh`, and `Intent::refresh` made only by the server.
- `tomori-server`'s stage:
  - inheritance (own over wall over default over live), kept across restarts, rested with panels;
  - a refresh difference never making an instance;
  - frozen and fixed-window panels having no refresh, with a press re-asking a waiting window;
  - holding updates outside a refresh's window and making the latest at the next refresh;
  - staggered ticks with 96 panels at 5 min spread within the period;
  - the live budget refusing a 33rd and giving new panels the overflow policy;
  - rate limits for participants, the agent and the side, with visitors refused.
- Through the hub: a side told its panels' policies, refreshed on schedule and on request, and an older side's per-second updates shown at the policy's cadence.
- `tests/mcp_refresh.rs`: `set_refresh` and `refresh_panel` refusals, notices, and `contexts` listing refresh.
- `projection::chips`: the chip's text, states and tones. `tomori-render` on lavapipe: `scene_graphics::tests::refresh`, a chart with `↻ 3 min ago · every 5 min` beside a live one (`TOMORI_REFRESH_PREVIEW` to see it).
- Downstream, Observe's tests: 80 charts within 128, the newest 16 live, the rest every 5 min, read through the cache only on refresh, and answered `refreshed` while nobody is in the space.
