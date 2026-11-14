// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Pairing devices, its window, and knocking to be admitted.

use super::*;

impl Hub {
    /// Act on `message`, a message about pairing or knocking, from `participant` on `connection`, who is in `space` if any.
    pub(super) fn act_on_pairing(
        &self,
        state: &mut HubState,
        connection: ConnectionId,
        participant: &str,
        space: Option<&str>,
        message: ClientMessage,
        deferred: &mut Vec<Deferred>,
    ) {
        let outbox = state.connections[&connection].outbox.clone();
        match message {
            ClientMessage::PairBegin { commitment } => {
                if tomori_security::pairing::Nonce::parse(&commitment).is_none() {
                    state.error(
                        participant,
                        "A pairing commitment is 64 hexadecimal digits.",
                    );
                    return;
                }
                let failed = |error: &str| ServerMessage::PairFailed {
                    error: error.to_owned(),
                };
                let Some(setup) = self.pairing.read().expect("never left poisoned").clone() else {
                    outbox.message(failed("This server does not pair devices."));
                    return;
                };
                if !self.encrypted.load(Ordering::Relaxed) {
                    outbox.message(failed("Pairing needs a connection over TLS."));
                    return;
                }
                let now = Instant::now();
                // Closed, also on a server with no owner yet, except during a window an owner or the operator opened; refused at once, and not counted against the attempts, so that requests while closed cannot pause pairing for the owner who opens it. See `design/security.md`.
                if !self.pairing_open(state, now) {
                    outbox.message(failed(tomori_protocol::PAIRING_CLOSED));
                    self.refuse_closed(state, now, deferred);
                    return;
                }
                // Refused even to the same connection, which must not get a fresh value from the server once it has seen one; see `design/security.md`.
                if state.pairing.is_some() {
                    outbox.message(failed(
                        "A pairing is already in progress; try again shortly.",
                    ));
                    return;
                }
                while state
                    .pairing_attempts
                    .front()
                    .is_some_and(|began| now.saturating_duration_since(*began) >= PAIRING_WINDOW)
                {
                    state.pairing_attempts.pop_front();
                }
                if state.pairing_attempts.len() >= PAIRING_ATTEMPTS {
                    outbox.message(failed(
                        "Too many pairings began recently; try again in a few minutes.",
                    ));
                    deferred.push(Deferred::PairingPrompt(PairingEvent::Paused));
                    return;
                }
                state.pairing_attempts.push_back(now);
                state.next_pairing += 1;
                let server = tomori_security::pairing::Nonce::generate();
                // The name the device chose enters pairing here, made safe to show once, so that every display of the pairing shows the same; see `design/security.md`.
                let token_name = state.participants[participant].name.clone();
                let device = ChosenName::new(&token_name);
                log::info!(target: LOG, "Pairing began from {device}.");
                deferred.push(Deferred::PairingPrompt(PairingEvent::Began {
                    device: device.clone(),
                }));
                state.pairing = Some(PendingPairing {
                    serial: state.next_pairing,
                    connection,
                    outbox: outbox.clone(),
                    setup,
                    device,
                    token_name,
                    commitment: commitment.to_ascii_lowercase(),
                    server,
                    code: None,
                    started: now,
                });
                outbox.message(ServerMessage::PairNonce {
                    nonce: server.to_hex(),
                });
            }
            ClientMessage::PairReveal { nonce } => {
                let Some(pairing) = state
                    .pairing
                    .as_mut()
                    .filter(|pairing| pairing.connection == connection && pairing.code.is_none())
                else {
                    state.error(participant, "No pairing is waiting for that.");
                    return;
                };
                let Some(device) = tomori_security::pairing::Nonce::parse(&nonce)
                    .filter(|device| device.commitment() == pairing.commitment)
                else {
                    state.end_pairing(
                        Some(connection),
                        "The pairing could not be checked; try again.",
                    );
                    return;
                };
                let code = tomori_security::pairing::code(
                    pairing.setup.fingerprint,
                    device,
                    pairing.server,
                );
                let device = pairing.device.clone();
                pairing.code = Some(code);
                outbox.message(ServerMessage::PairWaiting);
                // Never the code: the log and the console may be read by others on the server's machine, such as its agents, and the operator confirms with the code the device shows. See `design/security.md`.
                log::info!(
                    target: LOG,
                    "Pairing request from {device}: it waits to be confirmed with the code its device shows."
                );
                let timeout = *self.pairing_timeout.read().expect("never left poisoned");
                let now = SystemTime::now();
                let began = now
                    .checked_sub(Instant::now().saturating_duration_since(pairing.started))
                    .unwrap_or(now);
                deferred.push(Deferred::PairingPrompt(PairingEvent::Waiting {
                    device,
                    began,
                    // Only a library's timeout too long for any clock could not be added.
                    expires: began.checked_add(timeout).unwrap_or(began),
                }));
            }
            ClientMessage::Knock => {
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                if state.role(connection, Some(&space)).participates() {
                    state.error(participant, "You already take part in this space.");
                    return;
                }
                // One token admits a device as one thing; a collaborator elsewhere asks an owner instead.
                if state.connections[&connection].access.principal.is_some() {
                    state.error(
                        participant,
                        "You are admitted elsewhere on this server; ask an owner to let you in here.",
                    );
                    return;
                }
                let Some(tokens) = self.tokens.read().expect("never left poisoned").clone() else {
                    state.error(participant, "This server does not admit by knocking.");
                    return;
                };
                if !self.encrypted.load(Ordering::Relaxed) {
                    state.error(participant, "Knocking needs a connection over TLS.");
                    return;
                }
                if state
                    .knocks
                    .iter()
                    .any(|knock| knock.participant == participant)
                {
                    state.error(participant, "Your knock is already waiting.");
                    return;
                }
                let now = Instant::now();
                let key = (participant.to_owned(), space.clone());
                if state
                    .knock_cooldowns
                    .get(&key)
                    .is_some_and(|until| now < *until)
                {
                    state.error(participant, "You knocked recently; try again in a minute.");
                    return;
                }
                if state
                    .knocks
                    .iter()
                    .filter(|knock| knock.space == space)
                    .count()
                    >= KNOCKS_PER_SPACE
                {
                    state.error(
                        participant,
                        "Too many knocks are waiting on this space; try again shortly.",
                    );
                    return;
                }
                state.next_knock += 1;
                let knock = KnockRecord {
                    id: format!("knock-{}", state.next_knock),
                    space: space.clone(),
                    participant: participant.to_owned(),
                    connection,
                    outbox: outbox.clone(),
                    tokens,
                    name: state.participants[participant].name.clone(),
                    started: now,
                };
                log::info!(target: LOG, "{:?} knocked on {space}.", knock.name);
                state.knocks.push(knock);
                outbox.message(ServerMessage::KnockState {
                    state: tomori_protocol::KnockState::Pending,
                });
                state.announce_knocks(&space);
            }
            ClientMessage::DecideKnock { knock, admit } => {
                if let Err(error) = limits::check(&knock, limits::IDENTIFIER_BYTES, "knock") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                // The first decision resolves a knock; later ones find it gone.
                let Some(index) = state
                    .knocks
                    .iter()
                    .position(|waiting| waiting.id == knock && waiting.space == space)
                else {
                    state.error(participant, "That knock was already answered.");
                    return;
                };
                let knock = state.knocks.remove(index);
                state.announce_knocks(&space);
                if admit {
                    deferred.push(Deferred::Admit(knock));
                } else {
                    log::info!(target: LOG, "{:?} was declined from {space}.", knock.name);
                    state.knock_cooldowns.insert(
                        (knock.participant.clone(), space),
                        Instant::now() + KNOCK_COOLDOWN,
                    );
                    knock.outbox.message(ServerMessage::KnockState {
                        state: tomori_protocol::KnockState::Declined,
                    });
                }
            }
            ClientMessage::PairCancel => {
                state.end_pairing(Some(connection), "The pairing was cancelled.");
            }
            // Owners only: visitors, collaborators and agents never open pairing. See `design/security.md`.
            ClientMessage::OpenPairing => {
                let name = state.participants[participant].name.clone();
                if let Err(error) = self.open_window(state, Opener::owner(&name), deferred) {
                    state.error(participant, error);
                }
            }
            ClientMessage::ClosePairing => {
                let name = state.participants[participant].name.clone();
                if !self.close_window(state, format!("{name} closed it."), deferred) {
                    state.error(participant, "Pairing is not open.");
                }
            }
            // `act` hands this function only messages about pairing or knocking.
            _ => unreachable!("not a message about pairing or knocking"),
        }
    }
}
