// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use tomori_backend::{AnswerValue, Event, Modes, Part, Permission, Prompt, Snapshot};
use tomori_protocol::{
    Activity, Attention, Participant, QueueState, SpaceState, SpaceSummary, SpeechSetting,
};

use crate::limits;
use crate::projection;
use crate::queue::InputQueue;

/// Why a participant's decision, answer or mode choice is refused while the backend reconnects.
pub const RECONNECTING: &str = "The agent is reconnecting; try again once it is back.";

/// Messages kept in the shared conversation.
const MESSAGE_LIMIT: usize = 50;

/// Longest message text sent to clients.
const MESSAGE_TEXT_LIMIT: usize = 20_000;

/// A participant's entry of input that the space gave its agent, the last one given, which began the agent's current
/// turn: Tomori's number for it, who said it and when it was given, so that a tool the agent calls on someone's word
/// can be checked against who spoke. Only the server makes one, from input a participant's connection sent; nothing
/// the agent, a tool or a visitor sends makes one. See `design/security.md`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Given {
    /// Tomori's number for the entry, counting the entries given to this space's agent since the server started.
    pub entry: u64,
    /// The speaker's participant number, their public handle.
    pub source: u32,
    /// The speaker's display name, as the agent was given it.
    pub speaker: String,
    /// When it was given to the agent.
    pub at: std::time::Instant,
    /// Whether Tomori's own notes, such as a timer that fired or a worker's report, were carried in front of the
    /// words, so that what the agent does in the turn may answer the notes rather than the words. See
    /// `design/environment-actions.md`.
    pub carried: bool,
    /// Whether input that did not come through Tomori, or that Tomori may have missed, reached the agent after it, so
    /// that its words are no longer the agent's latest input, and why.
    pub superseded: Option<Superseded>,
}

/// Where a speaker's head was, and the way it faced, as their words began, and when they began. See
/// `design/projection.md`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Origin {
    pub head: tomori_protocol::Pose,
    pub at: std::time::Instant,
}

/// Who stands behind what the agent does in its turn, for placing what it projects, or where it goes, for whoever
/// asked: the speaker of the entry given last, whether the turn still answers their words, and, while it does, where
/// they were as their words began, if the space knows. The hub finds where they are now. See `design/projection.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Asked {
    /// The participant number of the speaker of the entry given last, if any was.
    pub source: Option<u32>,
    /// Whether their words are still the agent's latest input: nothing reached it since, such as Tomori's notes
    /// alone, a sub-agent's report or a message typed into its session outside Tomori.
    pub answering: bool,
    /// Where they were as their words began, while the turn answers them.
    pub origin: Option<Origin>,
}

/// Why the entry given last is no longer the agent's latest input. See `design/security.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Superseded {
    /// The agent's session was given input Tomori did not send, such as a message typed into it outside Tomori, or
    /// the agent's service's report that a sub-agent finished.
    Input,
    /// Tomori's connection to the agent's session was lost, so it may have missed input the session was given.
    Reconnected,
    /// Tomori gave the agent a prompt of its own notes alone, such as a timer firing, which no participant asked for.
    /// See `design/timers.md`.
    Event,
}

/// How many of the keys of entries it gave its agent a space remembers, to tell its own deliveries from input that
/// did not come through Tomori, which is more than can be delivered between two entries.
const GIVEN_KEYS: usize = 64;

/// The shared state of one space, driven by backend events and participant actions. Pure: no I/O.
#[derive(Debug)]
pub struct SpaceModel {
    pub id: String,
    /// The title shown: its session's, unless someone renamed the space; see [`Self::rename`].
    pub title: String,
    /// Who gave the space its title in place of its session's, if anyone has since it last followed the session. See
    /// `design/spaces.md`.
    renamed_by: Option<tomori_protocol::Renamer>,
    /// The session's own title, as the backend last reported it, cleaned, if it has.
    session_title: Option<String>,
    pub backend: String,
    /// The directory its session is in, which the space follows when the agent moves the session; see
    /// [`Self::follow_directory`].
    pub directory: String,
    /// The environment the space is bound to, the same for every participant, which an owner may change.
    pub environment: String,
    /// The bound session's identifier, namespaced by backend.
    pub session: String,
    /// Participants (owners and collaborators), by their hub key, in join order.
    pub participants: Vec<String>,
    /// Visitors watching the space, by their hub key, in join order. They receive what participants receive, but are not participants; see `design/security.md`.
    pub visitors: Vec<String>,
    busy: Option<bool>,
    closed: bool,
    messages: Vec<MessageModel>,
    /// Backend user messages, whose text is already shown from the delivered prompt.
    user_messages: HashSet<String>,
    queue: InputQueue,
    in_flight: Option<Prompt>,
    /// The entry given to the agent last, if it was delivered and nothing has been given since; see [`Given`].
    given: Option<Given>,
    /// How many entries have been given to the agent, numbering each.
    entries_given: u64,
    /// The keys of the entries given to the agent most recently, at most [`GIVEN_KEYS`], as its backend reports
    /// delivering them, so that a delivery of any other is input that did not come through Tomori.
    given_keys: VecDeque<String>,
    /// Pending permissions, oldest first. The first is presented; the rest wait their turn.
    approvals: VecDeque<Permission>,
    /// Permissions whose decision has been sent to the backend but not yet confirmed.
    deciding: Vec<Permission>,
    /// Pending questions, oldest first. The first is presented and collects answers.
    questions: VecDeque<PendingQuestion>,
    /// Questions whose answers have been sent to the backend but not yet confirmed.
    submitting: Vec<PendingQuestion>,
    workers: Vec<tomori_protocol::Worker>,
    workers_authoritative: bool,
    /// When this space saw each of its workers finish, in seconds since the epoch: only those it saw go from not
    /// finished to finished, and only while they stay finished. See `design/mcp.md`.
    finished: HashMap<String, u64>,
    attribution: bool,
    /// How much of each reply is spoken; see `design/speech.md`.
    speech: SpeechSetting,
    /// How the agent moves about the space, an owner's setting, kept with the space. See `design/environments.md`.
    movement: tomori_protocol::AgentMovement,
    /// The modes the session offers and its current one, as the agent last reported them.
    modes: Modes,
    error: Option<String>,
    /// Worker observation failures are separate from unrelated space errors.
    worker_error: Option<String>,
    /// Why the environment's server side failed, until the space is bound to another environment. See `design/environments.md`.
    environment_error: Option<String>,
    /// The backend reported a lost connection and has not yet read the session again, with the reason it gave. The
    /// space's state may be stale meanwhile, so no input is released.
    reconnecting: Option<String>,
    /// Requests whose decision or answers the backend accepted since it reported reconnecting. The state it reads
    /// again may predate them, so a resynchronization does not present them again.
    settled: HashSet<String>,
    /// Something that happened while nobody was in the space and that nobody has seen since: [`Attention::Problem`]
    /// or [`Attention::Reply`], the higher kept. Cleared by [`Self::attend`]. See `design/spaces.md`.
    unattended: Option<Attention>,
    /// The agent completed a reply in the current turn while nobody was in the space, so its turn ending is a reply
    /// nobody heard.
    unheard: bool,
    /// The space's own instructions to the agent, kept even while its backend takes none. See `design/spaces.md`.
    instructions: String,
    /// The space's backend takes instructions.
    takes_instructions: bool,
    /// Why the agent may not have the instructions shown, until they are next given successfully.
    instructions_error: Option<String>,
    /// What each waiting queue entry's speaker attended to as they began speaking, by entry key, as the space knew it
    /// then. Told to the agent with the entry. See `design/spaces.md`.
    attending: BTreeMap<String, Vec<Attended>>,
    /// Where each waiting queue entry's speaker's head was, and the way it faced, as they began speaking, and when, by
    /// entry key. See `design/projection.md`.
    origins: BTreeMap<String, Origin>,
    /// What each waiting queue entry's speaker had seen of the agent's messages as they began speaking, by entry key,
    /// so that the agent is told which message they answer if it said more before their words arrived. See
    /// `design/interaction.md`.
    seen: BTreeMap<String, Seen>,
    /// The [`MessageModel::order`] of the message the space saw last.
    last_order: u64,
    /// Where the speaker of the input delivered last stood, and the way they faced, as they began speaking, and when, if
    /// the space knows: what the agent projects in answer goes where they were looking, or beside them.
    origin: Option<Origin>,
    /// The panels of each reply completed since the hub last took them, with who stood behind it as it completed.
    projected: Vec<(Vec<crate::stage::Found>, Asked)>,
    /// The latest replies whose panels were taken, so that none is projected twice.
    projected_replies: VecDeque<String>,
    /// What participants did that the agent is told before the next words spoken in the space, oldest first: each
    /// checklist item ticked or unticked, by its panel and line, as [`SpaceModel::note_tick`] keeps it. See
    /// `design/projection.md`.
    notes: VecDeque<(String, u32, String)>,
    /// The space's timers, kept with it. See `design/timers.md`.
    timers: crate::timers::Timers,
    /// Notes from Tomori's own sources, such as its timers, waiting to be told to the agent. See `design/timers.md`.
    events: crate::events::EventQueue,
    /// When the agent's turn last ended, after which a prompt of notes alone waits a quiet moment.
    quiet_since: Option<std::time::Instant>,
    /// How many of the agent's turns have ended, which tells one turn from the next. See [`SpaceModel::turns`].
    turns: u64,
    /// The participant the notes alone given the agent last were about, while they are its latest input. See
    /// [`SpaceModel::noted`].
    noted: Option<String>,
    /// The watch over its workers, which tells the agent of those that failed, stalled or left a message unanswered.
    /// See `design/events.md`.
    watch: crate::watch::Watch,
}

/// The most notes of what participants did that the space keeps for the agent until the next words, the oldest left
/// out first.
const NOTES: usize = 8;

/// How many replies the space remembers having projected the panels of; more than the conversation keeps.
const PROJECTED_REPLIES: usize = 2 * MESSAGE_LIMIT;

/// A resource a participant attended to as they began speaking, as the space knew it then: never as their client
/// described it. Made only by [`SpaceModel::attended`], from an identity the space has, so that nothing else can
/// make one from what a client says.
#[derive(Clone, Debug, PartialEq)]
pub struct Attended(Resolved);

impl Attended {
    /// Whether `other` is the same resource, however each describes it.
    fn is(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Resolved::Worker { id, .. }, Resolved::Worker { id: other, .. })
            | (Resolved::Card { id, .. }, Resolved::Card { id: other, .. })
            | (Resolved::Object { id, .. }, Resolved::Object { id: other, .. })
            | (Resolved::Gathered { anchor: id, .. }, Resolved::Gathered { anchor: other, .. })
            | (Resolved::Timer { name: id, .. }, Resolved::Timer { name: other, .. }) => {
                id == other
            }
            // Two parts of one panel, as each hand points at a node of one volume, are two.
            (
                Resolved::Panel { id, description },
                Resolved::Panel {
                    id: other,
                    description: described,
                },
            ) => id == other && description == described,
            _ => false,
        }
    }
}

/// What an [`Attended`] resource is. Each kind of [`tomori_protocol::Resource`] the server resolves has a variant here.
#[derive(Clone, Debug, PartialEq)]
enum Resolved {
    /// One of the space's delegated workers.
    Worker { id: String, title: String },
    /// A card on the board of a space that shows cards, as [`crate::cards::Board::describe`] described it as the
    /// utterance began, since the board changes only when it is read again. See `design/github.md`.
    Card { id: String, description: String },
    /// Something the space's environment presents, as its server side last described it as the utterance began, with
    /// the id the agent may inspect it by, its reference after the environment's identifier, if it has one. See
    /// `design/environments.md`. With the names of the actions it offers, the first few, which the agent may take on it
    /// with `act`. See `design/environment-actions.md`.
    Object {
        id: String,
        description: String,
        reference: Option<String>,
        offers: Vec<String>,
    },
    /// A panel projected in the space, as the stage described it as the utterance began. See `design/projection.md`.
    Panel { id: String, description: String },
    /// A cluster anchor's "+N more" sign, by the anchor's name, and the workers gathered into it as the utterance
    /// began, each by session and title, described as the space knows them when the words are delivered. See
    /// `design/interaction.md`.
    Gathered {
        anchor: String,
        workers: Vec<(String, String)>,
    },
    /// One of the space's timers, by its name, as the space described it as the utterance began. See
    /// `design/timers.md`.
    Timer { name: String, description: String },
}

/// The longest worker title told to the agent, in characters.
const CONTEXT_TITLE_CHARS: usize = 160;

/// The longest context block told to the agent with one utterance's words, in characters, its brackets included.
/// Resources that would not fit, after the first, are left out.
pub const CONTEXT_CHARS: usize = 1_000;

/// How a panel attended to is described: by its identity, the line of its body pointed at, the row's id and where across
/// a chart's time axis, as the client named them, from the stage's own store, or `None` where the space does not show
/// it.
pub type DescribePanel<'a> = dyn Fn(&str, crate::stage::Pointed<'_>) -> Option<String> + 'a;

/// Which workers a cluster anchor's "+N more" sign gathers, by the anchor's name, each by session, in the order of their
/// slots, as the space lays its clusters out, or `None` where it has no such anchor or the sign gathers none.
pub type Gathered<'a> = dyn Fn(&str) -> Option<Vec<String>> + 'a;

/// The most characters of the workers a "+N more" sign gathers told to the agent, after which the rest are counted.
const GATHERED_CHARS: usize = 800;

/// `text` on one line of at most `limit` characters, as [`tomori_protocol::text::one_line`] makes all text others choose, with the context block's delimiters, double quotes and square brackets, made single quotes and parentheses, so that nothing in it can end the quotation or the block early, or begin another.
pub(crate) fn one_line(text: &str, limit: usize) -> String {
    tomori_protocol::text::one_line(text, limit)
        .chars()
        .map(|c| match c {
            '"' => '\'',
            '[' => '(',
            ']' => ')',
            c => c,
        })
        .collect()
}

/// The longest environment identifier told before an object's reference, in characters.
const ENVIRONMENT_CHARS: usize = 64;

/// The id the agent inspects an object of `environment` by, its `reference` after the environment's identifier, such as
/// `farmyard:hen/marigold`, so that objects of different environments never share one. A reference has no `:`, so the
/// last one ends the environment's identifier.
pub(crate) fn inspectable(environment: &str, reference: &str) -> String {
    format!("{}:{reference}", one_line(environment, ENVIRONMENT_CHARS))
}

/// One resource `attended` to: a worker described from `workers`, the space's workers when the words are delivered,
/// or as the space last knew it and no longer present, such as `worker "Tests" (opencode session ses_1, running)`; a
/// card as the board described it as the words began.
fn describe(backend: &str, attended: &Attended, workers: &[tomori_protocol::Worker]) -> String {
    let (id, title) = match attended {
        Attended(Resolved::Worker { id, title }) => (id, title),
        // Each worker it gathers, as many as fit, and how many more.
        Attended(Resolved::Gathered {
            anchor,
            workers: gathered,
        }) => {
            let mut listed: Vec<String> = Vec::new();
            let mut length = 0;
            for (id, title) in gathered {
                let worker = describe_worker(backend, id, title, workers);
                if !listed.is_empty() && length + worker.chars().count() > GATHERED_CHARS {
                    break;
                }
                length += worker.chars().count() + 2;
                listed.push(worker);
            }
            let rest = gathered.len() - listed.len();
            let mut said = format!(
                "the \"+{} more\" sign at {}, which stands for {}",
                gathered.len(),
                one_line(anchor, tomori_protocol::cluster::NAME_BYTES),
                listed.join(", ")
            );
            if rest > 0 {
                said.push_str(&format!(" and {rest} more"));
            }
            return said;
        }
        Attended(Resolved::Card { description, .. }) => return description.clone(),
        // Cleaned as the side described it; its identity, which a client gave, is not told, but the id the agent may
        // inspect it by is, if the side gave one.
        Attended(Resolved::Object {
            description,
            reference,
            offers,
            ..
        }) => {
            return match reference {
                Some(reference) if offers.is_empty() => format!("{description} (id {reference})"),
                Some(reference) => {
                    format!(
                        "{description} (id {reference}; offers {})",
                        offers.join(", ")
                    )
                }
                None => description.clone(),
            };
        }
        // As the stage described it, from its own store, and a timer as the space did.
        Attended(Resolved::Panel { description, .. } | Resolved::Timer { description, .. }) => {
            return description.clone();
        }
    };
    describe_worker(backend, id, title, workers)
}

/// The most actions the note before someone's words names of what they point at, and the most characters their names
/// take together, so that the note stays short; the agent finds the rest with `inspect`.
const OFFERED_NAMES: usize = 3;
const OFFERED_CHARS: usize = 60;

/// The names of the first of `actions` the note before someone's words names, within [`OFFERED_NAMES`] and
/// [`OFFERED_CHARS`].
fn offered_names(actions: &[&tomori_environment::Action]) -> Vec<String> {
    let mut names = Vec::new();
    let mut length = 0;
    for action in actions.iter().take(OFFERED_NAMES) {
        length += action.name().len() + if names.is_empty() { 0 } else { 2 };
        if length > OFFERED_CHARS {
            break;
        }
        names.push(action.name().to_owned());
    }
    names
}

/// The worker `id`, titled `title` as the space last knew it, described from `workers`, the space's workers when the
/// words are delivered: `worker "Tests" (opencode session ses_1, running)`.
fn describe_worker(
    backend: &str,
    id: &str,
    title: &str,
    workers: &[tomori_protocol::Worker],
) -> String {
    use tomori_protocol::WorkerState;
    let session = one_line(id, limits::SESSION_BYTES);
    let backend = one_line(backend, limits::NAME_BYTES);
    let (title, state) = match workers.iter().find(|worker| worker.id == id) {
        Some(worker) => (
            worker.title.as_str(),
            match worker.state {
                WorkerState::Running => "running",
                WorkerState::Waiting => "waiting",
                WorkerState::Idle => "idle",
                WorkerState::Completed => "completed",
                WorkerState::Error => "failed",
                WorkerState::Stopped => "stopped",
                WorkerState::Stalled => "stalled",
                WorkerState::Unanswered => "not answering",
                WorkerState::Unknown => "state unknown",
            },
        ),
        None => (title, "no longer present"),
    };
    let title = one_line(title, CONTEXT_TITLE_CHARS);
    format!("worker \"{title}\" ({backend} session {session}, {state})")
}

/// The context block told to the agent before the words of a participant who attended to `attention` as they began
/// speaking, such as `[Pointing at worker "Tests" (opencode session ses_1, running)]`, or `None` for nothing. Each
/// resource is described as the space knows it when the words are delivered, separated by `; `, and those that would
/// make the block longer than [`CONTEXT_CHARS`], after the first, are left out.
fn context(
    backend: &str,
    attention: &[Attended],
    workers: &[tomori_protocol::Worker],
) -> Option<String> {
    let mut block = String::new();
    let mut length = 0;
    for attended in attention {
        let description = describe(backend, attended, workers);
        let (separator, prefix) = if block.is_empty() {
            ("", "[Pointing at ")
        } else {
            ("; ", "")
        };
        let added = prefix.chars().count() + separator.len() + description.chars().count();
        // The first always fits, as each description is bounded; one more must leave room for the closing bracket.
        if !block.is_empty() && length + added + 1 > CONTEXT_CHARS {
            break;
        }
        block.push_str(prefix);
        block.push_str(separator);
        block.push_str(&description);
        length += added;
    }
    (!block.is_empty()).then(|| block + "]")
}

#[derive(Debug)]
struct MessageModel {
    id: String,
    role: tomori_protocol::Role,
    text: String,
    tools: Vec<ToolModel>,
    completed: bool,
    /// Where the message came in the conversation as the space first saw it, kept when the session is read again, so
    /// that what the agent said after a participant began speaking can be told from what they had seen. See
    /// [`Seen`].
    order: u64,
    /// When the space saw the message complete, in seconds since the epoch; `None` for one it read as completed.
    completed_at: Option<u64>,
}

/// The most characters of the end of one of the agent's messages quoted back to it, as [`ending`] makes it.
pub const ENDING_CHARS: usize = 60;

/// How `text`, one of the agent's messages, ends, to quote back to it: its words outside code blocks, without
/// Markdown's marks, on one line, as [`excerpt`] makes the start of a reply, and, when that is longer than
/// [`ENDING_CHARS`], its last sentence after "…", or, for a last sentence too long for that, as many of its last words
/// as fit. Its double quotes and square brackets are made single quotes and parentheses, as titles' are, so that it
/// cannot end the quotation or the note early, or read as one. Empty when the message says nothing outside code.
fn ending(text: &str) -> String {
    let mut words = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let line = trimmed.trim_start_matches(['#', '>', '-', '*', '+', ' ']);
        words.extend(
            line.split_whitespace()
                .map(|word| word.trim_matches(['*', '_', '`']))
                .filter(|word| !word.is_empty()),
        );
    }
    let line = tomori_protocol::text::clean_name(&words.join(" "));
    if line.chars().count() <= ENDING_CHARS {
        return one_line(&line, ENDING_CHARS);
    }
    // The last sentence: what follows the last mark that ends one before the marks that end the message.
    let body = line.trim_end_matches(['.', '?', '!', ' ']);
    let start = body.rfind(['.', '?', '!']).map_or(0, |mark| mark + 1);
    let sentence = line[start..].trim_start();
    let room = ENDING_CHARS - 1;
    let count = sentence.chars().count();
    let end = if count <= room {
        sentence.to_owned()
    } else {
        // As many whole words of its end as fit, or the end of a word longer than that.
        let tail: String = sentence.chars().skip(count - room).collect();
        match tail.split_once(' ') {
            Some((_, rest)) if !rest.is_empty() => rest.to_owned(),
            _ => tail,
        }
    };
    one_line(&format!("…{end}"), ENDING_CHARS)
}

/// How long ago something was, `seconds` ago, to tell the agent: in seconds under a minute, in minutes under an hour,
/// and in hours after that.
fn ago(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds} s ago"),
        60..3600 => format!("{} min ago", seconds / 60),
        _ => format!("{} h ago", seconds / 3600),
    }
}

/// Whether `message` is the agent's and says something outside code: what a participant hears or reads of it.
fn says_something(message: &MessageModel) -> bool {
    message.role == tomori_protocol::Role::Assistant && !ending(&message.text).is_empty()
}

/// What a participant had seen of the agent's messages as they began speaking: the newest of them, which their words
/// follow, unless the agent says more before they are delivered, when they answer an earlier message. Made only by
/// [`SpaceModel::seen`], from the space's own conversation. See `design/interaction.md`.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    /// The [`MessageModel::order`] of the message seen: the agent's messages after it are what the speaker had not.
    order: u64,
    /// The message seen, as the space knew it as they began, if there was one.
    earlier: Option<Earlier>,
}

/// The message a participant had seen last as they began speaking, as the space knew it then, so that it can be named
/// even once the conversation no longer keeps it.
#[derive(Clone, Debug, PartialEq)]
struct Earlier {
    id: String,
    /// How it ended, as [`ending`] says.
    ending: String,
    completed_at: Option<u64>,
}

/// The note told to the agent before the words of a speaker who had seen what `seen` says, when it said something in
/// `messages` after that by `now`, in seconds since the epoch, that they had not: that they answer the earlier message,
/// as `[In reply to your earlier message (2 min ago) ending "…should I do A?", before "…should I do B?"]`, naming
/// the newest message they had not seen. `None` when the agent said nothing after what they had seen, so that words
/// that follow the conversation carry no note.
fn interleaved(messages: &[MessageModel], seen: &Seen, now: u64) -> Option<String> {
    let later = messages
        .iter()
        .rfind(|message| message.order > seen.order && says_something(message))?;
    let later = ending(&later.text);
    let Some(earlier) = &seen.earlier else {
        return Some(format!("[Said before your message ending \"{later}\"]"));
    };
    // As it is now, if the conversation still keeps it, since it may have gone on as they began.
    let (end, completed_at) = messages
        .iter()
        .find(|message| message.id == earlier.id)
        .map_or((earlier.ending.clone(), earlier.completed_at), |message| {
            (ending(&message.text), message.completed_at)
        });
    let age = completed_at.map_or(String::new(), |at| {
        format!(" ({})", ago(now.saturating_sub(at)))
    });
    let end = if end.is_empty() {
        String::new()
    } else {
        format!(" ending \"{end}\"")
    };
    Some(format!(
        "[In reply to your earlier message{age}{end}, before \"{later}\"]"
    ))
}

#[derive(Debug)]
struct ToolModel {
    id: String,
    name: String,
    state: &'static str,
}

#[derive(Clone, Debug)]
struct PendingQuestion {
    question: tomori_backend::Question,
    answers: Vec<(String, AnswerValue)>,
}

/// The outcome of a participant's answer to the current question field.
#[derive(Debug, PartialEq)]
pub enum AnswerOutcome {
    /// The answer was recorded; the question has more fields.
    Recorded,
    /// The question is complete and these answers must be sent to the backend.
    Complete(Vec<(String, AnswerValue)>),
}

impl MessageModel {
    /// Displayable text: the message text, or its tool activity when it has no text yet.
    fn display(&self) -> String {
        let text = if self.text.is_empty() {
            self.tools
                .iter()
                .map(|tool| format!("[{}: {}]", tool.name, tool.state))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            self.text.clone()
        };
        match text.char_indices().nth(MESSAGE_TEXT_LIMIT) {
            Some((index, _)) => text[..index].to_owned(),
            None => text,
        }
    }
}

fn role(role: tomori_backend::Role) -> tomori_protocol::Role {
    match role {
        tomori_backend::Role::User => tomori_protocol::Role::User,
        tomori_backend::Role::Assistant => tomori_protocol::Role::Assistant,
    }
}

/// A snapshot message as the space shows it: its text parts joined, and its tool calls.
fn message_model(message: tomori_backend::SnapshotMessage) -> MessageModel {
    let mut text = Vec::new();
    let mut tools = Vec::new();
    for part in message.parts {
        match part {
            Part::Text { text: part } => text.push(part),
            Part::Reasoning { .. } => {}
            Part::Tool {
                id, tool, finished, ..
            } => tools.push(ToolModel {
                id,
                name: tool,
                state: if finished { "done" } else { "working" },
            }),
        }
    }
    MessageModel {
        id: message.id,
        role: role(message.role),
        text: text.join("\n\n"),
        tools,
        completed: message.completed,
        order: 0,
        completed_at: None,
    }
}

/// Project a backend worker. A worker whose parent is the space's own session is reported without a parent, so clients need not know the backend's session identifier.
fn worker(worker: tomori_backend::Worker, session: &str) -> tomori_protocol::Worker {
    use tomori_backend::WorkerState as From;
    use tomori_protocol::WorkerState as To;
    tomori_protocol::Worker {
        id: worker.id,
        parent: worker.parent.filter(|parent| parent != session),
        title: worker.title,
        state: match worker.state {
            From::Running => To::Running,
            From::Waiting => To::Waiting,
            From::Idle => To::Idle,
            From::Completed => To::Completed,
            From::Error => To::Error,
            From::Stopped => To::Stopped,
            From::Unknown => To::Unknown,
        },
    }
}

/// Whether a worker in `state` has finished: completed, failed or stopped. See `design/mcp.md`.
pub fn worker_finished(state: tomori_protocol::WorkerState) -> bool {
    use tomori_protocol::WorkerState;
    matches!(
        state,
        WorkerState::Completed | WorkerState::Error | WorkerState::Stopped
    )
}

/// `update`, the workers a backend now lists in its own order, oldest first as far as the space has seen: those in
/// `known`, the workers it already had, keep their places, and new ones follow in the order the backend gave, so that
/// trimming a snapshot drops the oldest workers first. See `design/server.md`.
fn oldest_first(
    known: &[tomori_protocol::Worker],
    mut update: Vec<tomori_protocol::Worker>,
) -> Vec<tomori_protocol::Worker> {
    let places: HashMap<&str, usize> = known
        .iter()
        .enumerate()
        .map(|(place, worker)| (worker.id.as_str(), place))
        .collect();
    update.sort_by_key(|worker| {
        places
            .get(worker.id.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    update
}

/// Whether `message` is a reply: an assistant message the agent completed with text, not tool calls alone.
fn is_reply(message: &MessageModel) -> bool {
    message.role == tomori_protocol::Role::Assistant
        && message.completed
        && !message.text.trim().is_empty()
}

/// The most characters of the newest reply's start that the lobby's listing shows of a space. See `design/arrival.md`.
pub const LATEST_CHARS: usize = 60;

/// The start of a reply's `text`, as the lobby's listing shows it: its words outside code blocks, without Markdown's
/// marks for headings, quotes, lists and emphasis, on one line as a display name is cleaned, and cut at a word with
/// "…" to at most [`LATEST_CHARS`] characters; `None` when no words are left.
pub fn excerpt(text: &str) -> Option<String> {
    let mut words = Vec::new();
    let mut fenced = false;
    // Only the start is shown, so a long reply is read no further than enough words for it.
    for line in text.lines() {
        if words
            .iter()
            .map(|word: &String| word.len() + 1)
            .sum::<usize>()
            > LATEST_CHARS * 4
        {
            break;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let line = trimmed.trim_start_matches(['#', '>', '-', '*', '+', ' ']);
        words.extend(
            line.split_whitespace()
                .map(|word| word.trim_matches(['*', '_', '`']))
                .filter(|word| !word.is_empty())
                .map(str::to_owned),
        );
    }
    let line = tomori_protocol::text::clean_name(&words.join(" "));
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= LATEST_CHARS {
        return Some(line);
    }
    // Cut at the last space that leaves room for the mark, or within a word longer than that.
    let room: String = line.chars().take(LATEST_CHARS - 1).collect();
    let cut = room
        .rfind(' ')
        .map_or(room.as_str(), |space| &room[..space]);
    Some(format!(
        "{}…",
        cut.trim_end_matches([',', ';', ':', '.', ' '])
    ))
}

/// Whether `event` shows the agent working in its space, so that the space was active as it arrived: a turn starting
/// or ending, a message or a tool starting, changing or ending, input reaching the agent, or a request asked of the
/// participants. Reconnecting, reading the session again and changes of workers, modes or title are not. See
/// `design/arrival.md`.
pub fn is_activity(event: &Event) -> bool {
    matches!(
        event,
        Event::Busy
            | Event::Idle
            | Event::Delivered { .. }
            | Event::MessageStarted { .. }
            | Event::TextDelta { .. }
            | Event::MessageCompleted { .. }
            | Event::ToolStarted { .. }
            | Event::ToolFinished { .. }
            | Event::PermissionRequested(_)
            | Event::QuestionAsked(_)
    )
}

/// `key` as a backend reports delivering it, without the `msg_` that OpenCode's message identifiers begin with, which
/// its backend adds to a key without one and removes again from the delivery it reports.
fn delivery_key(key: &str) -> &str {
    key.strip_prefix("msg_").unwrap_or(key)
}

/// The text delivered to the agent for a queue entry. With attribution, the speaker's display name prefixes the text as `Name: text`.
pub fn attributed(speaker: &str, text: &str, attribution: bool) -> String {
    if attribution && !speaker.is_empty() {
        format!("{speaker}: {text}")
    } else {
        text.to_owned()
    }
}

/// The prompt text for `text` said by `speaker`, as [`attributed`] makes it, with `notes`, what participants did since
/// the words before, and `context`, Tomori's notes of which message they answer and what they attended to, before the
/// words and after the speaker's name: `Bob: [Ada ticked …] [In reply to …] [Pointing at …] how is this one doing?`.
/// Square brackets in the words, such as a note of noise Whisper writes among them, as in "[Music] deploy it", are made
/// parentheses, as its other notes are, so that Tomori's notes are the only bracketed text in a prompt, with or without
/// them.
fn contextual(
    speaker: &str,
    notes: Option<String>,
    context: Option<String>,
    text: &str,
    attribution: bool,
) -> String {
    let text = text.replace('[', "(").replace(']', ")");
    let said: Vec<String> = notes.into_iter().chain(context).chain([text]).collect();
    attributed(speaker, &said.join(" "), attribution)
}

/// The block of `notes`, what participants did since the words before, told to the agent before the next words, such
/// as `[Ada ticked item 2 of 5 of panel "Agenda" (id agenda): "Panel ids"]`, separated by `; `, or `None` for none. The
/// latest are kept where they would make the block longer than [`CONTEXT_CHARS`], the oldest left out.
fn noted<'a>(notes: impl DoubleEndedIterator<Item = &'a str>) -> Option<String> {
    let mut kept: Vec<&str> = Vec::new();
    // The brackets.
    let mut length = 2;
    for note in notes.rev() {
        let added = note.chars().count() + if kept.is_empty() { 0 } else { 2 };
        if length + added > CONTEXT_CHARS {
            break;
        }
        length += added;
        kept.push(note);
    }
    kept.reverse();
    (!kept.is_empty()).then(|| format!("[{}]", kept.join("; ")))
}

/// Whether a space follows its session to `directory`, which the backend reports: one that is empty, not absolute, or
/// longer than [`limits::DIRECTORY_BYTES`] is not a directory the space can name.
fn followed(directory: &str) -> bool {
    std::path::Path::new(directory).is_absolute() && directory.len() <= limits::DIRECTORY_BYTES
}

impl SpaceModel {
    pub fn new(
        id: String,
        title: String,
        backend: String,
        directory: String,
        environment: String,
        session: String,
        snapshot: Snapshot,
    ) -> Self {
        let workers = snapshot
            .workers
            .into_iter()
            .map(|entry| worker(entry, &session))
            .collect();
        let session_title = snapshot.title.as_deref().and_then(limits::session_title);
        // Where the agent says the session is, if it says, over where it was asked to be.
        let directory = snapshot
            .directory
            .filter(|reported| followed(reported))
            .unwrap_or(directory);
        let mut model = Self {
            id,
            // The session's own title, where the backend reports it, which the space follows from the start.
            title: session_title.clone().unwrap_or(title),
            renamed_by: None,
            session_title,
            backend,
            directory,
            environment,
            session,
            participants: Vec::new(),
            visitors: Vec::new(),
            busy: snapshot.busy,
            closed: false,
            messages: Vec::new(),
            user_messages: HashSet::new(),
            queue: InputQueue::default(),
            in_flight: None,
            given: None,
            entries_given: 0,
            given_keys: VecDeque::new(),
            approvals: VecDeque::new(),
            deciding: Vec::new(),
            questions: VecDeque::new(),
            submitting: Vec::new(),
            workers,
            // A list the backend is still completing never authorizes a departure.
            workers_authoritative: snapshot.workers_incomplete.is_none(),
            finished: HashMap::new(),
            attribution: true,
            speech: SpeechSetting::Full,
            movement: tomori_protocol::AgentMovement::default(),
            modes: snapshot.modes,
            error: None,
            worker_error: snapshot.workers_incomplete,
            environment_error: None,
            reconnecting: None,
            settled: HashSet::new(),
            unattended: None,
            unheard: false,
            instructions: String::new(),
            takes_instructions: false,
            instructions_error: None,
            attending: BTreeMap::new(),
            origins: BTreeMap::new(),
            seen: BTreeMap::new(),
            last_order: 0,
            origin: None,
            projected: Vec::new(),
            projected_replies: VecDeque::new(),
            notes: VecDeque::new(),
            timers: crate::timers::Timers::default(),
            events: crate::events::EventQueue::default(),
            quiet_since: None,
            turns: 0,
            noted: None,
            watch: crate::watch::Watch::default(),
        };
        model.load_messages(snapshot.messages);
        for permission in snapshot.permissions {
            model.apply(Event::PermissionRequested(permission));
        }
        for question in snapshot.questions {
            model.apply(Event::QuestionAsked(question));
        }
        model
    }

    /// Replace the conversation with a snapshot's messages.
    fn load_messages(&mut self, messages: Vec<tomori_backend::SnapshotMessage>) {
        // A message the space already had keeps its place and when it completed, so that reading the session again
        // says nothing new was said; one listed before the last of those is no newer than the one before it, as when
        // the read lists more of the history, and only those after them are new.
        let known: HashMap<String, (u64, Option<u64>)> = self
            .messages
            .iter()
            .map(|message| (message.id.clone(), (message.order, message.completed_at)))
            .collect();
        let last_known = messages
            .iter()
            .rposition(|message| known.contains_key(&message.id));
        self.messages.clear();
        self.user_messages.clear();
        let mut previous = 0;
        for (index, message) in messages.into_iter().enumerate() {
            let mut model = message_model(message);
            match known.get(&model.id) {
                Some(&(order, completed_at)) => {
                    (model.order, model.completed_at) = (order, completed_at);
                    previous = order;
                    self.keep_message(model);
                }
                None if last_known.is_some_and(|last| index < last) => {
                    model.order = previous;
                    self.keep_message(model);
                }
                None => self.push_message(model),
            }
        }
    }

    /// Take the session as the backend read it again after losing events. The conversation, running state and modes
    /// are replaced. Pending requests the snapshot no longer lists are withdrawn, including any whose decision or
    /// answers are being delivered, since the agent no longer waits for them; those it lists and the space already
    /// holds keep their place and any partial answers, and new ones are queued after them. Workers are left to their
    /// own events. The error the backend reported for the gap no longer applies, so it is cleared, and input is released
    /// again.
    fn resynchronize(&mut self, snapshot: Snapshot) {
        let Snapshot {
            busy,
            messages,
            permissions,
            questions,
            workers: _,
            workers_incomplete: _,
            modes,
            title,
            directory,
        } = snapshot;
        // The session may have been renamed, or moved, while the space could not hear of it.
        if let Some(title) = title {
            self.follow(&title);
        }
        if let Some(directory) = directory {
            self.follow_directory(&directory);
        }
        // The agent keeps a message's text in its records only once the message ends, so a message read again while
        // it is still being written may list less text than the space already shows. Keep what the space has; the
        // text it missed while disconnected is restored when the message ends.
        let shown: Vec<(String, String)> = self
            .messages
            .iter()
            .map(|message| (message.id.clone(), message.text.clone()))
            .collect();
        let replied: HashSet<String> = self
            .messages
            .iter()
            .filter(|message| is_reply(message))
            .map(|message| message.id.clone())
            .collect();
        self.load_messages(messages);
        // A reply the agent completed during the gap was not heard either.
        if self.participants.is_empty()
            && self
                .messages
                .iter()
                .any(|message| is_reply(message) && !replied.contains(&message.id))
        {
            self.unheard = true;
        }
        // And their panels, never seen, are projected.
        let completed: Vec<usize> = (0..self.messages.len())
            .filter(|index| {
                let message = &self.messages[*index];
                is_reply(message) && !replied.contains(&message.id)
            })
            .collect();
        for index in completed {
            self.project_reply(index);
        }
        for message in self
            .messages
            .iter_mut()
            .filter(|message| !message.completed)
        {
            if let Some((_, text)) = shown.iter().find(|(id, _)| *id == message.id)
                && text.starts_with(message.text.as_str())
            {
                message.text.clone_from(text);
            }
        }
        if busy.is_some() {
            self.busy = busy;
        }
        // The turn may have ended during the gap, its idle event lost with the rest.
        if busy == Some(false) {
            self.turn_ended();
        }
        self.modes = modes;
        let listed: HashSet<&str> = permissions
            .iter()
            .map(|permission| permission.id.as_str())
            .collect();
        let settled = std::mem::take(&mut self.settled);
        let listed: HashSet<&str> = listed
            .into_iter()
            .filter(|id| !settled.contains(*id))
            .collect();
        self.approvals
            .retain(|pending| listed.contains(pending.id.as_str()));
        self.deciding
            .retain(|pending| listed.contains(pending.id.as_str()));
        for permission in permissions {
            if !settled.contains(&permission.id) {
                self.apply(Event::PermissionRequested(permission));
            }
        }
        let listed: HashSet<&str> = questions
            .iter()
            .map(|question| question.id.as_str())
            .collect();
        let listed: HashSet<&str> = listed
            .into_iter()
            .filter(|id| !settled.contains(*id))
            .collect();
        self.questions
            .retain(|pending| listed.contains(pending.question.id.as_str()));
        self.submitting
            .retain(|pending| listed.contains(pending.question.id.as_str()));
        for question in questions {
            if !settled.contains(&question.id) {
                self.apply(Event::QuestionAsked(question));
            }
        }
        if let Some(reason) = self.reconnecting.take()
            && self.error.as_deref() == Some(reason.as_str())
        {
            self.error = None;
        }
    }

    pub fn activity(&self) -> Activity {
        if self.closed {
            return Activity::Closed;
        }
        if self.is_waiting() {
            return Activity::Waiting;
        }
        match self.busy {
            Some(true) => Activity::Running,
            Some(false) => Activity::Idle,
            None => Activity::Unknown,
        }
    }

    /// Whether a permission or question is pending, including one whose decision or answers are still being delivered.
    fn is_waiting(&self) -> bool {
        !self.approvals.is_empty()
            || !self.deciding.is_empty()
            || !self.questions.is_empty()
            || !self.submitting.is_empty()
    }

    /// Whether `message` is one of a user's, rather than the agent's, as its start said.
    pub fn is_user_message(&self, message: &str) -> bool {
        self.user_messages.contains(message)
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// What about the space needs a participant, for anyone: a pending approval or question first, then what
    /// happened while nobody was in it. See `design/spaces.md`.
    pub fn attention(&self) -> Option<Attention> {
        if !self.closed && self.is_waiting() {
            Some(Attention::Input)
        } else {
            self.unattended
        }
    }

    /// Someone joined the space, so whatever happened while nobody was in it has been seen.
    pub fn attend(&mut self) {
        self.unattended = None;
        self.unheard = false;
    }

    /// The agent made a request, `what`, whose identifier is longer than any a client may name in a reply. Nobody
    /// could answer it here, and presenting it could exceed the snapshot budget, so it is not presented; the space says
    /// so instead. See `design/server.md`.
    fn refuse_request(&mut self, what: &str) {
        self.error = Some(format!(
            "The agent asked {what} with an identifier too long for Tomori to answer; answer it in the agent's own \
             interface."
        ));
        self.unattended_problem();
    }

    /// Something failed while nobody was in the space.
    fn unattended_problem(&mut self) {
        if self.participants.is_empty() {
            self.unattended = Some(Attention::Problem);
        }
    }

    /// The agent's turn ended; it counts as a reply nobody heard if it completed one while nobody was in the space.
    fn turn_ended(&mut self) {
        self.quiet_since = Some(std::time::Instant::now());
        self.turns += 1;
        if std::mem::take(&mut self.unheard) && self.unattended.is_none() {
            self.unattended = Some(Attention::Reply);
        }
    }

    /// How many of the agent's turns have ended: the same number throughout a turn, so that what is decided once in a
    /// turn, such as the agent's ambient list of an environment's objects (see [`crate::ambient`]), stays as it was.
    pub fn turns(&self) -> u64 {
        self.turns
    }

    /// Whether the backend reported reconnecting and has not yet read the session again.
    pub fn is_reconnecting(&self) -> bool {
        self.reconnecting.is_some()
    }

    /// Remember a request settled while reconnecting; see `settled`.
    fn settle(&mut self, id: &str) {
        // Bounded by the requests the space holds, which are bounded themselves.
        if self.reconnecting.is_some() {
            self.settled.insert(id.to_owned());
        }
    }

    pub fn set_attribution(&mut self, enabled: bool) {
        self.attribution = enabled;
    }

    pub fn attribution(&self) -> bool {
        self.attribution
    }

    /// Why the space closed, once it has.
    pub fn closed_reason(&self) -> Option<&str> {
        if self.closed {
            self.error.as_deref()
        } else {
            None
        }
    }

    /// The space's workers, as everyone in it sees them, oldest first.
    pub fn workers(&self) -> &[tomori_protocol::Worker] {
        &self.workers
    }

    /// Whether a worker missing from [`Self::workers`] is known to have left, and why the list may be incomplete.
    pub fn workers_complete(&self) -> (bool, Option<&str>) {
        (self.workers_authoritative, self.worker_error.as_deref())
    }

    /// When this space saw the worker `id` finish, in seconds since the epoch, if it did and it is still finished.
    pub fn worker_finished_at(&self, id: &str) -> Option<u64> {
        self.finished.get(id).copied()
    }

    /// Take the workers `removed` out of the space, as removed on its agent's behalf, before the backend reports them
    /// gone. See `design/mcp.md`.
    pub fn remove_workers(&mut self, removed: &HashSet<String>) {
        self.workers.retain(|worker| !removed.contains(&worker.id));
        self.finished.retain(|id, _| !removed.contains(id));
        self.watch.judge(&self.workers);
    }

    /// What is wrong with the worker `id`, as the space's watch over its workers last judged it, if anything is. See
    /// `design/events.md`.
    pub fn worker_condition(&self, id: &str) -> Option<&crate::watch::Condition> {
        self.watch.condition(id)
    }

    /// `worker` as everyone in the space sees it, stalled or unanswered while it is. See `design/events.md`.
    pub fn shown_worker(&self, worker: &tomori_protocol::Worker) -> tomori_protocol::Worker {
        self.watch.shown(worker)
    }

    /// Whether the space watches any worker, which the clock then judges.
    pub fn watching_workers(&self) -> bool {
        self.watch.watching()
    }

    /// Judge the space's workers at `now`, in milliseconds since the Unix epoch, with `monotonic` the monotonic clock
    /// now, by which a sleep or a jump of the clock is noticed, if it is to be. Returns whether how any is shown
    /// changed. See `design/events.md`.
    pub fn tick_workers(&mut self, now: u64, monotonic: Option<std::time::Instant>) -> bool {
        self.watch.tick(now, monotonic, &self.workers)
    }

    /// Replace the space's workers with `workers`, the backend's latest list, noting when each that was not finished has
    /// finished.
    fn replace_workers(&mut self, workers: Vec<tomori_protocol::Worker>) {
        let now = crate::cards::unix_now();
        for worker in &workers {
            if !worker_finished(worker.state) {
                self.finished.remove(&worker.id);
            } else if self
                .workers
                .iter()
                .any(|known| known.id == worker.id && !worker_finished(known.state))
            {
                self.finished.insert(worker.id.clone(), now);
            }
        }
        self.workers = oldest_first(&self.workers, workers);
        let workers = &self.workers;
        self.finished
            .retain(|id, _| workers.iter().any(|worker| &worker.id == id));
    }

    /// Bind a space listed again after a restart, which waits as reconnecting, to its session as the backend read it: like reading it again after a gap, but with the workers it reports, since none of their events were seen. See `design/spaces.md`.
    pub fn bind(&mut self, session: String, snapshot: Snapshot) {
        self.workers = snapshot
            .workers
            .iter()
            .cloned()
            .map(|entry| worker(entry, &session))
            .collect();
        self.finished.clear();
        self.workers_authoritative = snapshot.workers_incomplete.is_none();
        self.worker_error.clone_from(&snapshot.workers_incomplete);
        self.session = session;
        // The session's history, read again after a restart, is not news: what happened while the server was stopped
        // is not known, so it raises no problem or reply. A pending request still waits for input.
        let (unattended, unheard) = (self.unattended, self.unheard);
        self.resynchronize(snapshot);
        (self.unattended, self.unheard) = (unattended, unheard);
        // Nor is it projected again: its panels were projected when it was written.
        self.projected.clear();
    }

    /// The space's own instructions are `space`, and its backend takes instructions if `takes`. A space whose backend
    /// is not enabled keeps them, for when it is.
    pub fn set_instructions(&mut self, space: String, takes: bool) {
        self.instructions = space;
        self.takes_instructions = takes;
    }

    /// The space's own instructions, empty when it has none.
    pub fn space_instructions(&self) -> &str {
        &self.instructions
    }

    /// Whether the space's backend takes instructions.
    pub fn takes_instructions(&self) -> bool {
        self.takes_instructions
    }

    /// The instruction layers in effect, with `environment` its environment's layer, or `None` when the backend takes none. The space keeps only its environment's identifier, and the caller looks up the layer in the environments the server offers now, so that a space is given the same layer however and whenever it was bound: created, restored or changed. See `design/environments.md`.
    pub fn instructions(&self, environment: &str) -> Option<tomori_protocol::AgentInstructions> {
        self.takes_instructions.then(|| {
            crate::instructions::layers(
                environment,
                &self.instructions,
                self.instructions_error.clone(),
            )
        })
    }

    /// The entries giving the agent every layer, with `environment` its environment's layer as in [`Self::instructions`], or none when its backend takes none or the session has closed.
    pub fn instruction_entries(&self, environment: &str) -> Vec<(String, Option<String>)> {
        match self.instructions(environment) {
            Some(layers) if !self.closed => crate::instructions::entries(&layers),
            _ => Vec::new(),
        }
    }

    /// Replace the space's own instructions with `text`, which is trimmed, or say why not.
    pub fn set_space_instructions(&mut self, text: &str) -> Result<(), &'static str> {
        if self.closed {
            return Err("The agent session has stopped.");
        }
        if !self.takes_instructions {
            return Err("This agent takes no instructions from Tomori.");
        }
        self.instructions = text.trim().to_owned();
        Ok(())
    }

    /// Whether the agent has refused instructions since it last took every layer.
    pub fn instructions_failed(&self) -> bool {
        self.instructions_error.is_some()
    }

    /// The agent was given instructions, every layer if `all`, or refused them with `error`, which is shown until it
    /// next takes every layer: taking one alone, or removing one, does not show that it has the others. The space works
    /// without them, so a refusal is not a problem that needs attention.
    pub fn instructed(&mut self, result: Result<(), String>, all: bool) {
        match result {
            Err(error) => {
                self.instructions_error = Some(limits::truncate(
                    &format!("The agent did not take Tomori's instructions: {error}"),
                    limits::ERROR_BYTES,
                ));
            }
            Ok(()) if all => self.instructions_error = None,
            Ok(()) => {}
        }
    }

    /// Take `directory`, which the backend reports as the one the session is in now, as the space's, as the agent
    /// names it, so that Tomori's tools are registered where the agent looks for them. A directory that is empty, not
    /// absolute, or longer than [`limits::DIRECTORY_BYTES`] is ignored. See `design/mcp.md`.
    pub fn follow_directory(&mut self, directory: &str) {
        if followed(directory) {
            directory.clone_into(&mut self.directory);
        }
    }

    /// Take `title`, which the backend reports as the session's own, as the space's title too, unless someone renamed
    /// the space. A title with nothing left of it once cleaned is ignored. See `design/spaces.md`.
    fn follow(&mut self, title: &str) {
        let Some(title) = limits::session_title(title) else {
            return;
        };
        if self.renamed_by.is_none() {
            self.title.clone_from(&title);
        }
        self.session_title = Some(title);
    }

    /// Give the space `title`, already cleaned by [`limits::title`], as `by` asked, in place of its session's, which it
    /// no longer follows; or, when `title` is empty, let it follow its session's title again, taking it now if the
    /// backend has reported it, and keeping the title it has otherwise. Returns whether the title shown changed. See
    /// `design/spaces.md`.
    pub fn rename(&mut self, title: String, by: tomori_protocol::Renamer) -> bool {
        let before = std::mem::take(&mut self.title);
        if title.is_empty() {
            self.renamed_by = None;
            self.title = self.session_title.clone().unwrap_or_else(|| before.clone());
        } else {
            self.renamed_by = Some(by);
            self.title = title;
        }
        self.title != before
    }

    /// Who gave the space its title in place of its session's, if anyone did.
    pub fn renamed_by(&self) -> Option<&tomori_protocol::Renamer> {
        self.renamed_by.as_ref()
    }

    /// Restore that `by` gave the space its title, as kept across a restart, so that it does not follow its session.
    pub fn keep_title(&mut self, by: Option<tomori_protocol::Renamer>) {
        self.renamed_by = by;
    }

    /// How much of each reply is spoken. Full unless a participant chose brief.
    pub fn speech(&self) -> SpeechSetting {
        self.speech
    }

    /// Bind the space to `environment`, which the caller has checked the server offers, for everyone in it. Returns
    /// whether it changed. A closed space keeps the environment it had. See `design/environments.md`.
    pub fn set_environment(&mut self, environment: String) -> Result<bool, &'static str> {
        if self.closed {
            return Err("The agent session has stopped.");
        }
        if self.environment == environment {
            return Ok(false);
        }
        self.environment = environment;
        // The failure was of the environment it left.
        self.environment_error = None;
        Ok(true)
    }

    /// Show that the environment's server side failed, for as long as the space stays in the environment.
    pub fn fail_environment(&mut self, message: String) {
        self.environment_error = Some(message);
    }

    /// Change how much of each reply is spoken, from the next reply on. An unknown setting is refused.
    pub fn set_speech(&mut self, speech: SpeechSetting) -> Result<(), &'static str> {
        match speech {
            SpeechSetting::Full | SpeechSetting::Brief => {
                self.speech = speech;
                Ok(())
            }
            SpeechSetting::Unknown => Err("Tomori does not know that speech setting."),
        }
    }

    /// How the agent moves about the space. See `design/environments.md`.
    pub fn movement(&self) -> tomori_protocol::AgentMovement {
        self.movement
    }

    /// Change how the agent moves about the space, its delay brought within bounds; whether it changed. An unknown
    /// mode is refused.
    pub fn set_movement(
        &mut self,
        movement: tomori_protocol::AgentMovement,
    ) -> Result<bool, &'static str> {
        if movement.mode == tomori_protocol::Movement::Unknown {
            return Err("Tomori does not know that way for the agent to move.");
        }
        let movement = tomori_protocol::AgentMovement::new(movement.mode, movement.return_after);
        let changed = self.movement != movement;
        self.movement = movement;
        Ok(changed)
    }

    /// Whether `mode` is one the space shows as offered, so a participant may choose it.
    pub fn offers_mode(&self, mode: &str) -> bool {
        self.shown_modes().iter().any(|offered| offered.id == mode)
    }

    /// The offered modes as shared, bounded so that a misbehaving agent cannot fill the snapshot: a mode whose
    /// identifier no client could send back is left out, and names and descriptions are cut.
    fn shown_modes(&self) -> Vec<tomori_protocol::Mode> {
        self.modes
            .offered
            .iter()
            .filter(|mode| mode.id.len() <= limits::IDENTIFIER_BYTES)
            .take(limits::MODES)
            .map(|mode| tomori_protocol::Mode {
                id: mode.id.clone(),
                name: limits::truncate(&mode.name, limits::MODE_NAME_BYTES),
                description: mode.description.as_deref().map(|description| {
                    limits::truncate(description, limits::MODE_DESCRIPTION_BYTES)
                }),
            })
            .collect()
    }

    /// The name the space shows for `mode`, falling back to its identifier.
    fn mode_name(&self, mode: &str) -> String {
        let name = self
            .modes
            .offered
            .iter()
            .find(|offered| offered.id == mode)
            .map_or(mode, |offered| offered.name.as_str());
        limits::truncate(name, limits::MODE_NAME_BYTES)
    }

    /// Add a message the space has not seen before, after every message it has seen.
    fn push_message(&mut self, mut message: MessageModel) {
        self.last_order += 1;
        message.order = self.last_order;
        self.keep_message(message);
    }

    /// Add `message` to the conversation, in the place it already has.
    fn keep_message(&mut self, message: MessageModel) {
        self.messages.push(message);
        if self.messages.len() > MESSAGE_LIMIT {
            let excess = self.messages.len() - MESSAGE_LIMIT;
            self.messages.drain(..excess);
        }
    }

    /// The message with this identifier, created as an assistant message if it is new.
    fn message(&mut self, id: &str) -> &mut MessageModel {
        let index = match self.messages.iter().rposition(|message| message.id == id) {
            Some(index) => index,
            None => {
                self.push_message(MessageModel {
                    id: id.to_owned(),
                    role: tomori_protocol::Role::Assistant,
                    text: String::new(),
                    tools: Vec::new(),
                    completed: false,
                    order: 0,
                    completed_at: None,
                });
                self.messages.len() - 1
            }
        };
        &mut self.messages[index]
    }

    /// Apply a backend event to the shared state.
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::Busy => self.busy = Some(true),
            Event::Idle => {
                self.busy = Some(false);
                self.turn_ended();
            }
            Event::Delivered { key } => {
                // Input Tomori did not give, such as a message typed into the session outside Tomori or a sub-agent's
                // report, means the participant's words are no longer the agent's latest input.
                if !self
                    .given_keys
                    .iter()
                    .any(|given| given == delivery_key(&key))
                {
                    self.supersede(Superseded::Input);
                }
                self.delivered(&key)
            }
            Event::MessageStarted { message, role } => match role {
                tomori_backend::Role::User => {
                    self.user_messages.insert(message);
                }
                tomori_backend::Role::Assistant => {
                    self.message(&message);
                }
            },
            Event::TextDelta { message, text } => {
                if !self.user_messages.contains(&message) {
                    self.message(&message).text.push_str(&text);
                }
            }
            Event::MessageCompleted { message } => {
                if !self.user_messages.remove(&message) {
                    let completed = self.message(&message);
                    completed.completed = true;
                    completed
                        .completed_at
                        .get_or_insert_with(crate::cards::unix_now);
                    // A message of tool calls alone is a step of the turn, not a reply.
                    let replied = is_reply(completed);
                    if replied && self.participants.is_empty() {
                        self.unheard = true;
                    }
                    let index = self
                        .messages
                        .iter()
                        .rposition(|shown| shown.id == message)
                        .expect("found or added just above");
                    self.project_reply(index);
                }
            }
            Event::ToolStarted {
                message, tool, id, ..
            } => {
                // A start the session was re-read with is not added again.
                let tools = &mut self.message(&message).tools;
                if !tools.iter().any(|known| known.id == id) {
                    tools.push(ToolModel {
                        id,
                        name: tool,
                        state: "working",
                    });
                }
            }
            Event::ToolFinished { id, error } => {
                if let Some(tool) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .flat_map(|message| message.tools.iter_mut())
                    .find(|tool| tool.id == id)
                {
                    tool.state = if error.is_some() { "failed" } else { "done" };
                }
            }
            Event::PermissionRequested(permission)
                if permission.id.len() > limits::IDENTIFIER_BYTES =>
            {
                self.refuse_request("for an approval");
            }
            Event::PermissionRequested(permission) => {
                // Parallel tool calls produce several requests; each waits its turn.
                let known = self
                    .approvals
                    .iter()
                    .chain(&self.deciding)
                    .any(|pending| pending.id == permission.id);
                if !known {
                    self.approvals.push_back(permission);
                }
            }
            Event::PermissionResolved { id } => {
                self.approvals.retain(|pending| pending.id != id);
                self.deciding.retain(|pending| pending.id != id);
            }
            Event::QuestionAsked(question) if question.id.len() > limits::IDENTIFIER_BYTES => {
                self.refuse_request("a question");
            }
            Event::QuestionAsked(question) => {
                let known = self
                    .questions
                    .iter()
                    .chain(&self.submitting)
                    .any(|pending| pending.question.id == question.id);
                if !known {
                    self.questions.push_back(PendingQuestion {
                        question,
                        answers: Vec::new(),
                    });
                }
            }
            Event::QuestionResolved { id } => {
                self.questions.retain(|pending| pending.question.id != id);
                self.submitting.retain(|pending| pending.question.id != id);
            }
            Event::WorkersChanged { workers } => {
                let workers = workers
                    .into_iter()
                    .map(|entry| worker(entry, &self.session))
                    .collect();
                self.replace_workers(workers);
                self.workers_authoritative = true;
                self.worker_error = None;
                self.watch.retain(&self.workers);
                self.watch.judge(&self.workers);
            }
            Event::WorkersPartial { workers, message } => {
                let workers = workers
                    .into_iter()
                    .map(|entry| worker(entry, &self.session))
                    .collect();
                self.replace_workers(workers);
                self.workers_authoritative = false;
                self.worker_error = Some(message);
                self.watch.judge(&self.workers);
            }
            Event::WorkerActivity { worker, activity } => {
                self.watch.observe(worker, activity);
                self.watch.judge(&self.workers);
            }
            Event::WorkersUnavailable { message } => {
                self.worker_error = Some(message);
                self.workers_authoritative = false;
            }
            Event::ModesChanged(modes) => self.modes = modes,
            Event::ModeRefused { mode, reason } => {
                let name = self.mode_name(&mode);
                self.error = Some(match reason {
                    Some(reason) => format!("The agent refused {name} mode: {reason}"),
                    None => format!("The agent refused {name} mode."),
                });
                self.unattended_problem();
            }
            Event::Error { message } => {
                self.error = Some(message);
                self.unattended_problem();
            }
            Event::Reconnecting { reason } => {
                // What the workers do is unknown until the backend reads them again. See `design/events.md`.
                self.watch.unknown();
                // Whatever the session is given while Tomori cannot see it may not be a participant's.
                self.supersede(Superseded::Reconnected);
                if !self.closed {
                    self.error = Some(reason.clone());
                    self.reconnecting = Some(reason);
                    self.settled.clear();
                }
            }
            Event::MessageRestored { message } => {
                if let Some(index) = self
                    .messages
                    .iter()
                    .rposition(|shown| shown.id == message.id)
                {
                    let mut restored = message_model(message);
                    let shown = &self.messages[index];
                    let was_reply = is_reply(shown);
                    // Its text may be recorded before the message is marked completed, and a record does not say
                    // which tools failed: what the space already knows is kept.
                    restored.completed |= shown.completed;
                    for tool in &mut restored.tools {
                        if shown
                            .tools
                            .iter()
                            .any(|known| known.id == tool.id && known.state == "failed")
                        {
                            tool.state = "failed";
                        }
                    }
                    // It keeps its place, and when it completed, or completes now, for all the space knew.
                    restored.order = shown.order;
                    restored.completed_at = shown
                        .completed_at
                        .or_else(|| restored.completed.then(crate::cards::unix_now));
                    // Its end, and so a reply, may only now be known, after a gap that lost its events.
                    let replied = !was_reply && is_reply(&restored);
                    self.messages[index] = restored;
                    if replied {
                        self.project_reply(index);
                    }
                    if replied && self.participants.is_empty() {
                        self.unheard = true;
                        if self.busy == Some(false) {
                            self.turn_ended();
                        }
                    }
                }
            }
            Event::Resynchronized { snapshot } => {
                self.supersede(Superseded::Reconnected);
                if !self.closed {
                    self.resynchronize(snapshot);
                }
            }
            Event::TitleChanged { title } => {
                if !self.closed {
                    self.follow(&title);
                }
            }
            Event::DirectoryChanged { directory } => {
                if !self.closed {
                    self.follow_directory(&directory);
                }
            }
            Event::Closed { reason } => {
                if !self.closed {
                    self.unattended_problem();
                }
                self.closed = true;
                self.reconnecting = None;
                self.settled.clear();
                self.workers_authoritative = false;
                self.worker_error = None;
                self.busy = None;
                self.error = Some(reason.unwrap_or_else(|| "The agent session ended.".to_owned()));
                if let Some(prompt) = self.in_flight.take() {
                    self.queue.finish(&prompt.key, QueueState::Failed);
                }
                // Nothing waiting can be delivered any more, so say so rather than leave it queued.
                self.queue.fail_waiting();
                self.attending.clear();
                self.origins.clear();
                self.seen.clear();
                // A closed session can no longer receive decisions or answers.
                self.approvals.clear();
                self.deciding.clear();
                self.questions.clear();
                self.submitting.clear();
                // The agent process has ended, so nothing it was running is running any more, whatever the backend
                // last reported.
                let now = crate::cards::unix_now();
                for worker in &mut self.workers {
                    if matches!(
                        worker.state,
                        tomori_protocol::WorkerState::Running
                            | tomori_protocol::WorkerState::Waiting
                    ) {
                        worker.state = tomori_protocol::WorkerState::Stopped;
                        self.finished.insert(worker.id.clone(), now);
                    }
                }
            }
        }
    }

    /// The text queued under `key`, if it was queued recently; see [`InputQueue::queued_text`].
    pub fn queued_text(&self, key: &str) -> Option<&str> {
        self.queue.queued_text(key)
    }

    /// Queue finalized input from the participant numbered `source`, shown as spoken by `speaker`. Returns false if this key was already queued.
    #[cfg(test)]
    pub fn enqueue(&mut self, key: String, source: u32, speaker: String, text: String) -> bool {
        self.enqueue_attending(key, source, speaker, text, Vec::new())
    }

    /// The resource `resource` identifies, as the space knows it now, for telling the agent that a participant
    /// attended to it; `None` if the space does not have it, whatever a client claims, for an oversized identity, or
    /// for a kind this server does not know. A card is looked up on `cards`, the board, given only for a space that
    /// shows it, and described as it is now.
    #[cfg(test)]
    pub fn attended(
        &self,
        resource: &tomori_protocol::Resource,
        cards: Option<&crate::cards::Board>,
        objects: Option<&crate::channel::Channel>,
    ) -> Option<Attended> {
        self.attended_with(resource, cards, objects, &|_, _| None, &|_| None)
    }

    /// The resource `resource` identifies, as [`Self::attended`] says, with a panel described by `panels`, the stage's
    /// description of the panel it has with that identity, if any, and a cluster's "+N more" sign by the workers
    /// `gathered` says it gathers.
    pub fn attended_with(
        &self,
        resource: &tomori_protocol::Resource,
        cards: Option<&crate::cards::Board>,
        objects: Option<&crate::channel::Channel>,
        panels: &DescribePanel<'_>,
        gathered: &Gathered<'_>,
    ) -> Option<Attended> {
        match resource {
            tomori_protocol::Resource::Worker { worker, .. }
                if worker.len() <= limits::SESSION_BYTES =>
            {
                self.workers
                    .iter()
                    .find(|known| &known.id == worker)
                    .map(|known| {
                        Attended(Resolved::Worker {
                            id: known.id.clone(),
                            title: known.title.clone(),
                        })
                    })
            }
            // A card's identity is only looked up, never told to the agent: the board describes the card it has.
            tomori_protocol::Resource::Card { card, .. }
                if card.len() <= limits::IDENTIFIER_BYTES =>
            {
                let description = cards?.describe(card, crate::cards::unix_now())?;
                Some(Attended(Resolved::Card {
                    id: card.clone(),
                    description,
                }))
            }
            // An object only of the environment the space is in now, as its side described it.
            tomori_protocol::Resource::Object { object, .. }
                if object.len() <= tomori_environment::IDENTITY_LIMIT =>
            {
                let channel = objects.filter(|channel| channel.environment == self.environment)?;
                let described = channel.describe(object)?;
                Some(Attended(Resolved::Object {
                    id: object.clone(),
                    description: described.description.clone(),
                    reference: described
                        .reference
                        .as_ref()
                        .map(|reference| inspectable(&channel.environment, reference)),
                    offers: offered_names(&channel.offered(described)),
                }))
            }
            // A panel's identity is only looked up: the stage describes the panel it has.
            // A list's row is named by its id, which is dropped where it is longer than any, and the line taken alone.
            tomori_protocol::Resource::Panel {
                panel,
                line,
                row,
                across,
                node,
                edge,
                world_edge,
                chip,
                ..
            } if panel.len() <= limits::IDENTIFIER_BYTES => {
                let row = row
                    .as_deref()
                    .filter(|row| row.len() <= tomori_environment::ROW_ID_BYTES);
                Some(Attended(Resolved::Panel {
                    id: panel.clone(),
                    description: panels(
                        panel,
                        crate::stage::Pointed {
                            line: *line,
                            row,
                            across: *across,
                            // A node is named by its id, which is dropped where it is longer than any.
                            node: node
                                .as_deref()
                                .filter(|node| node.len() <= tomori_protocol::diagram::ID_BYTES),
                            edge: *edge,
                            world_edge: *world_edge,
                            // A chip is named by its parameter, which is dropped where it is longer than any.
                            chip: chip
                                .as_deref()
                                .filter(|chip| chip.len() <= tomori_environment::CONTEXT_ID_BYTES),
                        },
                    )?,
                }))
            }
            // Its workers are those the space's own layout gathers into it, each as the space knows it now.
            tomori_protocol::Resource::Gathered {
                gathered: anchor, ..
            } if anchor.len() <= tomori_protocol::cluster::NAME_BYTES => {
                let workers = gathered(anchor)?
                    .into_iter()
                    .filter_map(|session| {
                        self.workers
                            .iter()
                            .find(|known| known.id == session)
                            .map(|known| (known.id.clone(), known.title.clone()))
                    })
                    .collect::<Vec<_>>();
                (!workers.is_empty()).then(|| {
                    Attended(Resolved::Gathered {
                        anchor: anchor.clone(),
                        workers,
                    })
                })
            }
            // A timer, by its name, as the space's own timers describe it now.
            tomori_protocol::Resource::Timer { timer, .. } => {
                self.timers.describe(timer).map(|description| {
                    Attended(Resolved::Timer {
                        name: timer.clone(),
                        description,
                    })
                })
            }
            _ => None,
        }
    }

    /// What `attention` identifies, as [`Self::attended`] resolves each of its first
    /// [`tomori_protocol::ATTENTION_LIMIT`] resources, cards on `cards`, without those the space does not have or
    /// repeats.
    #[cfg(test)]
    pub fn resolve(
        &self,
        attention: &[tomori_protocol::Resource],
        cards: Option<&crate::cards::Board>,
        objects: Option<&crate::channel::Channel>,
    ) -> Vec<Attended> {
        self.resolve_with(attention, cards, objects, &|_, _| None, &|_| None)
    }

    /// What `attention` identifies, as [`Self::resolve`] says, with panels described by `panels`, and "+N more" signs
    /// by what `gathered` says they gather, as in [`Self::attended_with`].
    pub fn resolve_with(
        &self,
        attention: &[tomori_protocol::Resource],
        cards: Option<&crate::cards::Board>,
        objects: Option<&crate::channel::Channel>,
        panels: &DescribePanel<'_>,
        gathered: &Gathered<'_>,
    ) -> Vec<Attended> {
        let mut resolved: Vec<Attended> = Vec::new();
        for attended in attention
            .iter()
            .take(tomori_protocol::ATTENTION_LIMIT)
            .filter_map(|resource| self.attended_with(resource, cards, objects, panels, gathered))
        {
            if !resolved.iter().any(|known| known.is(&attended)) {
                resolved.push(attended);
            }
        }
        resolved
    }

    /// Queue input as [`Self::enqueue`] does, from a speaker who attended to `attention` as they began speaking. The
    /// queue shows only the words; the agent is told about what they attended to before them, as the space knows it
    /// when they are delivered.
    pub fn enqueue_attending(
        &mut self,
        key: String,
        source: u32,
        speaker: String,
        text: String,
        attention: Vec<Attended>,
    ) -> bool {
        if !self.queue.push(key.clone(), source, speaker, text) {
            return false;
        }
        if !attention.is_empty() {
            self.attending.insert(key, attention);
        }
        true
    }

    /// Entries waiting to be delivered.
    pub fn waiting(&self) -> usize {
        self.queue.waiting()
    }

    pub fn cancel(&mut self, key: &str) -> Result<(), &'static str> {
        self.queue.cancel(key)?;
        self.attending.remove(key);
        self.origins.remove(key);
        self.seen.remove(key);
        Ok(())
    }

    /// The speaker of the waiting entry `key`'s head was at `origin`, facing as it faces, as they began speaking, at
    /// `origin`'s time, so that what the agent projects in answer goes where they were looking, or beside them. See
    /// `design/projection.md`.
    pub fn originate(&mut self, key: &str, origin: Origin) {
        if self
            .queue
            .entries()
            .iter()
            .any(|entry| entry.id == key && entry.state == QueueState::Queued)
        {
            self.origins.insert(key.to_owned(), origin);
        }
    }

    /// What a participant had seen of the agent's messages as they begin speaking, as the space knows it now: the
    /// message `message` names, which their client says is the newest of the agent's it had shown or played them, if
    /// the space has it as the agent's, and otherwise the newest of the agent's that says anything. Only an identity
    /// is taken from the client; the space describes the message from its own conversation. See
    /// `design/interaction.md`.
    pub fn seen(&self, message: Option<&str>) -> Seen {
        let named = message.and_then(|id| {
            self.messages
                .iter()
                .rposition(|known| known.id == id && known.role == tomori_protocol::Role::Assistant)
        });
        let Some(index) = named.or_else(|| self.messages.iter().rposition(says_something)) else {
            // Nothing the agent said yet: anything it says says more.
            return Seen {
                order: 0,
                earlier: None,
            };
        };
        let message = &self.messages[index];
        Seen {
            order: message.order,
            earlier: Some(Earlier {
                id: message.id.clone(),
                ending: ending(&message.text),
                completed_at: message.completed_at,
            }),
        }
    }

    /// Say that the message `id` completed `seconds` ago, for tests of how long ago the agent is told it was.
    #[cfg(test)]
    fn completed_ago(&mut self, id: &str, seconds: u64) {
        let message = self
            .messages
            .iter_mut()
            .find(|message| message.id == id)
            .expect("a message the conversation keeps");
        message.completed_at = Some(crate::cards::unix_now() - seconds);
    }

    /// The speaker of the waiting entry `key` had `seen` the agent's messages as they began speaking, where the space
    /// knew, so that, if it says more before their words are delivered, it is told which message they answer.
    pub fn saw(&mut self, key: &str, seen: Option<Seen>) {
        if let Some(seen) = seen
            && self
                .queue
                .entries()
                .iter()
                .any(|entry| entry.id == key && entry.state == QueueState::Queued)
        {
            self.seen.insert(key.to_owned(), seen);
        }
    }

    /// The panels of each reply completed since this was last called, with who stood behind it as it completed, for
    /// the replies that have any. See `design/projection.md`.
    pub fn take_projected(&mut self) -> Vec<(Vec<crate::stage::Found>, Asked)> {
        std::mem::take(&mut self.projected)
    }

    /// Who stands behind what the agent does now: the speaker of the entry given last, whether its turn still answers
    /// their words, and, while it does, where they stood as they began speaking, if the space knows.
    pub fn asked(&self) -> Asked {
        let answering = self
            .given
            .as_ref()
            .is_some_and(|given| given.superseded.is_none());
        Asked {
            source: self.given.as_ref().map(|given| given.source),
            answering,
            origin: self.origin.filter(|_| answering),
        }
    }

    /// Have the words given last have begun `ago` earlier than they did, as a test does rather than wait.
    #[cfg(test)]
    pub fn age_origin(&mut self, ago: std::time::Duration) {
        let origin = self
            .origin
            .as_mut()
            .expect("words given where their speaker began");
        origin.at -= ago;
    }

    /// Lift the panel blocks out of the reply at `index`, once, for the hub to project.
    fn project_reply(&mut self, index: usize) {
        let message = &self.messages[index];
        if !is_reply(message) || self.projected_replies.contains(&message.id) {
            return;
        }
        if self.projected_replies.len() == PROJECTED_REPLIES {
            self.projected_replies.pop_front();
        }
        self.projected_replies.push_back(message.id.clone());
        let panels = crate::stage::panels(&message.text);
        if !panels.is_empty() {
            let asked = self.asked();
            self.projected.push((panels, asked));
        }
    }

    /// A participant ticked or unticked the checklist item on `line` of the panel `panel`, which the agent is told, as
    /// `note` says, before the next words spoken in the space, rather than in a turn of its own, so that it follows
    /// along without replying to every tick; `panels` shows it at once. A later change to the same item replaces it, and
    /// only the latest [`NOTES`] are kept. See `design/projection.md`.
    pub fn note_tick(&mut self, panel: &str, line: u32, note: String) {
        self.notes
            .retain(|(noted, at, _)| !(noted == panel && *at == line));
        if self.notes.len() == NOTES {
            self.notes.pop_front();
        }
        self.notes.push_back((panel.to_owned(), line, note));
    }

    /// What the agent is told of a panel it projected, as `note` says, before the next words spoken in the space, as of a
    /// tick: a later note of the same panel replaces it. See `design/diagrams.md`.
    pub fn note_panel(&mut self, panel: &str, note: String) {
        self.note_tick(panel, u32::MAX, note);
    }

    /// The notes waiting for the next words, for tests.
    #[cfg(test)]
    pub fn notes(&self) -> Vec<&str> {
        self.notes
            .iter()
            .map(|(_, _, note)| note.as_str())
            .collect()
    }

    /// The next prompt to deliver, if the agent is idle, nothing is pending and input is queued. Marks the entry as sending.
    #[cfg(test)]
    pub fn next_prompt(&mut self) -> Option<Prompt> {
        self.next_prompt_at(std::time::Instant::now(), false)
    }

    /// The next prompt to deliver at `now`, as [`Self::next_prompt`] says: queued input first, with Tomori's notes
    /// waiting before it, or else a prompt of the notes alone, once the event queue says it may be given and unless
    /// `hold`, as while someone in the space is speaking, whose words will take them. See `design/timers.md`.
    pub fn next_prompt_at(&mut self, now: std::time::Instant, hold: bool) -> Option<Prompt> {
        if self.closed
            || self.reconnecting.is_some()
            || self.activity() != Activity::Idle
            || self.in_flight.is_some()
        {
            return None;
        }
        let attribution = self.attribution;
        let queued = self
            .queue
            .entries()
            .iter()
            .any(|entry| entry.state == QueueState::Queued);
        // Notes about workers join the queue only as they can be told, so that each is judged again just before.
        if self.watch.due() && (queued || (!hold && self.events.may_give(now, self.quiet_since))) {
            let present = !self.participants.is_empty();
            for (key, note) in self.watch.take(&self.backend, &self.workers, present) {
                self.events.push(crate::events::Source::Worker, key, note);
            }
        }
        if !queued {
            return self.event_prompt(now, hold);
        }
        let events = self.events.block();
        let carried = events.is_some();
        // An entry is sending only while it is in flight, which nothing is.
        let entry = self.queue.begin_next().expect("an entry is queued");
        let key = entry.id.clone();
        let (source, speaker) = (entry.source, entry.speaker.clone());
        // What the speaker attended to comes before their words, described as the space knows it now.
        let context = self.attending.remove(&key).and_then(|attention| {
            // As everyone sees the workers, stalled or unanswered while they are.
            let shown: Vec<tomori_protocol::Worker> = self
                .workers
                .iter()
                .map(|worker| self.watch.shown(worker))
                .collect();
            context(&self.backend, &attention, &shown)
        });
        // And, before that, which of the agent's messages they answer, should it have said more since they began.
        let reply = self
            .seen
            .remove(&key)
            .and_then(|seen| interleaved(&self.messages, &seen, crate::cards::unix_now()));
        let context = match (reply, context) {
            (Some(reply), Some(context)) => Some(format!("{reply} {context}")),
            (reply, context) => reply.or(context),
        };
        // What participants did since the words before, such as ticking a checklist's item, comes before both.
        let notes = noted(self.notes.iter().map(|(_, _, note)| note.as_str()));
        self.notes.clear();
        // And Tomori's own notes waiting, such as a timer that fired, before those.
        let notes = match (events, notes) {
            (Some(events), Some(notes)) => Some(format!("{events} {notes}")),
            (events, notes) => events.or(notes),
        };
        let text = contextual(&entry.speaker, notes, context, &entry.text, attribution);
        // What the agent projects in its turn goes onto the wall nearest where they stood, or the Tomori wall.
        self.origin = self.origins.remove(&key);
        // Entries that failed or were dropped without being delivered take their attention with them.
        let waiting: HashSet<&str> = self
            .queue
            .entries()
            .iter()
            .filter(|entry| entry.state == QueueState::Queued)
            .map(|entry| entry.id.as_str())
            .collect();
        self.attending
            .retain(|key, _| waiting.contains(key.as_str()));
        self.origins.retain(|key, _| waiting.contains(key.as_str()));
        self.seen.retain(|key, _| waiting.contains(key.as_str()));
        self.remember_given(delivery_key(&key).to_owned());
        let prompt = Prompt { key, text };
        self.in_flight = Some(prompt.clone());
        // Who began the turn, from the moment it is given, since the agent may call a tool before delivery is confirmed.
        self.noted = None;
        self.entries_given += 1;
        self.given = Some(Given {
            entry: self.entries_given,
            source,
            speaker,
            at: std::time::Instant::now(),
            carried,
            superseded: None,
        });
        Some(prompt)
    }

    /// A prompt of Tomori's notes alone, if they may be given at `now` and are not held: the notes, each in its
    /// brackets, with no speaker. No participant asked for it, so the words given last no longer stand behind what the
    /// agent does next. See `design/timers.md`.
    fn event_prompt(&mut self, now: std::time::Instant, hold: bool) -> Option<Prompt> {
        if hold || !self.events.ready(now, self.quiet_since) {
            return None;
        }
        let about = self.events.about().map(str::to_owned);
        let (key, text) = self.events.prompt(now).expect("ready says notes wait");
        self.supersede(Superseded::Event);
        self.noted = about;
        self.remember_given(key.clone());
        self.origin = None;
        let prompt = Prompt { key, text };
        self.in_flight = Some(prompt.clone());
        Some(prompt)
    }

    /// Remember `key` as one Tomori gave the agent, among the latest [`GIVEN_KEYS`], so that its delivery is not taken
    /// for input from elsewhere.
    fn remember_given(&mut self, key: String) {
        if self.given_keys.len() == GIVEN_KEYS {
            self.given_keys.pop_front();
        }
        self.given_keys.push_back(key);
    }

    /// Have the agent's last turn have ended a quiet moment ago, as a test does rather than wait.
    #[cfg(test)]
    pub fn quieten(&mut self) {
        self.quiet_since = self.quiet_since.map(|since| since - crate::events::QUIET);
    }

    /// Have the agent's last turn have ended a quiet moment ago, and its last prompt of notes alone a gap ago, as a test
    /// does rather than wait.
    #[cfg(test)]
    pub fn lapse(&mut self) {
        self.quieten();
        self.events.lapse();
    }

    /// Whether Tomori's notes wait for the agent.
    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    /// The space's timers.
    pub fn timers(&self) -> &crate::timers::Timers {
        &self.timers
    }

    /// The space's timers, to change.
    pub fn timers_mut(&mut self) -> &mut crate::timers::Timers {
        &mut self.timers
    }

    /// Look at the space's timers at `now`, with `present` the participants present, queueing what fires for the
    /// agent. Returns whether how they look changed.
    pub fn tick_timers(&mut self, now: jiff::Timestamp, present: &[String]) -> bool {
        let before = self.timers.views();
        for firing in self.timers.tick(now, present) {
            self.events.push_timed(
                crate::events::Source::Timer,
                &firing.name,
                firing.text,
                firing.on_time,
            );
        }
        self.timers.views() != before
    }

    /// Queue Tomori's note `text`, that a participant came near one of the space's anchors, under `key`, which names
    /// both, for the agent, as a timer's firing waits. See `design/environments.md`.
    pub fn tell_proximity(&mut self, key: &str, about: Option<&str>, text: String) {
        self.events
            .push_about(crate::events::Source::Proximity, key, text, about);
    }

    /// The participant, by their key, that the notes alone which began the agent's latest turn were about, if Tomori's
    /// notes were its latest input and one was about someone: whoever came near an anchor last, of those told. None
    /// once anyone's words, or input from elsewhere, are given it after them. See `design/environments.md`.
    pub fn noted(&self) -> Option<&str> {
        self.noted.as_deref()
    }

    /// The entry given to the agent last, which began its current turn, if it has not failed to be delivered. See
    /// [`Given`].
    pub fn given(&self) -> Option<&Given> {
        self.given.as_ref()
    }

    /// Give the agent an entry from the participant numbered `source`, called `speaker`, at `at`, as delivering their
    /// input does, for tests of who asked.
    #[cfg(test)]
    pub fn give(&mut self, source: u32, speaker: &str, at: std::time::Instant) {
        self.entries_given += 1;
        self.given = Some(Given {
            entry: self.entries_given,
            source,
            speaker: speaker.to_owned(),
            at,
            carried: false,
            superseded: None,
        });
    }

    /// The entry given last is no longer the agent's latest input, for `why`.
    fn supersede(&mut self, why: Superseded) {
        // Nor are the notes given last, if any were.
        self.noted = None;
        if let Some(given) = &mut self.given {
            given.superseded.get_or_insert(why);
        }
    }

    fn delivered(&mut self, key: &str) {
        if !self
            .in_flight
            .as_ref()
            .is_some_and(|prompt| prompt.key == key)
        {
            return;
        }
        let prompt = self.in_flight.take().expect("checked above");
        self.queue.finish(key, QueueState::Delivered);
        // The agent accepted the prompt, so its turn has started even if `Busy` has not arrived yet.
        self.busy = Some(true);
        self.push_message(MessageModel {
            id: prompt.key,
            role: tomori_protocol::Role::User,
            text: prompt.text,
            tools: Vec::new(),
            completed: true,
            order: 0,
            completed_at: None,
        });
    }

    /// The backend finished a `send` call.
    pub fn sent(&mut self, key: &str, result: Result<(), String>) {
        match result {
            Ok(()) => self.delivered(key),
            Err(error) => {
                if self
                    .in_flight
                    .as_ref()
                    .is_some_and(|prompt| prompt.key == key)
                {
                    self.in_flight = None;
                    // The agent was not given it, so nobody's word stands behind its turn.
                    self.given = None;
                    self.queue.finish(key, QueueState::Failed);
                    self.error = Some(format!("Could not deliver input: {error}"));
                    self.unattended_problem();
                }
            }
        }
    }

    /// Claim the presented approval for a decision, which grants `option` if given. Only the first claim succeeds; the next pending approval is presented at once. A claim naming an option the approval does not offer fails and leaves it presented.
    pub fn claim_approval(
        &mut self,
        request: &str,
        option: Option<&str>,
    ) -> Result<(), &'static str> {
        // The approval may already have been resolved during the gap; the session read again will say.
        if self.reconnecting.is_some() {
            return Err(RECONNECTING);
        }
        match self.approvals.front() {
            Some(pending)
                if pending.id == request
                    && option.is_some_and(|option| {
                        !projection::options(pending)
                            .iter()
                            .any(|offered| offered.id == option)
                    }) =>
            {
                Err("That option is not offered for this approval.")
            }
            Some(pending) if pending.id == request => {
                let claimed = self.approvals.pop_front().expect("checked above");
                self.deciding.push(claimed);
                Ok(())
            }
            _ => Err("That approval is no longer pending."),
        }
    }

    /// The backend accepted a decision.
    pub fn decided(&mut self, request: &str) {
        if self.deciding.iter().any(|pending| pending.id == request) {
            self.settle(request);
        }
        self.deciding.retain(|pending| pending.id != request);
    }

    /// The backend rejected a decision for a reason other than the request being gone: present the approval again, ahead of later ones.
    pub fn restore_approval(&mut self, request: &str) {
        if let Some(index) = self
            .deciding
            .iter()
            .position(|pending| pending.id == request)
        {
            let pending = self.deciding.remove(index);
            self.approvals.push_front(pending);
        }
    }

    /// Record an answer to the current field. The first answer to a field wins.
    pub fn answer(
        &mut self,
        question: &str,
        field: &str,
        text: &str,
    ) -> Result<AnswerOutcome, String> {
        if self.reconnecting.is_some() {
            return Err(RECONNECTING.to_owned());
        }
        let pending = self
            .questions
            .front_mut()
            .filter(|pending| pending.question.id == question)
            .ok_or("That question is no longer pending.")?;
        if !projection::supported(&pending.question) {
            return Err("Complete this question outside Tomori.".to_owned());
        }
        let current = projection::current_field(&pending.question, &pending.answers)
            .filter(|current| projection::field_name(&pending.question, current) == field)
            .ok_or("The question changed; read the current field and answer again.")?;
        let value = projection::answer_value(current, text)?;
        let key = current.key.clone();
        pending.answers.push((key, value));
        if projection::current_field(&pending.question, &pending.answers).is_some() {
            return Ok(AnswerOutcome::Recorded);
        }
        let complete = self.questions.pop_front().expect("checked above");
        let answers = complete.answers.clone();
        self.submitting.push(complete);
        Ok(AnswerOutcome::Complete(answers))
    }

    /// The backend accepted the answers to a question.
    pub fn answered(&mut self, question: &str) {
        if self
            .submitting
            .iter()
            .any(|pending| pending.question.id == question)
        {
            self.settle(question);
        }
        self.submitting
            .retain(|pending| pending.question.id != question);
    }

    /// The backend rejected the completed answers for a reason other than the question being gone: present it again from the start, ahead of later questions.
    pub fn restore_question(&mut self, question: &str) {
        if let Some(index) = self
            .submitting
            .iter()
            .position(|pending| pending.question.id == question)
        {
            let mut pending = self.submitting.remove(index);
            pending.answers.clear();
            self.questions.push_front(pending);
        }
    }

    /// Everyone in the space, participants and visitors, who receive its state, presence, audio and speech.
    pub fn members(&self) -> impl Iterator<Item = &String> {
        self.participants.iter().chain(&self.visitors)
    }

    /// The space's shared state, with `participants` those in it and `environment` its environment's layer of the instructions, as in [`Self::instructions`].
    pub fn state(&self, participants: Vec<Participant>, environment: &str) -> SpaceState {
        SpaceState {
            id: self.id.clone(),
            title: self.title.clone(),
            backend: self.backend.clone(),
            directory: self.directory.clone(),
            environment: self.environment.clone(),
            // Set by the hub, which knows the environments offered.
            environment_version: None,
            activity: self.activity(),
            messages: self
                .messages
                .iter()
                .filter_map(|message| {
                    let text = message.display();
                    (!text.is_empty()).then(|| tomori_protocol::Message {
                        id: message.id.clone(),
                        role: message.role,
                        text,
                        completed: message.completed,
                    })
                })
                .collect(),
            queue: self.queue.entries().to_vec(),
            approval: self.approvals.front().map(projection::approval),
            question: self
                .questions
                .front()
                .map(|pending| projection::question(&pending.question, &pending.answers)),
            workers: self
                .workers
                .iter()
                .map(|worker| self.watch.shown(worker))
                .collect(),
            workers_authoritative: Some(self.workers_authoritative),
            presentations: Vec::new(),
            participants,
            visitors: self.visitors.len(),
            attribution: self.attribution,
            speech: self.speech,
            // Set by the hub, which knows the agent's speech.
            speech_to_come: false,
            // Set by the hub, which drives the agent's state machine.
            agent: None,
            voice_answers: true,
            environment_changes: true,
            // Set by the hub.
            projection: false,
            walls: false,
            checks: false,
            choices: false,
            groups: false,
            wall_locks: false,
            // Set by the hub, which knows the board.
            cards: false,
            // Set by the hub, which knows the backends.
            deletes: false,
            deletes_session: false,
            renamed_by: self.renamed_by.clone(),
            renames: true,
            // Set by the hub, which knows the environments.
            anchors: tomori_protocol::Anchors::default(),
            summoned: None,
            summoner: None,
            summons: false,
            movement: Some(self.movement),
            // Set by the hub, which knows where the agent stands.
            pinned: false,
            pins: false,
            walks: false,
            walking: None,
            facing: None,
            partner: None,
            via: None,
            stream: false,
            // Set by the hub, which knows the backends.
            avatar: None,
            clusters: Vec::new(),
            timers: self.timers.views(),
            error: {
                let problems: Vec<&str> =
                    [&self.error, &self.worker_error, &self.environment_error]
                        .into_iter()
                        .flatten()
                        .map(String::as_str)
                        .collect();
                (!problems.is_empty()).then(|| problems.join(" "))
            },
            mode: self
                .modes
                .current
                .as_ref()
                .filter(|mode| mode.len() <= limits::IDENTIFIER_BYTES)
                .cloned(),
            modes: self.shown_modes(),
            instructions: self.instructions(environment),
        }
    }

    pub fn summary(&self) -> SpaceSummary {
        SpaceSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            backend: self.backend.clone(),
            directory: self.directory.clone(),
            environment: self.environment.clone(),
            activity: self.activity(),
            participants: self.participants.len(),
            attention: self.attention(),
            // How long it has needed it is known to the hub, which stamps when the level changes.
            attention_age: None,
            // The hub knows the workspace roots and when the space was last active.
            folder: None,
            active_age: None,
            latest: self
                .messages
                .iter()
                .rev()
                .filter(|message| is_reply(message))
                .find_map(|message| excerpt(&message.text)),
        }
    }
}

#[cfg(test)]
mod tests;
