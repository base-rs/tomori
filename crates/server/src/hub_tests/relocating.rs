// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A space follows its session when the agent moves it, such as with OpenCode's session move, and Tomori's tools move
//! with it: removed from where it was first, unless another space is open there, then registered where it is now. See
//! `design/mcp.md`.
use super::*;

/// A hub serving Tomori's tools, with every registration already due taken.
fn served() -> (Arc<Hub>, async_channel::Receiver<Job>) {
    let (hub, jobs) = hub_with(FakeBackend::new());
    hub.serve_tools(tomori_backend::ToolServer::new(
        "tomori",
        "http://127.0.0.1:1/mcp",
    ));
    (hub, jobs)
}

/// The registrations due, in the order they are made: each directory, and whether the tools are registered there.
fn due(hub: &Hub) -> Vec<(String, bool)> {
    std::iter::from_fn(|| hub.next_registration())
        .map(|registration| (registration.directory, registration.server.is_some()))
        .collect()
}

fn directory(hub: &Hub, space: &str) -> String {
    hub.lock().space(space).unwrap().model.directory.clone()
}

#[test]
fn a_space_follows_its_session_where_the_agent_moves_it_and_its_tools_leave_where_it_was_first() {
    let (hub, _jobs) = served();
    let (space, _commands) = running(&hub, &request("nobody"), Snapshot::default());
    assert_eq!(due(&hub), [("/tmp".to_owned(), true)]);

    // `/a/moved` sorts before `/tmp`, but the removal is made first, so that it never undoes the registration where
    // the session is now, should the two name the same place.
    let _ = hub.space_event(
        &space,
        Event::DirectoryChanged {
            directory: "/a/moved".into(),
        },
    );
    assert_eq!(directory(&hub, &space), "/a/moved");
    assert_eq!(
        due(&hub),
        [("/tmp".to_owned(), false), ("/a/moved".to_owned(), true)]
    );
    let state = hub
        .lock()
        .space(&space)
        .unwrap()
        .model
        .state(Vec::new(), "");
    assert_eq!(state.directory, "/a/moved");

    // Moved where it already is, or somewhere it cannot name, it stays.
    for nowhere in [
        "/a/moved",
        "",
        "relative",
        &format!("/{}", "d".repeat(limits::DIRECTORY_BYTES)),
    ] {
        let _ = hub.space_event(
            &space,
            Event::DirectoryChanged {
                directory: nowhere.to_owned(),
            },
        );
        assert_eq!(directory(&hub, &space), "/a/moved");
        assert!(due(&hub).is_empty(), "{nowhere:?}");
    }

    // A move it missed, said by the session read again, is followed too.
    let _ = hub.space_event(
        &space,
        Event::Resynchronized {
            snapshot: Snapshot {
                directory: Some("/b".into()),
                ..Snapshot::default()
            },
        },
    );
    assert_eq!(directory(&hub, &space), "/b");
    assert_eq!(
        due(&hub),
        [("/a/moved".to_owned(), false), ("/b".to_owned(), true)]
    );

    // Closed, it follows its session no more.
    let _ = hub.space_event(&space, Event::Closed { reason: None });
    let _ = hub.space_event(
        &space,
        Event::DirectoryChanged {
            directory: "/c".into(),
        },
    );
    assert_eq!(directory(&hub, &space), "/b");
    assert!(due(&hub).is_empty());
}

#[test]
fn a_session_moved_from_a_directory_another_space_is_open_in_leaves_the_tools_there() {
    let (hub, _jobs) = served();
    let (moving, _commands) = running(&hub, &request("nobody"), Snapshot::default());
    let (_staying, _commands) = running(&hub, &request("nobody"), Snapshot::default());
    due(&hub);
    let _ = hub.space_event(
        &moving,
        Event::DirectoryChanged {
            directory: "/a".into(),
        },
    );
    assert_eq!(
        due(&hub),
        [("/a".to_owned(), true), ("/tmp".to_owned(), true)]
    );
}

#[test]
fn a_space_started_in_one_directory_is_where_the_agent_says_its_session_is() {
    let (hub, _jobs) = served();
    let (space, _commands) = running(
        &hub,
        &request("nobody"),
        Snapshot {
            directory: Some("/tmp/project".into()),
            ..Snapshot::default()
        },
    );
    assert_eq!(directory(&hub, &space), "/tmp/project");
    assert_eq!(due(&hub), [("/tmp/project".to_owned(), true)]);
}
