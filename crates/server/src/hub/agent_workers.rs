// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! What Tomori's worker tools read and remove: the workers of the space bound to the calling session, as everyone in it sees them, and their removal, which only ever reaches the space's own workers. See `design/mcp.md`.
use std::collections::HashSet;

use serde_json::{Value, json};
use tomori_backend::{Backend, ErrorKind};
use tomori_protocol::{Worker, WorkerState};

use super::provenance::{Needs, Requester};
use super::{Hub, HubState};
use crate::space::worker_finished;

/// The most workers one call removes, those it names or finds; the rest are refused, to be asked for again.
pub const REMOVAL_LIMIT: usize = 64;

/// The kind of the notice of removed workers on the space's Tomori wall, of which only the latest is shown.
const NOTICE: &str = "workers";

/// The most characters of a worker's title the `workers` tool gives, on one line.
const TITLE_CHARS: usize = 120;

/// What the agent asked `remove_workers` to remove: the workers it names in `sessions`, every finished worker of the space with `finished`, and, with `force`, those still running, stopped first, which needs `requested_by`, the participant who asked, whom the server checks whenever they are named. See `design/security.md`.
#[derive(Debug, Default)]
pub(crate) struct Removal {
    pub sessions: Vec<String>,
    pub finished: bool,
    pub force: bool,
    pub requested_by: Option<String>,
}

/// A worker that left the space when it was removed.
#[derive(Debug, PartialEq)]
pub(crate) struct Gone {
    pub session: String,
    pub title: String,
    /// It had finished, as the space last saw it.
    pub finished: bool,
    /// It was running, as the space last saw it, and was stopped to remove it.
    pub stopped: bool,
    /// The worker whose removal took it, when it was one of that worker's own.
    pub with: Option<String>,
}

/// What removing workers did, for the tool's result: each worker removed, and each one refused or failed, with why.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    pub space: String,
    pub removed: Vec<Gone>,
    pub refused: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
    /// Who asked, as the server checked it, when the agent named them.
    pub requester: Option<Requester>,
}

/// One worker to remove, and its own workers, which go with it, the worker first.
struct Target {
    family: Vec<Worker>,
}

/// The removals decided before the backend is asked: which space, its backend and session, what to remove and what was refused.
struct Plan {
    space: String,
    backend: String,
    session: String,
    targets: Vec<Target>,
    outcome: Outcome,
    force: bool,
}

/// What a refusal to stop workers says, for the log.
fn asked(removal: &Removal) -> &'static str {
    if removal.force {
        "asked to remove workers with force"
    } else {
        "asked to remove workers"
    }
}

/// Whether a worker in `state` is still at work, so that removing it would stop it.
fn running(state: WorkerState) -> bool {
    matches!(state, WorkerState::Running | WorkerState::Waiting)
}

/// What is wrong with a worker, and the evidence, for the `workers` tool, at `now`, in milliseconds since the Unix epoch:
/// its kind, how many seconds it has held, why it failed, the tool it is running and for how long, and how its last step
/// ended. See `design/events.md`.
fn condition_json(condition: &crate::watch::Condition, now: u64) -> Value {
    let seconds = |since: u64| now.saturating_sub(since) / 1_000;
    json!({
        "kind": condition.kind.name(),
        "seconds": seconds(condition.since),
        "error": condition.error,
        "tool": condition.tool.as_ref().map(|tool| json!({
            "name": tool.name,
            "seconds": seconds(tool.since),
        })),
        "last_step_ended": condition.finish,
    })
}

/// How a worker in `state` is described to the agent.
fn state_name(state: WorkerState) -> &'static str {
    match state {
        WorkerState::Running => "running",
        WorkerState::Waiting => "waiting",
        WorkerState::Idle => "idle",
        WorkerState::Completed => "completed",
        WorkerState::Error => "failed",
        WorkerState::Stopped => "stopped",
        WorkerState::Stalled => "stalled",
        WorkerState::Unanswered => "unanswered",
        WorkerState::Unknown => "unknown",
    }
}

/// `worker` and its own workers in `workers`, at any depth, `worker` first, as the space knows them.
fn family(workers: &[Worker], worker: &Worker) -> Vec<Worker> {
    let mut family = vec![worker.clone()];
    let mut index = 0;
    while let Some(parent) = family.get(index).map(|member| member.id.clone()) {
        family.extend(
            workers
                .iter()
                .filter(|other| other.parent.as_deref() == Some(parent.as_str()))
                .filter(|other| !family.iter().any(|member| member.id == other.id))
                .cloned()
                .collect::<Vec<_>>(),
        );
        index += 1;
    }
    family
}

/// `count` workers, in words.
fn workers(count: usize) -> String {
    match count {
        1 => "1 worker".to_owned(),
        count => format!("{count} workers"),
    }
}

/// The notice of `removed` put on the space's Tomori wall, such as "Tomori removed 5 finished workers.", naming each, in the order removed, as many as a panel's body holds, [`crate::stage::BODY_LINES`] lines and [`crate::stage::BODY_CHARS`] characters, the rest counted, and who asked, `by`, when the server checked who did.
fn notice(removed: &[Gone], by: Option<&str>) -> String {
    let stopped = removed.iter().filter(|gone| gone.stopped).count();
    let what = if removed.iter().all(|gone| gone.finished) {
        format!(
            "{} finished {}",
            removed.len(),
            if removed.len() == 1 {
                "worker"
            } else {
                "workers"
            }
        )
    } else {
        workers(removed.len())
    };
    let mut said = match by {
        Some(by) => format!("Tomori removed {what}, as {by} asked."),
        None => format!("Tomori removed {what}."),
    };
    let stopped = match (stopped, removed.len()) {
        (0, _) => None,
        (1, 1) => Some("It was still running, and was stopped first.".to_owned()),
        (1, _) => Some("One was still running, and was stopped first.".to_owned()),
        (stopped, count) if stopped == count => {
            Some("They were still running, and were stopped first.".to_owned())
        }
        (stopped, _) => Some(format!(
            "{stopped} were still running, and were stopped first."
        )),
    };
    if let Some(stopped) = stopped {
        said = format!("{said} {stopped}");
    }
    // Every one is named, in the order removed, each worker before its own, as long as a panel's body holds them; only
    // beyond that are the rest counted, so that the panel scrolls rather than hides them.
    let all: Vec<String> = removed
        .iter()
        .map(|gone| format!("- {}", tomori_protocol::text::one_line(&gone.title, 80)))
        .collect();
    let mut lines = crate::stage::BODY_LINES - 2;
    let mut room = crate::stage::BODY_CHARS.saturating_sub(said.chars().count());
    let length = |line: &String| line.chars().count();
    let names = if all.len() <= lines && all.iter().map(length).sum::<usize>() <= room {
        all
    } else {
        let more = |count: usize| format!("- and {count} more");
        lines -= 1;
        room = room.saturating_sub(length(&more(all.len())));
        let mut names: Vec<String> = Vec::new();
        for name in all.iter() {
            if names.len() == lines || length(name) > room {
                break;
            }
            room -= length(name);
            names.push(name.clone());
        }
        names.push(more(all.len() - names.len()));
        names
    };
    format!("{said}\n\n{}", names.join("\n"))
}

impl Hub {
    /// The workers of the space bound to `session`, as everyone in it sees them, for the `workers` tool. See `design/mcp.md`.
    pub(crate) fn tool_workers(&self, session: &str) -> Result<Value, String> {
        let state = self.lock();
        let space = self.calling_space(&state, session)?;
        let record = state.space(&space).expect("found above");
        self.reports_workers(&record.model.backend)?;
        let now = crate::cards::unix_now();
        let now_ms = u64::try_from(self.wall_now().as_millisecond()).unwrap_or(0);
        let environments = self.environments();
        let seating = record.seated(&environments);
        let listed: Vec<Value> = record
            .model
            .workers()
            .iter()
            .map(|worker| {
                let finished = worker_finished(worker.state);
                // Where it stands, the anchor it was placed at or stands at with its parent, and who placed it.
                let at = seating
                    .seat_of(&crate::seating::Member::Worker(worker.id.clone()))
                    .map_or(tomori_protocol::cluster::HOME, |(at, _)| at);
                // Stalled or unanswered as the space shows it, with the evidence; see `design/events.md`.
                let shown = record.model.shown_worker(worker).state;
                let condition = record
                    .model
                    .worker_condition(&worker.id)
                    .map(|condition| condition_json(condition, now_ms));
                json!({
                    "session": worker.id,
                    // Written by whatever named the worker, so on one line and bounded; see `design/mcp.md`.
                    "title": crate::space::one_line(&worker.title, TITLE_CHARS),
                    "state": state_name(shown),
                    "condition": condition,
                    "running": running(worker.state),
                    "finished": finished,
                    "finished_seconds_ago": record
                        .model
                        .worker_finished_at(&worker.id)
                        .map(|at| now.saturating_sub(at)),
                    "parent": worker.parent,
                    "at": at,
                    "placed_by": record.placed_by(&worker.id),
                })
            })
            .collect();
        let (complete, reason) = record.model.workers_complete();
        Ok(json!({
            "space": space,
            "workers": listed,
            "complete": complete,
            "incomplete_reason": reason,
        }))
    }

    /// Whether the space's backend, `backend`, reports workers, or why there are none.
    fn reports_workers(&self, backend: &str) -> Result<(), String> {
        let reports = self
            .backends
            .get(backend)
            .is_some_and(|backend| backend.capabilities().workers);
        if reports {
            Ok(())
        } else {
            Err(format!(
                "This space's agent backend, {backend}, does not report workers, so the space has none."
            ))
        }
    }

    /// Decide what removing `removal` from the space bound to `session` would remove, refusing what it must not: the space's own session, any session bound to a space, anything that is not one of the space's workers, and, without `force`, a worker that is still running or has running workers of its own.
    fn plan_removal(
        &self,
        state: &HubState,
        session: &str,
        removal: Removal,
    ) -> Result<Plan, String> {
        let space = self.calling_space(state, session)?;
        let record = state.space(&space).expect("found above");
        let backend = record.model.backend.clone();
        self.reports_workers(&backend)?;
        if !self
            .backends
            .get(&backend)
            .is_some_and(|found| found.capabilities().delete)
        {
            return Err(format!(
                "This space's agent backend, {backend}, cannot remove workers."
            ));
        }
        if removal.sessions.is_empty() && !removal.finished {
            return Err(
                "Name the workers to remove in sessions, or set finished to remove every finished worker."
                    .to_owned(),
            );
        }
        // Stopping work in progress is on a participant's word, which the server checks rather than the agent; so is anyone the agent names. Removing finished workers is the agent's own tidying. See `design/security.md`.
        let requester = if removal.force || removal.requested_by.is_some() {
            let checked = self.requester(
                state,
                &space,
                removal.requested_by.as_deref(),
                Needs::Participant,
            );
            match checked {
                Ok(requester) => Some(requester),
                Err(reason) => {
                    let line = format!(
                        "The agent of space {space}, session {session}, {} and was refused: {reason}",
                        asked(&removal)
                    );
                    log::info!("{line}");
                    return Err(reason);
                }
            }
        } else {
            None
        };
        let shown = record.model.workers();
        let root = record.model.session.clone();
        // Every session any space is bound to, open or closed, whichever space.
        let bound: HashSet<&str> = state
            .spaces
            .iter()
            .map(|other| other.model.session.as_str())
            .collect();
        let mut asked = removal.sessions;
        if removal.finished {
            asked.extend(
                shown
                    .iter()
                    .filter(|worker| worker_finished(worker.state))
                    .map(|worker| worker.id.clone()),
            );
        }
        let mut seen = HashSet::new();
        asked.retain(|id| seen.insert(id.clone()));
        let mut outcome = Outcome {
            space: space.clone(),
            requester,
            ..Outcome::default()
        };
        let mut targets: Vec<Target> = Vec::new();
        for id in asked {
            let refuse = |reason: &str| (id.clone(), reason.to_owned());
            if id == root {
                outcome.refused.push(refuse(
                    "This is this space's own agent session, which is never removed as a worker.",
                ));
                continue;
            }
            if bound.contains(id.as_str()) {
                outcome.refused.push(refuse(
                    "This session is bound to a Tomori space, so it is not a worker to remove.",
                ));
                continue;
            }
            let Some(worker) = shown.iter().find(|worker| worker.id == id) else {
                outcome
                    .refused
                    .push(refuse("This is not one of this space's workers."));
                continue;
            };
            let family = family(shown, worker);
            if family
                .iter()
                .any(|member| bound.contains(member.id.as_str()))
            {
                outcome.refused.push(refuse(
                    "One of this worker's own workers is bound to a Tomori space, so it is not removed.",
                ));
                continue;
            }
            if !removal.force {
                if running(worker.state) {
                    outcome.refused.push(refuse(
                        "This worker is still running. Remove it only once it has finished, or, if someone asked to stop it, with force.",
                    ));
                    continue;
                }
                if family.iter().any(|member| running(member.state)) {
                    outcome.refused.push(refuse(
                        "One of this worker's own workers is still running. Remove it only once they have finished, or, if someone asked to stop them, with force.",
                    ));
                    continue;
                }
            }
            if targets.len() == REMOVAL_LIMIT {
                outcome.refused.push(refuse(
                    "Tomori removes at most 64 workers in one call; ask again for the rest.",
                ));
                continue;
            }
            targets.push(Target { family });
        }
        // A worker within another one removed goes with it, so only the outermost of each family is asked for.
        let within = |target: &Target, other: &Target| {
            !std::ptr::eq(target, other)
                && other.family[1..]
                    .iter()
                    .any(|member| member.id == target.family[0].id)
        };
        let outermost: Vec<bool> = targets
            .iter()
            .map(|target| !targets.iter().any(|other| within(target, other)))
            .collect();
        let targets = targets
            .into_iter()
            .zip(outermost)
            .filter_map(|(target, outermost)| outermost.then_some(target))
            .collect();
        Ok(Plan {
            space,
            backend,
            session: root,
            targets,
            outcome,
            force: removal.force,
        })
    }

    /// Remove `removal` from the space bound to `session`, as its agent asked, and only that space's own workers: each one the space shows is checked here, and again by the backend against the agent's own records as it is removed. Everyone in the space sees them leave, with a notice on its Tomori wall. See `design/mcp.md`.
    pub(crate) async fn agent_remove_workers(
        &self,
        session: &str,
        removal: Removal,
    ) -> Result<Outcome, String> {
        let plan = self.plan_removal(&self.lock(), session, removal)?;
        let backend = self
            .backends
            .get(&plan.backend)
            .expect("a space's backend is the server's");
        let mut outcome = plan.outcome;
        for target in plan.targets {
            let worker = &target.family[0];
            match backend
                .delete_worker(&plan.session, &worker.id, plan.force)
                .await
            {
                Ok(()) => outcome
                    .removed
                    .extend(target.family.iter().map(|member| Gone {
                        session: member.id.clone(),
                        title: member.title.clone(),
                        finished: worker_finished(member.state),
                        stopped: running(member.state),
                        with: (member.id != worker.id).then(|| worker.id.clone()),
                    })),
                Err(error) if matches!(error.kind, ErrorKind::Busy) => outcome.refused.push((
                    worker.id.clone(),
                    format!(
                        "The agent says this worker, or one of its own, is still running: {}. Remove it only once it has finished, or, if someone asked to stop it, with force.",
                        error.message
                    ),
                )),
                Err(error) if matches!(error.kind, ErrorKind::Invalid) => {
                    outcome.refused.push((
                        worker.id.clone(),
                        format!(
                            "The agent refused to remove it as one of this space's workers: {}.",
                            error.message
                        ),
                    ));
                }
                Err(error) => outcome.failed.push((
                    worker.id.clone(),
                    format!("It could not be removed: {}.", error.message),
                )),
            }
        }
        self.removed_workers(&plan.space, &outcome.removed, outcome.requester.as_ref());
        Ok(outcome)
    }

    /// The workers `removed` left the space `space`, as `by` asked, if the agent named who did: everyone in it sees them go, and a notice on its Tomori wall says so, and who asked, replacing the last one. Kept and logged.
    fn removed_workers(&self, space: &str, removed: &[Gone], by: Option<&Requester>) {
        if removed.is_empty() {
            return;
        }
        let ids: HashSet<String> = removed.iter().map(|gone| gone.session.clone()).collect();
        let mut state = self.lock();
        // The space may have been deleted while they were removed.
        let Some(record) = state.space_mut(space) else {
            return;
        };
        record.model.remove_workers(&ids);
        let environments = self.environments();
        let walls = environments.walls(&record.model.environment);
        let content = crate::stage::Content {
            title: "Workers".to_owned(),
            body: notice(removed, by.map(|by| by.name.as_str())),
            meta: vec![("source".to_owned(), "Tomori".to_owned())],
            code: None,
            slug: None,
            image: None,
            diagram: None,
            warnings: Vec::new(),
        };
        let panel = record
            .stage
            .project_notice(&walls, NOTICE, content, crate::cards::unix_now());
        drop(environments);
        let names: Vec<&str> = removed.iter().map(|gone| gone.session.as_str()).collect();
        let (count, names) = (workers(removed.len()), names.join(", "));
        let asked = by.map_or_else(String::new, |by| format!(", as {} asked", by.logged()));
        log::info!(
            "The agent of space {space} removed {count}{asked}: {names}; projected {panel}."
        );
        state.touch_space(space);
        state.touch_projection(space);
        drop(state);
        self.keep();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gone(title: &str, finished: bool, stopped: bool) -> Gone {
        Gone {
            session: title.to_owned(),
            title: title.to_owned(),
            finished,
            stopped,
            with: None,
        }
    }

    #[test]
    fn a_notice_counts_what_was_removed_and_names_each() {
        assert_eq!(
            notice(&[gone("Tests", true, false)], None),
            "Tomori removed 1 finished worker.\n\n- Tests"
        );
        let tasks = |count: usize, title: &dyn Fn(usize) -> String| -> Vec<Gone> {
            (1..=count)
                .map(|number| gone(&title(number), true, false))
                .collect()
        };
        let task = |number: usize| format!("Task {number}");
        // Twelve, more than the five once named, are all named.
        let said = notice(&tasks(12, &task), None);
        let names: Vec<String> = (1..=12).map(|number| format!("- Task {number}")).collect();
        assert_eq!(
            said,
            format!(
                "Tomori removed 12 finished workers.\n\n{}",
                names.join("\n")
            )
        );
        // As many as a panel's body has lines for, and no more, are all named.
        let fits = crate::stage::BODY_LINES - 2;
        let said = notice(&tasks(fits, &task), None);
        assert_eq!(said.lines().count(), crate::stage::BODY_LINES);
        assert!(said.ends_with(&format!("- Task {fits}")), "{said}");
        // Beyond the body's lines, the last line counts the rest.
        let said = notice(&tasks(40, &task), None);
        assert_eq!(said.lines().count(), crate::stage::BODY_LINES);
        assert!(said.ends_with("- Task 27\n- and 13 more"), "{said}");
        // Beyond the body's characters, too.
        let long = |number: usize| format!("{number:02} {}", "x".repeat(77));
        let said = notice(&tasks(20, &long), None);
        let text: usize = said.lines().map(|line| line.chars().count()).sum();
        assert!(text <= crate::stage::BODY_CHARS, "{text}");
        assert!(said.ends_with("x\n- and 6 more"), "{said}");
        assert!(
            said.contains("- 14 x") && !said.contains("- 15 x"),
            "{said}"
        );
        // Exactly as many characters as a body holds are all named; one more, and the rest are counted.
        let text = |said: &str| -> usize { said.lines().map(|line| line.chars().count()).sum() };
        let ending = |last: usize| {
            move |number: usize| match number {
                15 => "y".repeat(last),
                number => long(number),
            }
        };
        let said = notice(&tasks(15, &ending(15)), None);
        assert_eq!(text(&said), crate::stage::BODY_CHARS);
        assert!(said.ends_with(&format!("\n- {}", "y".repeat(15))), "{said}");
        let said = notice(&tasks(15, &ending(16)), None);
        assert!(said.ends_with("x\n- and 1 more"), "{said}");
        // Counting the rest, a name that just fills what is left, room kept for the count at its longest, "- and 16
        // more", is still named.
        let filling = |number: usize| match number {
            15 => "ab".to_owned(),
            16 => "z".repeat(12),
            number => long(number),
        };
        let said = notice(&tasks(16, &filling), None);
        assert_eq!(text(&said), crate::stage::BODY_CHARS - 1);
        assert!(said.ends_with("\n- ab\n- and 1 more"), "{said}");
        assert_eq!(
            notice(&[gone("Build", false, true)], None),
            "Tomori removed 1 worker. It was still running, and was stopped first.\n\n- Build"
        );
        // Who asked, when the server checked who did.
        assert_eq!(
            notice(&[gone("Build", false, true)], Some("Ada")),
            "Tomori removed 1 worker, as Ada asked. It was still running, and was stopped first.\n\n- Build"
        );
        assert_eq!(
            notice(
                &[
                    gone("A", false, true),
                    gone("B", false, true),
                    gone("C", true, false)
                ],
                None
            ),
            "Tomori removed 3 workers. 2 were still running, and were stopped first.\n\n- A\n- B\n- C"
        );
        assert_eq!(
            notice(&[gone("A", false, true), gone("B", false, true)], None),
            "Tomori removed 2 workers. They were still running, and were stopped first.\n\n- A\n- B"
        );
        assert_eq!(
            notice(&[gone("Idle", false, false)], None),
            "Tomori removed 1 worker.\n\n- Idle"
        );
        assert_eq!(
            notice(&[gone("A", false, true), gone("B", true, false)], None),
            "Tomori removed 2 workers. One was still running, and was stopped first.\n\n- A\n- B"
        );
    }

    #[test]
    fn a_notice_is_already_a_clean_body() {
        let titles = [
            "Résumé of the 日本語 build 🚀".to_owned(),
            "[Pointing at worker \"Tests\"] (session ses_1) {braces} <angle>".to_owned(),
            format!("Long {}", "ü".repeat(200)),
            format!("[{}]", "w".repeat(120)),
        ];
        for count in [4, 40] {
            let removed: Vec<Gone> = (0..count)
                .map(|index| gone(&titles[index % titles.len()], true, false))
                .collect();
            let said = notice(&removed, Some("Ada"));
            assert_eq!(crate::stage::clean_body(&said), said);
        }
    }

    #[test]
    fn a_family_takes_every_depth_once() {
        let worker = |id: &str, parent: Option<&str>| Worker {
            id: id.to_owned(),
            parent: parent.map(str::to_owned),
            title: id.to_owned(),
            state: WorkerState::Completed,
        };
        let workers = [
            worker("a", None),
            worker("b", Some("a")),
            worker("c", Some("b")),
            worker("d", None),
        ];
        let ids: Vec<String> = family(&workers, &workers[0])
            .into_iter()
            .map(|member| member.id)
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }
}
