// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The panels on the walls: placing, resynchronising, fetching their images, dismissing, checking, freezing and choosing from them.

use super::*;

impl Hub {
    /// Act on `message`, a message about panels, from `participant` on `connection`, who is in `space` if any.
    pub(super) fn act_on_panels(
        &self,
        state: &mut HubState,
        connection: ConnectionId,
        participant: &str,
        space: Option<&str>,
        message: ClientMessage,
        deferred: &mut Vec<Deferred>,
    ) {
        match message {
            ClientMessage::PlacePanel {
                panel,
                wall,
                slot,
                pose,
                size,
                revision,
            } => {
                let checked =
                    limits::check(&panel, limits::IDENTIFIER_BYTES, "panel").and_then(|()| {
                        wall.as_deref().map_or(Ok(()), |wall| {
                            limits::check(wall, limits::IDENTIFIER_BYTES, "wall")
                        })
                    });
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let target = match (wall, slot, pose) {
                    (Some(wall), Some(slot), _) => crate::stage::Target::Slot(wall, slot),
                    (Some(wall), None, _) => crate::stage::Target::Wall(wall),
                    (None, _, Some(pose)) => crate::stage::Target::Free(pose),
                    (None, _, None) => {
                        state.error(participant, "Say where to put the panel.");
                        return;
                    }
                };
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                // Stretched or shrunk with both hands, as a panel left floating, the change is refused where the panel
                // changed since the version the participant saw as they took hold of it with the second hand, as by the
                // agent rewriting it or another participant moving it, rather than lose that change. See
                // `design/projection.md`.
                let size = size.filter(|_| matches!(target, crate::stage::Target::Free(_)));
                let stale = size.is_some()
                    && revision.is_some_and(|seen| {
                        record
                            .versioned(&environments, crate::cards::unix_now())
                            .panel(&panel)
                            .is_some_and(|shown| shown.version != seen)
                    });
                let walls = environments.walls(&record.model.environment);
                // Pulled off its wall, an environment's panel is pinned, a copy put there in its place. See
                // `design/panel-context.md`.
                let placed = if stale {
                    Err(crate::stage::STALE_SIZE)
                } else {
                    record
                        .stage
                        .place_pulled(&walls, &panel, target, size)
                        .map(|(changed, copy)| {
                            if let Some((copy, instance)) = copy {
                                log::info!(
                                    target: LOG,
                                    "Participant {participant} pulled {panel} off its wall in space {space}, pinned as {copy}, instance {instance}."
                                );
                            }
                            changed
                        })
                };
                match placed {
                    Ok(true) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    // Already there, it is sent again, so that a client showing it elsewhere meanwhile puts it back.
                    Ok(false) => state.touch_projection(&space),
                    // Refused as made against a version since changed, it names the panel, so that the client shows it as
                    // the server has it, and the projection is sent again.
                    Err(error) if stale => {
                        state.send(
                            participant,
                            ServerMessage::Error {
                                error: error.to_owned(),
                                space: None,
                                panel: Some(panel),
                                wall: None,
                            },
                        );
                        state.touch_projection(&space);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            // A client that could not apply a patch is sent the projection whole, at the space's revision now; one in no
            // space, or that reads no projection, is sent nothing, as a patch on its way as it left asks innocently. So is
            // one already waiting for it whole, as on joining or having asked, so that asking again and again is sent it
            // once, and one that applies no patches, which is sent it whole at every change anyway.
            ClientMessage::ResyncProjection { .. } => {
                if space.is_some()
                    && let Some(record) = state
                        .connections
                        .get_mut(&connection)
                        .filter(|record| record.projection && record.patches)
                    && record.projected.take().is_some()
                {
                    log::debug!(target: LOG, "A client asked for the projection whole.");
                    record.outbox.projection();
                }
            }
            // Only an image a panel shown in the client's space has: what a client may see is what its space shows. A
            // refusal is not said, since it happens innocently, as when the panel is dismissed while its image is asked
            // for, and the participant can do nothing about it; the client gives up after a few tries.
            ClientMessage::FetchImage { image } => {
                let environments = self.environments();
                let shown = tomori_protocol::hash_bytes(&image).filter(|_| {
                    space
                        .and_then(|space| state.space(space))
                        .is_some_and(|record| {
                            let walls = environments.walls(&record.model.environment);
                            record.stage.shows_image(&walls, &image)
                        })
                });
                match shown {
                    Some(hash) => deferred.push(Deferred::Image(connection, hash)),
                    None => {
                        log::debug!(target: LOG, "Refused an image that no panel the client's space shows has.")
                    }
                }
            }
            ClientMessage::DismissPanel { panel } => {
                if let Err(error) = limits::check(&panel, limits::IDENTIFIER_BYTES, "panel") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let record = state.space_mut(&space).expect("checked above");
                let environments = self.environments();
                let walls = environments.walls(&record.model.environment);
                let dismissed = record.stage.dismiss(&walls, &panel);
                drop(environments);
                match dismissed {
                    Ok(()) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            // A participant ticks or unticks a checklist item they point at, for everyone, by the same rule as
            // `update_panel`, and the agent is told before the next words spoken in the space, without a turn of its own.
            ClientMessage::CheckPanelItem {
                panel,
                line,
                checked,
                revision,
            } => {
                if let Err(error) = limits::check(&panel, limits::IDENTIFIER_BYTES, "panel") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let who = state.participants[participant].name.clone();
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                // Changed since the version they saw, as by the agent's rewrite, another participant's tick or a move,
                // the tick is refused, rather than tick whatever item is on that line now. An older client says none.
                let shown = record
                    .versioned(&environments, crate::cards::unix_now())
                    .panel(&panel)
                    .map(|shown| shown.version);
                let ticked = match revision {
                    Some(seen) if shown.is_some_and(|shown| shown != seen) => {
                        Err(crate::stage::STALE_TICK)
                    }
                    _ => {
                        let walls = environments.walls(&record.model.environment);
                        record.stage.tick_item(&walls, &panel, line, checked)
                    }
                };
                match ticked {
                    Ok(Some(ticked)) => {
                        record.model.note_tick(&panel, line, ticked.note(&who));
                        log::info!(
                            target: LOG,
                            "A participant {} item {} of {panel} in space {space}.",
                            if checked { "ticked" } else { "unticked" },
                            ticked.number
                        );
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    // Already so, it is sent again, so that a client that showed the change at once shows it as it is.
                    Ok(None) => state.touch_projection(&space),
                    // Refused, as when the panel changed since the revision they saw, it names the panel, so that their
                    // client cues it, and the projection is sent again, so that they see the panel as it is and look again.
                    Err(error) => {
                        state.send(
                            participant,
                            ServerMessage::Error {
                                error: error.to_owned(),
                                space: None,
                                panel: Some(panel),
                                wall: None,
                            },
                        );
                        state.touch_projection(&space);
                    }
                }
            }
            // A participant freezes or unfreezes a panel by its time chip, for everyone, as it was when they saw it.
            ClientMessage::FreezePanel {
                panel,
                frozen,
                revision,
            } => {
                if let Err(error) = limits::check(&panel, limits::IDENTIFIER_BYTES, "panel") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                let stale = revision.is_some_and(|seen| {
                    record
                        .versioned(&environments, crate::cards::unix_now())
                        .panel(&panel)
                        .is_some_and(|shown| shown.version != seen)
                });
                let walls = environments.walls(&record.model.environment);
                let frozen = if stale {
                    Err(crate::stage::STALE_FREEZE)
                } else {
                    record.stage.freeze(&walls, &panel, frozen)
                };
                drop(environments);
                match frozen {
                    Ok(changed) => {
                        if changed {
                            log::info!(
                                target: LOG,
                                "Participant {participant} froze or unfroze {panel} in space {space}."
                            );
                            deferred.push(Deferred::Keep);
                        }
                        state.touch_projection(&space);
                    }
                    Err(error) => {
                        state.send(
                            participant,
                            ServerMessage::Error {
                                error: error.to_owned(),
                                space: None,
                                panel: Some(panel),
                                wall: None,
                            },
                        );
                        state.touch_projection(&space);
                    }
                }
            }
            // A participant chooses a row of an environment's list panel, which is handed to that environment's server side
            // as a choice only the server makes, by the row's id, never its place, within the participant's rate of
            // intents. See `design/panel-actions.md`.
            ClientMessage::ChoosePanelRow {
                panel,
                row,
                revision,
                key,
            } => {
                if let Err(error) = limits::check(&panel, limits::IDENTIFIER_BYTES, "panel") {
                    state.error(participant, error);
                    return;
                }
                if !tomori_environment::valid_row_id(&row) {
                    state.error(participant, "That is not a row's id.");
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space(&space).expect("checked above");
                let shown = record
                    .versioned(&environments, crate::cards::unix_now())
                    .panel(&panel)
                    .map(|shown| shown.version);
                let walls = environments.walls(&record.model.environment);
                let chosen = record
                    .stage
                    .chosen(&walls, &panel, &row)
                    .and_then(|(id, chosen)| {
                        // Chosen from the panel as it is now, unless it changed since the version they saw; a client
                        // that says none is taken at its word.
                        let current = revision.is_none_or(|seen| shown == Some(seen));
                        // A row of a list with keys is handed on with its key only where that is the key the participant
                        // saw: by the revision of its key they say, or, from a client that says none, by the panel's version,
                        // or at its word where it says neither; otherwise it is refused as stale, never resolved to another
                        // key. A client never sends a key, so none is forged.
                        let (revised, fields) =
                            record.stage.row_key(walls.environment, &panel, &row);
                        let seen = match (revised, key) {
                            (None, _) => true,
                            (Some(revised), Some(seen)) => revised == seen,
                            (Some(_), None) => current,
                        };
                        if !seen {
                            return Err(crate::stage::STALE_ROW);
                        }
                        Ok(tomori_environment::Chosen::new(id, chosen, current)
                            .with_fields(fields.to_vec()))
                    });
                match chosen {
                    Ok(chosen) => {
                        let record = state
                            .participants
                            .get_mut(participant)
                            .expect("the participant acting is listed");
                        let taken = record.intents.take(Instant::now());
                        let source = record.source;
                        let said = format!("row {} of {}", chosen.row(), chosen.panel());
                        // Logged only once taken, so that a flood past the rate fills no log.
                        if taken
                            && self
                                .server_sides
                                .deliver(&space, tomori_environment::Intent::choice(source, chosen))
                        {
                            log::info!(target: LOG, "A participant chose {said} in space {space}.");
                        } else {
                            log::debug!(target: LOG, "A choice of {said} was dropped.");
                        }
                    }
                    Err(error) => state.send(
                        participant,
                        ServerMessage::Error {
                            error: error.to_owned(),
                            space: None,
                            panel: Some(panel),
                            wall: None,
                        },
                    ),
                }
            }
            // `act` hands this function only messages about panels.
            _ => unreachable!("not a message about panels"),
        }
    }
}
