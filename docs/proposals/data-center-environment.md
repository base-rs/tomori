# Proposal: A Data Center Environment for Running Code

Status: **a proposal for discussion**, November 2026. Nothing here is a requirement. It is the companion of [A GitHub Environment, and Understanding Code in the Space](github-and-code-environments.md): GitHub is about working on the code; the data center is about **the running code**. The first crates.io release (#73) comes first.

It builds on the environment framework proposal (PR #90, `context/environments.md`), in particular data sources (#83), state synchronization (#76), media streams (#84), anchors (#79), shared attention (#81), agent projection (#82) and Tomori's agent tools (#100).

## Issues

| Issue | What |
|---|---|
| #156 | The data center environment: per-instance views of running systems from Prometheus |
| #157 | A live process data source: metrics pulled straight from running processes |
| #158 | The agent's investigations: data sources as agent tools, and findings in the space |
| #159 | Cross-environment hand-off: moving a space to another environment with context |
| #83 | Data sources and the Prometheus source, extended with per-instance needs (comment) |

## The discussion

Spoken by the maintainer from the headset, November 2026.

- **A proof of concept exists**, in a different app, which the agent doesn't have access to yet but will later. It hooks into **Prometheus** to visualise metrics as a **virtual data center**. You can drill into specific **Kubernetes pods** and see the application's instrumentation inside: how many requests it is processing, of what kind, and how much concurrency is used across HTTP, database, gRPC, Redis and similar, all captured as metrics in Prometheus. (`design/environments.md` already names Fantail Observe, "a separate project that shows a running system's topology and telemetry spatially", as the environment to connect eventually; whether that is this proof of concept is for the maintainer to confirm.)
- **A fast path.** The maintainer is working on a fast path where the server connects directly to a given running process and pulls metrics in real time. So data comes not only from Prometheus, which is an aggregate, but live from the running process.
- **Its role in Tomori.** GitHub is about working on the code. This environment, called "Prometheus" or better **"data center"**, is about **the running code**: seeing what is going on, and using **the human visual cortex for pattern matching and recognition**.
- **A motivating example.** Six servers ran new code behind a load balancer. Two were at 200% utilisation and four at 0%, while Prometheus's graph showed an average of about 80 to 90%. Only because each server was visualised **independently** did the maintainer spot that load balancing was broken. Aggregates hide this; per-instance spatial visualisation reveals it.
- **The agent in the loop.** The user points at a server and asks "why is this server overloaded?". The agent investigates through Prometheus and MCP servers that give it access to those systems, then comes back: "I found why. Would you like me to make a pull request?" Then **the space jumps to the GitHub environment** to do that work.

## Recommendation in brief

- **`environment-datacenter`** is an environment crate (#75) whose server half reads metrics through data sources and keeps a **topology** (data center, nodes, pods, processes, subsystems) with **per-instance series** as environment state (#76). Its client half draws every instance individually.
- **Two kinds of data source,** both on the server:
  - **Prometheus** (#83): history, topology from labels, and the aggregate view, at its scrape cadence.
  - **Live processes** (#157): a few chosen processes pulled directly, a second or faster, only while someone looks at them.
- **Per instance, never only aggregate.** Siblings share one scale, outliers are made obvious, and the aggregate is shown as a reference against the instances, never instead of them.
- **Pointing reaches the agent** (#81): every node, pod, process and subsystem is a resource. The agent investigates with Prometheus and its own MCP servers, and projects its **findings** back into the space as resources with their evidence (#158).
- **"Make a pull request" is a hand-off** (#159): an owner confirms moving the space to the GitHub environment, which receives the finding, the instance, the repository and revision it runs, and the code the agent identified. Coming back restores the data center as it was.
- **Visuals wait for the proof of concept.** Everything about how the data center looks is pending the maintainer sharing it; the principles below are what any design must satisfy.

## Mapping onto the framework

| Need | Framework part | What the data center adds |
|---|---|---|
| Prometheus queries with credentials kept on the server | Data sources, #83 | Per-instance queries grouped `by (instance)` or `by (pod)`; topology from labels; label filtering before anything reaches a client |
| Live per-process metrics | A new data source, #157 | Direct pulls from processes, allow-listed, rate-limited, only while interested |
| Topology and metrics on every headset | State synchronization, #76 | Topology as collections; current values as values; recent history as bounded series; detail sent only to interested clients |
| High-rate visuals | #76 series, #77 animation, and possibly #84 | Rates and concurrency as declarative animation (flows, pulses) driven by values; media streams only if a proof-of-concept view needs them |
| Where the agent, findings and charts stand | Anchors, #79 | The agent near the fleet; findings at `presentation`; charts at `stage` |
| "Why is this server overloaded?" while pointing | Shared attention, #81 | Instances as resources, described with their labels, current values and time window |
| The agent's charts, highlights and findings | Agent projection, #82 | Findings as resources with evidence; charts bound to queries (#83) |
| The agent querying the same systems | Agent tools, #100 and #82 | Data sources offered as tools, answering #83's open question (#158) |
| Moving to GitHub to make a pull request | Changing environment, #74 | A hand-off carrying context, and state kept per environment (#159) |

## The data sources

### Prometheus (#83)

- **Instances, not averages.** Queries keep the instance: `sum by (pod) (rate(http_server_requests_total[1m]))` rather than `sum(...)`. The aggregate is computed alongside, as a reference.
- **Topology from labels.** Nodes, namespaces, pods, containers and jobs come from the labels Prometheus already has (`node`, `namespace`, `pod`, `instance`, `job`), and from kube-state-metrics where present. No Kubernetes access is needed for a first version.
- **Subsystems** (HTTP, database, gRPC, Redis and so on) come from the application's own metric names and labels. The mapping from metrics to subsystems is configuration, since applications name them differently; the proof of concept will show what it uses.
- **Labels are filtered on the server.** Only configured labels reach clients and the agent, since labels can carry URLs, user identifiers or secrets.

### The live process source, the fast path (#157)

- **What it is:** a data source that connects to a chosen running process and pulls its metrics directly, a second or faster, instead of waiting for Prometheus's scrape and query. The protocol is what the maintainer is building; until it is shared, assume a process's own metrics endpoint (Prometheus or OpenMetrics text) as a stand-in.
- **Discovery.** Targets come from configuration or from Prometheus's own target list (`/api/v1/targets`), filtered by an allow-list (namespaces, jobs, label selectors). **Neither a client nor the agent ever names an address:** they name a resource identity, which the server resolves to a target it already allows. This keeps the server from being used to reach arbitrary hosts.
- **Network reach.** The server must reach the processes: running in the cluster, through a port-forward, or through an authenticated proxy. Which one is configuration, not a client choice.
- **Credentials** (bearer tokens, mutual TLS, a kubeconfig) stay in server configuration, as for every data source (#83). Clients receive values only.
- **Rate limits.**
  - Pull only while someone is interested (#76 interest), shared among every viewer, and stop when interest lapses.
  - A floor on the interval per target (say 250 ms), a cap on concurrent targets per space and per server, and a cap on series per target, since a process can expose thousands.
  - Back off on errors and time-outs; a slow process is shown as stale, never retried in a tight loop.
  - Being pulled must cost the process little: bounded response sizes, and no pulls the process hasn't been configured to allow.
- **Freshness.** Every value carries its time and source, so that a view can tell live values from Prometheus's, and grey out stale ones.

## Visual design principles

Specific visuals are **pending review of the proof of concept**. Whatever it looks like, it should hold to these:

- **Per instance, never only aggregate.** Each server, pod and process is its own object. An aggregate is drawn as a reference among the instances (a line, a ghost) and never replaces them.
- **Siblings share one scale.** Six servers are drawn on the same scale, so that 200% and 0% look nothing alike. Utilisation over 100% visibly overflows its object rather than being clipped at full.
- **Outliers are obvious.** An instance far from its siblings' median stands out by more than one channel, such as height, colour and motion, so it is seen in peripheral vision before it is looked for. Colour is never the only channel, for those who can't tell the colours apart.
- **Stable places.** An instance keeps its place as values change, and a new one appears beside its siblings, so that spatial memory works and a change of shape means a change in the system.
- **Drill down,** by pointing and choosing, or by walking closer (#77's distance detail): data center, node, pod, process, subsystem. Each level keeps its parent in view, so you know where you are.
- **Rates and concurrency as motion**: flows for requests, filled slots for concurrency, driven by values and animated on clients, so no per-frame data crosses the network.
- **Time in reach.** Each instance shows a short recent history (#76 series), and the agent can project longer charts (#82).
- **Calm when healthy.** A healthy data center is quiet; what moves or glows is what needs looking at.

## The agent's investigations

Tracked by #158.

- **Asking.** Pointing at a server and asking "why is this server overloaded?" delivers its identity and description (#81): its labels, current values, siblings' median and the time window.
- **Investigating.**
  - Through Tomori's data sources offered as tools: Prometheus queries, bounded, with the same label filtering, so the agent needs no credentials of its own. This answers #83's open question in favour of offering them, with the agent's own MCP servers still allowed.
  - Through the agent's own MCP servers for other systems, such as Kubernetes (events, logs, rollouts), tracing and the code.
- **Reporting back.** A **finding** is a card the agent places (#82) at the `presentation` anchor: a summary, the resources it concerns (highlighted in the space), and its evidence as charts bound to queries, so anyone can check the claim. It is a resource itself, so pointing at it and asking "how sure are you?" works.
- **Offering work.** "I found why. Would you like me to make a pull request?" is an offer the agent makes in speech, with a [hand-off](#cross-environment-hand-off) attached to the finding. Accepting it is a participant's choice.

## Cross-environment hand-off

A framework capability, not specific to these two environments (#159).

- **What moves:** the space changes environment, as an owner can today (`design/environments.md`, #74). The conversation, queue, workers and agent session continue; the agent is given the new environment's instructions.
- **What it carries:** a **hand-off record**, typed and bounded:
  - a note: what was found and what to do ("open a pull request fixing the load balancer's hashing");
  - resource identities from the old environment (the server, the finding);
  - where the code is: repository and the revision the instance runs, from the image's OCI labels (`org.opencontainers.image.source`, `org.opencontainers.image.revision`) where present, and files and lines the agent identified.

  The new environment's server half receives it and presents it: the GitHub environment shows the finding at its `presentation` anchor, opens code panels at the lines, and the pull request the agent opens arrives as a card in the inbox.
- **Who decides.** The agent proposes a hand-off through a tool; changing the environment is an owner's choice, so an owner confirms with **A**, as `design/interaction.md` requires. Everyone in the space moves together.
- **Coming back.** Each environment's state is kept per space while another is shown, so "take me back to the data center" restores it as it was, with the finding still there. This needs #76's persistence hook to be per environment.
- **Both ways.** From GitHub, "show me this running" hands off to the data center with the pull request's revision, to watch a deploy.

## Experiments

Ordered by value and feasibility.

| # | Experiment | Value | Feasibility | Effort |
|---|---|---|---|---|
| 1 | Per-instance utilisation from Prometheus | High: the motivating example | Needs #75, #76, #83 | Medium |
| 2 | The load-balancer imbalance, as an acceptance test | High: proves the principle | A synthetic fleet; builds on 1 | Small |
| 3 | "Why is this server overloaded?" | High | Needs #81, #82 and data-source tools | Medium |
| 4 | Drill down to pod and subsystem | Medium to high | Builds on 1; mapping from the proof of concept | Medium |
| 5 | Live process fast path | Medium: depends on the protocol | Needs the maintainer's protocol | Medium |
| 6 | Hand-off to GitHub | High, once both exist | Needs the GitHub inbox | Medium |

### 1. Per-instance utilisation from Prometheus (#156)

- **Hypothesis:** a per-instance spatial view reveals imbalances that the usual aggregate graph hides, at a glance.
- **Build:** the server half queries one utilisation metric per instance from a configured Prometheus, every few seconds; each instance is an object on a shared scale, with the aggregate as a reference line; a short series each. No drill-down, no fast path.
- **Evaluate:** experiment 2.
- **Depends on:** #75, #76, #83, #79, #77.
- **Effort:** medium, mostly the framework.

### 2. The load-balancer imbalance, as an acceptance test (#156)

- **Hypothesis:** someone who doesn't know the setup spots broken load balancing in seconds, unprompted.
- **Build:** a synthetic fleet of six instances behind a deliberately broken balancer (or a fake Prometheus replaying the incident): two at 200%, four at 0%, while the average reads high. Shown beside a conventional aggregate graph of the same data.
- **Evaluate:** time to notice, unprompted, on the headset; whether the aggregate graph alone leads anyone to it. Keep the fixture as a regression test for every later visual design.
- **Depends on:** experiment 1.
- **Effort:** small.

### 3. "Why is this server overloaded?" (#158)

- **Hypothesis:** pointing and asking leads the agent to the cause faster than describing the server, and a finding with evidence is trusted more than a spoken answer.
- **Build:** instances as resources (#81); Prometheus as a bounded agent tool; the finding card with a bound chart (#82).
- **Evaluate:** with the fixture from 2, ask the question while pointing at an overloaded instance, then at an idle one. Does the agent find the imbalance, and say so with evidence?
- **Depends on:** experiments 1 and 2, #81, #82, #83, and the tool transport shared with #100.
- **Effort:** medium.

### 4. Drill down (#156)

- **Hypothesis:** going from the fleet into a pod's subsystems (HTTP, database, gRPC, Redis) shows where the time and concurrency go, which no single graph does.
- **Build:** pods inside nodes; subsystems inside a pod, with requests and concurrency from the application's metrics, mapped by configuration. **Pending review of the proof of concept**, which already does this.
- **Evaluate:** against a real service, find the subsystem behind a latency rise, on the headset and in Grafana.
- **Depends on:** experiment 1, and the proof of concept.
- **Effort:** medium.

### 5. The live fast path (#157)

- **Hypothesis:** seeing a process at a second or faster shows bursts and stalls that a 15- to 60-second Prometheus scrape averages away.
- **Build:** the process source against one allow-listed process, pulled only while someone looks at it, shown beside its Prometheus values with freshness marked.
- **Evaluate:** load a process in bursts; are the bursts visible live and invisible in Prometheus's view? Measure the process's cost of being pulled, and the server's at the target limits.
- **Depends on:** the maintainer's fast-path protocol, #83's data-source configuration, #76 interest.
- **Effort:** medium.

### 6. Hand-off to GitHub (#159)

- **Hypothesis:** moving from "I found why" to the code, with the context carried, is one step instead of a context switch to a laptop.
- **Build:** the hand-off record, the agent's tool to propose it, an owner's confirmation, state kept per environment, and the GitHub environment presenting the finding.
- **Evaluate:** from experiment 3's finding, accept "make a pull request", watch the agent open it in the GitHub environment, and return to the data center to find it as it was.
- **Depends on:** experiment 3, the GitHub inbox (#148), #74, #76's persistence.
- **Effort:** medium.

### Recommended first

1. **Per-instance utilisation from Prometheus**, with **the load-balancer imbalance as its acceptance test** from the start. It is the maintainer's own example, and it proves the central principle before any investment in visuals.
2. **"Why is this server overloaded?"**, which ties the data center to the agent and settles how data sources become agent tools.
3. **Review the proof of concept** before drill-down or visual design, and port what it already does well.

## The proof of concept

The maintainer's proof of concept already visualises Prometheus metrics as a virtual data center, with drill-down into Kubernetes pods and their instrumentation. The agent doesn't have access to it yet. Once it is shared:

- review its visuals, drill-down, metric mapping and interaction, and what worked and didn't on the headset;
- port what fits into `environment-datacenter`'s client half, using Tomori's renderer (#77) and UI kit (#80), and its data handling into the server half;
- revise this proposal's [visual design principles](#visual-design-principles) and experiment 4 accordingly.

Until then, the visual parts here are deliberately unspecified.

## Open questions

- **Is the proof of concept Fantail Observe,** and is it to be ported into Tomori, or connected to it as a separate environment crate built through the builders (`design/builders.md`)?
- **The fast path's protocol,** and whether processes push or the server pulls.
- **Topology:** are Prometheus labels enough, or does the server half need the Kubernetes API (with its own credentials)?
- **Which metrics mean "utilisation"** for an application, and is the mapping per application, per space or per server?
- **Visitors:** do they see production topology and metrics? By default, probably not, as for shared windows (#85).
- **Agent tools:** does offering data sources as tools make the agent's own Prometheus MCP server unnecessary, or do both stay?
- **Hand-off:** can several environments be visible at once (a portal to GitHub beside the data center), or is moving the space enough?
- **Alerts:** may the server half raise attention (`docs/proposals/attention.md`) when an outlier appears while nobody is looking, which is #75's open question about a server half putting input to the agent?

## Changes to existing documents

None yet. If experiments become requirements:

- `design/environments.md`: the data center environment, hand-off, and state kept per environment.
- `design/security.md`: the live process source's allow-list, network reach and credentials, and production data for visitors.
- `design/spaces.md`: the hand-off record and per-environment state in the space's record.
