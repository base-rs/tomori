// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! The space's projection walls: adding, moving, attaching, locking and removing them.

use super::*;

impl Hub {
    /// Act on `message`, a message about walls, from `participant`, who is in `space` if any.
    pub(super) fn act_on_walls(
        &self,
        state: &mut HubState,
        participant: &str,
        space: Option<&str>,
        message: ClientMessage,
        deferred: &mut Vec<Deferred>,
    ) {
        match message {
            ClientMessage::AddWall { pose } => {
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                let walls = environments.walls(&record.model.environment);
                match record.stage.add_wall(&walls, &pose, None) {
                    Ok(_) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            ClientMessage::SetWall {
                wall,
                grid,
                scale,
                aspect,
                shape,
                pose,
                tilt,
                content_scale,
                revision,
            } => {
                if let Err(error) = limits::check(&wall, limits::IDENTIFIER_BYTES, "wall") {
                    state.error(participant, error);
                    return;
                }
                let change = match crate::stage::WallChange::new(
                    grid,
                    scale,
                    aspect,
                    shape,
                    pose,
                    tilt,
                    content_scale,
                ) {
                    Ok(change) => change,
                    Err(error) => {
                        state.error(participant, error);
                        return;
                    }
                };
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                // Changed since the version they saw, as by another participant's change, it is refused, naming the
                // wall, so that their client cues it, rather than undo the other change; one that says none, as a
                // carry's or an older client's, goes by what it says alone.
                let shown = record
                    .versioned(&environments, crate::cards::unix_now())
                    .wall(&wall)
                    .map(|shown| shown.version);
                if let Some(seen) = revision
                    && shown.is_some_and(|shown| shown != seen)
                {
                    state.send(
                        participant,
                        ServerMessage::Error {
                            error: crate::stage::STALE_WALL.to_owned(),
                            space: None,
                            panel: None,
                            wall: Some(wall),
                        },
                    );
                    state.touch_projection(&space);
                    return;
                }
                let walls = environments.walls(&record.model.environment);
                match record.stage.set_wall(&walls, &wall, change) {
                    Ok(true) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    // Already so, it is sent again, so that a client showing it otherwise meanwhile shows it as it is.
                    Ok(false) => state.touch_projection(&space),
                    Err(error) => {
                        state.error(participant, error);
                        // Sent again, so that a client that showed the change at once shows the wall as it is.
                        state.touch_projection(&space);
                    }
                }
            }
            ClientMessage::AttachWall {
                wall,
                attach,
                alone,
                pose,
            } => {
                let checked =
                    limits::check(&wall, limits::IDENTIFIER_BYTES, "wall").and_then(|()| {
                        attach.as_ref().map_or(Ok(()), |attach| {
                            limits::check(&attach.to, limits::IDENTIFIER_BYTES, "wall")
                        })
                    });
                if let Err(error) = checked {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                let walls = environments.walls(&record.model.environment);
                match record.stage.attach_wall(&walls, &wall, attach, alone, pose) {
                    Ok(true) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    // Sent again either way, so that a client that showed the snap at once shows the walls as they are.
                    Ok(false) => state.touch_projection(&space),
                    Err(error) => {
                        state.error(participant, error);
                        state.touch_projection(&space);
                    }
                }
            }
            ClientMessage::LockWall {
                wall,
                locked,
                reset,
            } => {
                if let Err(error) = limits::check(&wall, limits::IDENTIFIER_BYTES, "wall") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                let walls = environments.walls(&record.model.environment);
                match record.stage.lock_wall(&walls, &wall, locked, reset) {
                    Ok(true) => {
                        log::info!(
                            target: LOG,
                            "An owner {} {wall} in space {space}.",
                            match (reset, locked) {
                                (true, _) => "put back and locked",
                                (false, true) => "locked",
                                (false, false) => "unlocked",
                            }
                        );
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Ok(false) => state.touch_projection(&space),
                    Err(error) => {
                        state.error(participant, error);
                        state.touch_projection(&space);
                    }
                }
            }
            ClientMessage::RemoveWall { wall } => {
                if let Err(error) = limits::check(&wall, limits::IDENTIFIER_BYTES, "wall") {
                    state.error(participant, error);
                    return;
                }
                let Some(space) = in_space(state, participant, space) else {
                    return;
                };
                let environments = self.environments();
                let record = state.space_mut(&space).expect("checked above");
                let walls = environments.walls(&record.model.environment);
                match record.stage.remove_wall(&walls, &wall) {
                    Ok(()) => {
                        state.touch_projection(&space);
                        deferred.push(Deferred::Keep);
                    }
                    Err(error) => state.error(participant, error),
                }
            }
            // `act` hands this function only messages about walls.
            _ => unreachable!("not a message about walls"),
        }
    }
}
