// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use tomori_protocol::diagram::parse;

/// A thing called `names`, labelled with the first, standing at `at`.
fn thing(names: &[&str], at: Option<[f32; 3]>) -> Thing {
    Thing {
        names: names.iter().map(|name| (*name).to_owned()).collect(),
        label: names[0].to_owned(),
        described: format!("the {}", names[0]),
        at,
        radius: 0.4,
        frame: None,
        absent: at.is_none().then(|| "held by someone".to_owned()),
    }
}

/// The farmyard as the marshmallow example finds it: the bonfire and four sticks, the second held, the lookout, a
/// notice board, a panel, a worker, Samuel, and the agent, with Samuel asking from the lookout.
fn campsite() -> Things {
    let mut sticks: Vec<Thing> = (0..4)
        .map(|index| {
            thing(
                &[
                    &format!("farmyard:campsite/stick-{index}"),
                    &format!("campsite/stick-{index}"),
                ],
                Some([-4.0, 0.5, -14.0 + index as f32 * 0.1]),
            )
        })
        .collect();
    sticks[2].at = None;
    sticks[2].absent = Some("held by someone".to_owned());
    let mut fire = thing(
        &["farmyard:campsite/bonfire", "campsite/bonfire"],
        Some([-4.45, 0.45, -14.85]),
    );
    fire.label = "the campsite's bonfire".to_owned();
    let mut board = thing(&["wall-2", "Notice board"], Some([0.0, 1.5, -3.0]));
    board.frame = Some(Frame {
        orientation: [0.0, 0.0, 0.0, 1.0],
        size: [2.0, 1.2],
    });
    Things {
        environment: "farmyard".to_owned(),
        objects: std::iter::once(fire).chain(sticks).collect(),
        anchors: vec![
            thing(&["home"], Some([0.0; 3])),
            thing(&["pool-lookout"], Some([-3.0, 0.0, -13.0])),
        ],
        walls: vec![board],
        panels: vec![thing(
            &["panel-3", "checklist", "Trial checklist"],
            Some([1.0, 1.4, -2.0]),
        )],
        workers: vec![thing(&["ses_1", "Build the client"], Some([2.0, 1.0, 0.0]))],
        people: vec![
            thing(&["Samuel#11", "Samuel"], Some([-2.0, 1.6, -12.0])),
            thing(&["Ada#3", "Ada"], None),
            thing(&["Ada#4", "Ada"], Some([5.0, 1.6, 0.0])),
        ],
        agent: Some(thing(&["agent"], Some([0.0, 1.2, -1.0]))),
        asker: Some(("Samuel".to_owned(), [-2.0, 1.6, -12.0])),
    }
}

fn resolved(body: &str, things: &Things) -> (Diagram, Vec<String>, Vec<String>) {
    let (mut diagram, _) = parse(body);
    let (told, warnings) = resolve(&mut diagram, things);
    (
        diagram,
        told,
        warnings.iter().map(ToString::to_string).collect(),
    )
}

#[test]
fn the_marshmallow_example_resolves_against_the_campsite() {
    let block = include_str!("marshmallow.diagram");
    let body = block
        .split_once("---\n")
        .unwrap()
        .1
        .trim_end_matches("```\n");
    let (diagram, told, warnings) = resolved(body, &campsite());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        told,
        [
            "fire → the farmyard:campsite/bonfire (farmyard:campsite/bonfire), 3.9 m from Samuel",
            "stick → the farmyard:campsite/stick-2 (farmyard:campsite/stick-2), held by someone, so nothing is drawn to it now",
            "seat → the pool-lookout (pool-lookout), 2.1 m from Samuel",
            "samuel → the Samuel#11 (Samuel#11), 0.0 m from Samuel",
        ]
    );
    let states: Vec<State> = diagram.world.iter().map(|target| target.state).collect();
    assert_eq!(
        states,
        [State::Here, State::Absent, State::Here, State::Here]
    );
    // The agent's labels are kept; the server's short name is used where it gave none.
    assert_eq!(diagram.world[0].label, "The bonfire");
    assert_eq!(diagram.world[3].label, "Samuel#11");
    assert_eq!(diagram.world[0].at, Some([-4.45, 0.45, -14.85]));
    assert_eq!(diagram.world[1].at, None);
    assert_eq!(diagram.world[0].radius, 0.4);
    // The outline an older client shows lists every world edge.
    assert!(
        diagram
            .outline()
            .contains("- 1. Find the fire → The bonfire (farmyard:campsite/bonfire): here")
    );
    assert!(
        diagram
            .outline()
            .contains("- 5. Let it cool and eat → Samuel#11: your turn")
    );
}

#[test]
fn every_kind_resolves_by_any_of_its_names_in_any_case() {
    let (diagram, told, warnings) = resolved(
        concat!(
            "a := object campsite/bonfire\n",
            "b := anchor HOME\n",
            "c := wall the notice board\n",
            "d := panel Trial Checklist\n",
            "e := worker ses_1\n",
            "f := person Ada#4\n",
            "g := agent\n",
            "n --> a & b & c & d & e & f & g\n",
        ),
        &Things {
            asker: None,
            ..campsite()
        },
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        told,
        [
            "a → the farmyard:campsite/bonfire (farmyard:campsite/bonfire)",
            "b → the home (home)",
            "c → the wall-2 (wall-2)",
            "d → the panel-3 (panel-3)",
            "e → the ses_1 (ses_1)",
            "f → the Ada#4 (Ada#4)",
            "g → the agent (agent)",
        ]
    );
    assert!(
        diagram
            .world
            .iter()
            .all(|target| target.state == State::Here)
    );
    assert_eq!(diagram.world[2].frame.unwrap().size, [2.0, 1.2]);
}

#[test]
fn a_reference_without_a_kind_is_tried_as_an_anchor_a_wall_a_panel_and_a_person_saying_which() {
    let (diagram, _, warnings) = resolved(
        "a := pool-lookout\nb := notice board\nc := checklist\nd := Samuel\ne := nowhere\nn --> a & b & c & d & e\n",
        &campsite(),
    );
    assert_eq!(
        warnings,
        [
            "a names no kind, so it was taken as the anchor \"pool-lookout\": write it as a := anchor pool-lookout",
            "b names no kind, so it was taken as the wall \"notice board\": write it as b := wall notice board",
            "c names no kind, so it was taken as the panel \"checklist\": write it as c := panel checklist",
            "d names no kind, so it was taken as the person \"Samuel\": write it as d := person Samuel",
            "e: nothing in this space is called \"nowhere\": write its kind, as e := anchor NAME; nothing is drawn to it",
        ]
    );
    let kinds: Vec<Kind> = diagram
        .world
        .iter()
        .map(|target| target.reference.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            Kind::Anchor,
            Kind::Wall,
            Kind::Panel,
            Kind::Person,
            Kind::Unknown
        ]
    );
    assert_eq!(diagram.world[4].state, State::Unknown);
}

#[test]
fn an_unknown_reference_is_told_with_a_suggestion_and_never_whether_it_is_elsewhere() {
    let mut things = campsite();
    things.workers = (0..8)
        .map(|index| thing(&[&format!("ses_worker_{index}")], Some([0.0; 3])))
        .collect();
    let (diagram, told, warnings) = resolved(
        concat!(
            "a := object farmyard:campsite/stick-7\n",
            "b := object sky:moon\n",
            "c := anchor lookout-pool\n",
            "d := person Bob\n",
            "e := person Ada\n",
            "f := worker Tests\n",
            "g := panel Agenda\n",
            "n --> a & b & c & d & e & f & g\n",
        ),
        &things,
    );
    assert!(told.is_empty());
    assert_eq!(
        warnings,
        [
            "a: no object \"farmyard:campsite/stick-7\" in this space (did you mean farmyard:campsite/stick-0?); nothing is drawn to it",
            "b: no object \"sky:moon\" in this space: it is in the farmyard environment now; nothing is drawn to it",
            "c: no anchor \"lookout-pool\" in this space (there are home and pool-lookout); nothing is drawn to it",
            "d: no person \"Bob\" in this space (there are Samuel#11, Ada#3 and Ada#4); nothing is drawn to it",
            "e: more than one person here is called \"Ada\": say which, as Ada#3 or Ada#4; nothing is drawn to it",
            "f: no worker \"Tests\" in this space (workers lists them); nothing is drawn to it",
            "g: no panel \"Agenda\" in this space (there are panel-3); nothing is drawn to it",
        ]
    );
    assert!(
        diagram
            .world
            .iter()
            .all(|target| target.state == State::Unknown)
    );
    // In a space with no agent, nothing of any kind, or an environment's object named by itself.
    let (_, _, warnings) = resolved(
        "me := agent\nx := panel any\ny := object farmyard:z\nn --> me & x & y\n",
        &Things::default(),
    );
    assert_eq!(
        warnings,
        [
            "me: the agent is not in this space; nothing is drawn to it",
            "x: no panel \"any\" in this space; nothing is drawn to it",
            "y: no object \"farmyard:z\" in this space: it is in the  environment now; nothing is drawn to it",
        ]
    );
    assert_eq!(listed(&[]), "");
}

#[test]
fn a_thing_whose_place_is_not_known_is_absent_and_says_why() {
    let mut things = campsite();
    things.people[0].absent = Some("away".to_owned());
    things.people[0].at = None;
    things.anchors[1].absent = None;
    things.anchors[1].at = None;
    let (diagram, told, _) = resolved(
        "s := person Samuel\nl := anchor pool-lookout\nn --> s & l\n",
        &things,
    );
    assert_eq!(
        told,
        [
            "s → the Samuel#11 (Samuel#11), away, so nothing is drawn to it now",
            "l → the pool-lookout (pool-lookout), where it is not known, so nothing is drawn to it now",
        ]
    );
    assert!(
        diagram
            .world
            .iter()
            .all(|target| target.drawn_at().is_none())
    );
}

/// A volume standing at `pose` of three nodes in a column, the middle pointing at a target at `at`.
fn pulled(layout: &str, pose: &Pose, at: [f32; 3]) -> (Vec<Spot>, Vec<Spot>) {
    let (mut diagram, _) = parse(&format!(
        "fire := object fire\n{layout}\na --> b --> c\nb --> fire\n"
    ));
    diagram.world[0].state = State::Here;
    diagram.world[0].at = Some(at);
    let before: Vec<Spot> = vec![[0.0, 200.0, 0.0], [0.0, 0.0, 0.0], [0.0, -200.0, 0.0]];
    let mut spots = before.clone();
    pull(&diagram, &mut spots, &[60.0; 3], pose, Some(1));
    (before, spots)
}

#[test]
fn a_node_pointing_at_a_thing_is_pulled_towards_the_side_facing_it_within_its_rank() {
    let pose = Pose::upright_facing([0.0, 1.5, 0.0], [0.0, 1.5, 2.0]);
    // A fire a metre to the volume's right pulls the middle node right, firmly, and not up or down.
    let (before, near) = pulled("", &pose, [1.0, 1.5, 0.0]);
    // Its port is on the side of the box about the nodes, here a column one node wide.
    assert!(near[1][0] > 10.0, "{near:?}");
    assert_eq!(near[1][1], 0.0);
    assert_eq!((near[0], near[2]), (before[0], before[2]));
    // One fifteen metres off barely turns it.
    let (_, far) = pulled("", &pose, [15.0, 1.5, 0.0]);
    assert!(far[1][0] > 0.0 && far[1][0] < near[1][0] / 4.0, "{far:?}");
    // One at the volume's middle, or not here, moves nothing.
    let (before, middle) = pulled("", &pose, [0.0, 1.5, 0.0]);
    assert_eq!(middle, before);
    let (mut diagram, _) = parse("fire := object fire\na --> fire\n");
    let mut spots = vec![[0.0; 3]];
    pull(&diagram, &mut spots, &[60.0], &pose, None);
    assert_eq!(spots, [[0.0; 3]]);
    diagram.world_edges[0].from = 7;
    diagram.world[0].state = State::Here;
    diagram.world[0].at = Some([1.0, 1.5, 0.0]);
    pull(&diagram, &mut spots, &[60.0], &pose, None);
    assert_eq!(spots, [[0.0; 3]]);
    pull(&diagram, &mut [], &[], &pose, None);
}

#[test]
fn nodes_pushed_together_are_parted_never_along_their_ranks() {
    let graph = super::super::Graph {
        widths: vec![60.0; 3],
        groups: vec![None; 3],
        edges: Vec::new(),
        written: Vec::new(),
        ids: vec!["a".into(), "b".into(), "c".into()],
    };
    // Two at one spot part across, the third, a rank below, stays.
    let mut spots = vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, -500.0, 0.0]];
    part(&graph, &mut spots, Some(1));
    assert!(
        (spots[1][0] - spots[0][0] - 120.0).abs() < 1e-6,
        "{spots:?}"
    );
    assert_eq!((spots[0][1], spots[1][1]), (0.0, 0.0));
    assert_eq!(spots[2], [0.0, -500.0, 0.0]);
    // Without ranks, apart the way they stand.
    let mut spots = vec![[0.0, 0.0, 0.0], [0.0, 50.0, 0.0], [0.0, -500.0, 0.0]];
    part(&graph, &mut spots, None);
    assert!(
        (spots[1][1] - spots[0][1] - 120.0).abs() < 1e-6,
        "{spots:?}"
    );
}

/// A diagram whose targets stand at `targets`, with an edge to each.
fn pointing_at(targets: &[[f32; 3]]) -> Diagram {
    let mut body = String::new();
    for (index, _) in targets.iter().enumerate() {
        body.push_str(&format!(
            "t{index} := object thing-{index}\nn --> t{index}\n"
        ));
    }
    let (mut diagram, _) = parse(&body);
    for (target, at) in diagram.world.iter_mut().zip(targets) {
        target.state = State::Here;
        target.at = Some(*at);
    }
    diagram
}

#[test]
fn near_references_stands_a_volume_between_whoever_asked_and_its_targets_or_by_them() {
    let head = Pose::upright_facing([0.0, 1.6, 0.0], [0.0, 1.6, -1.0]);
    // A fire 3 m ahead, a stick a little to its left: the volume floats 1.3 m out, turned off the line to the right,
    // where there are fewer targets, facing back.
    let near =
        near_references(&pointing_at(&[[0.0, 0.5, -3.0], [-0.5, 0.5, -3.0]]), &head).unwrap();
    let at = near.position;
    assert!(
        ((at[0].powi(2) + at[2].powi(2)).sqrt() - BETWEEN).abs() < 1e-4,
        "{at:?}"
    );
    assert!(at[0] > 0.3 && at[2] < -1.0 && at[1] == 1.6, "{at:?}");
    // More to the right of their middle, it turns left.
    let left = near_references(
        &pointing_at(&[[0.5, 0.5, -3.0], [0.6, 0.5, -3.0], [-1.0, 0.5, -3.0]]),
        &head,
    )
    .unwrap();
    assert!(left.position[0] < -0.3, "{:?}", left.position);
    // Far off, it waits by them, on whoever asked's side.
    let far = near_references(&pointing_at(&[[0.0, 0.5, -20.0]]), &head).unwrap();
    assert!(
        (far.position[2] + 18.5).abs() < 1e-4 && far.position[0].abs() < 1e-4,
        "{far:?}"
    );
    // Spread too wide about them, straight above them, or none here: none, so it goes beside them.
    assert_eq!(
        near_references(
            &pointing_at(&[[3.0, 0.5, 0.1], [-3.0, 0.5, 0.1], [0.0, 0.5, -3.0]]),
            &head
        ),
        None
    );
    assert_eq!(
        near_references(&pointing_at(&[[0.0, 3.0, 0.0]]), &head),
        None
    );
    let mut absent = pointing_at(&[[0.0, 0.5, -3.0]]);
    absent.world[0].state = State::Absent;
    assert_eq!(near_references(&absent, &head), None);
}

#[test]
fn a_target_far_off_pulls_barely_and_one_near_firmly() {
    assert_eq!(stiffness(0.0), 1.0);
    assert_eq!(stiffness(2.0), 0.5);
    assert!(stiffness(15.0) < 0.02);
    assert_eq!(key("The Notice Board "), "notice board");
    assert_eq!(quoted(" a \"b\" "), "\"a 'b'\"");
    assert_eq!(listed(&["a", "b", "c"]), "a, b and c");
}

#[test]
fn several_things_of_one_name_are_told_apart_by_their_first_and_many_are_listed_by_their_tool() {
    let mut things = campsite();
    things.panels = vec![
        thing(&["Notes", "panel-1"], Some([0.0; 3])),
        thing(&["panel-2", "notes"], Some([1.0; 3])),
    ];
    let (diagram, _, warnings) = resolved("n := panel Notes\nx --> n\n", &things);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(diagram.world[0].at, Some([0.0; 3]));
    things.panels = (0..8)
        .map(|index| thing(&[&format!("panel-{index}x")], Some([0.0; 3])))
        .collect();
    things.anchors = (0..8)
        .map(|index| thing(&[&format!("rack-{index}x")], Some([0.0; 3])))
        .collect();
    let (_, _, warnings) = resolved(
        "p := panel Agenda\na := anchor somewhere\nx --> p & a\n",
        &things,
    );
    assert_eq!(
        warnings,
        [
            "p: no panel \"Agenda\" in this space (panels lists them); nothing is drawn to it",
            "a: no anchor \"somewhere\" in this space (presence lists them); nothing is drawn to it",
        ]
    );
    // The agent is found by itself, and a kind not said is tried as each in turn, so neither is a list.
    assert_eq!(things.of(Kind::Agent).len(), 1);
    assert!(things.of(Kind::Unknown).is_empty());
}
