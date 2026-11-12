// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Volumes: panels with depth, each showing a diagram the agent describes, in a `diagram` block or with the
//! `show_diagram` tool, and the server lays out. A volume is a panel in every other way, laid out, carried, docked,
//! thrown away and kept as every panel is; its body is the diagram's outline, which a client that draws no volumes
//! shows. See `design/diagrams.md`.
use tomori_protocol::diagram::{Diagram, Layout};
use tomori_protocol::{Limits, Panel};

use super::placement::Placement;
use super::{Content, Stage, blocks};

/// The info string's language that marks a diagram block.
pub const LANGUAGE: &str = "diagram";

/// The longest diagram block, or `show_diagram`'s arguments, in bytes; a longer one is not projected.
pub const DIAGRAM_BYTES: usize = 16 * 1024;

/// The most volumes projected from one reply, among its panels.
pub const DIAGRAMS_PER_REPLY: usize = 2;

/// The longest note of a volume's warnings the agent is told, in characters.
const NOTE_CHARS: usize = 600;

impl Content {
    /// The volume `source`, a diagram block's text, describes, and where its `place` header says it goes; or `None`
    /// when it describes no node. What reading it found wrong is kept, by its line in the block, to tell the agent.
    pub fn parse_diagram(source: &str) -> Option<(Self, Option<Placement>)> {
        let header = blocks::Header::read(source, &["layout"]);
        let (mut diagram, found) = tomori_protocol::diagram::parse(&header.rest.join("\n"));
        let mut warnings: Vec<String> = found
            .into_iter()
            .map(|mut warning| {
                // By the block's lines, its header's included.
                warning.line = warning.line.map(|line| line + header.skipped);
                warning.to_string()
            })
            .collect();
        if let Some(layout) = &header.own[0] {
            match Layout::parse(layout) {
                Some(layout) => diagram.layout = layout,
                None => warnings.push(format!(
                    "layout '{}' is not one Tomori knows: layered with down, up, right, left or away, radial, force or grid; laid out layered down",
                    crate::space::one_line(layout, 32)
                )),
            }
        }
        (!diagram.nodes.is_empty()).then(|| {
            (
                Self::volume(header.title, header.id, header.meta, diagram, warnings),
                header.place,
            )
        })
    }

    /// The volume titled `title`, which the agent calls `slug`, with `meta`, showing `diagram`, not yet laid out, and
    /// what reading it found wrong.
    pub fn volume(
        title: String,
        slug: Option<String>,
        meta: Vec<(String, String)>,
        diagram: Diagram,
        warnings: Vec<String>,
    ) -> Self {
        Self {
            title,
            body: String::new(),
            meta,
            code: None,
            slug,
            image: None,
            diagram: Some(Box::new(diagram)),
            warnings,
        }
    }

    /// Resolve its diagram's world references, if it is a volume, against `things`, the space's: what the agent is told
    /// each resolved to, a line each, and, among its warnings, those that name nothing here. See `design/diagrams.md`.
    pub(crate) fn resolve(&mut self, things: &crate::diagram::world::Things) -> Vec<String> {
        let Some(diagram) = self.diagram.as_mut() else {
            return Vec::new();
        };
        let (told, warnings) = crate::diagram::world::resolve(diagram, things);
        self.warnings
            .extend(warnings.iter().map(ToString::to_string));
        told
    }

    /// Lay its diagram out, if it is a volume, from where `previous`, the diagram it changes, stood, its nodes pointing
    /// at things in the world pulled towards them where it stands at `pose`: its body then its outline, and its metadata
    /// how many warnings it has, while it has any.
    pub fn lay_out(&mut self, previous: Option<&Diagram>, pose: Option<&tomori_protocol::Pose>) {
        let Some(diagram) = self.diagram.as_mut() else {
            return;
        };
        let warnings = crate::diagram::lay_out(diagram, previous, pose);
        self.warnings
            .extend(warnings.iter().map(ToString::to_string));
        self.body = blocks::clean_body(&diagram.outline());
        self.meta.retain(|(key, _)| key != "warnings");
        if !self.warnings.is_empty() && self.meta.len() < blocks::META_ENTRIES {
            self.meta
                .push(("warnings".into(), self.warnings.len().to_string()));
        }
    }

    /// What the agent is told of the warnings of the volume it projected as `panel`, before the next words spoken, if
    /// it has any: `Tomori showed the diagram "Renderer" (id renderer, panel-3) with 2 warnings: line 7: …`, cleaned
    /// onto one line, without double quotes or square brackets, so that it cannot end its block.
    pub fn warned(&self, panel: &str) -> Option<String> {
        (!self.warnings.is_empty()).then(|| {
            let id = self
                .slug
                .as_ref()
                .map_or(String::new(), |slug| format!("id {slug}, "));
            format!(
                "Tomori showed the diagram \"{}\" ({id}{panel}) with {} warnings: {}",
                crate::space::one_line(&self.title, blocks::TITLE_CHARS),
                self.warnings.len(),
                crate::space::one_line(
                    &tomori_protocol::diagram::told(
                        &self
                            .warnings
                            .iter()
                            .map(|text| tomori_protocol::diagram::Warning {
                                line: None,
                                text: text.clone(),
                            })
                            .collect::<Vec<_>>()
                    )
                    .replace('\n', "; "),
                    NOTE_CHARS
                )
            )
        })
    }
}

impl Stage {
    /// The diagram of the volume the agent calls `slug` in `environment`, as it stands, from which a block or tool
    /// with its id lays the volume out again.
    pub(crate) fn named_diagram(&self, environment: &str, slug: &str) -> Option<&Diagram> {
        self.here(environment)
            .find(|kept| kept.content.code.is_none() && kept.content.slug.as_deref() == Some(slug))
            .and_then(|kept| kept.content.diagram.as_deref())
    }
}

/// The most edges of a node pointed at the agent is told of, beyond which they are counted.
const NODE_EDGES: usize = 8;

/// The node `node`, by its id, or else the world edge at `world_edge` or the edge at `edge` of `diagram`, as the agent is
/// told of one pointed at: what it is called, such as `node "Octree" (id octree)`, `world edge "1. Find the fire → The
/// bonfire: here" (world edge 0)` or `edge "Renderer → Octree: queries" (edge 3)`, and what it says of it, its shape,
/// group, edges, detail and reference, both ends of a world edge, as the agent named them, or an edge's style, every
/// piece of text in quotes and cleaned onto one line, as it is the agent's own and may be from what it read. `None`
/// where it names none of them.
pub(crate) fn part(
    diagram: &Diagram,
    node: Option<&str>,
    edge: Option<u16>,
    world_edge: Option<u16>,
) -> Option<(String, String)> {
    use tomori_protocol::diagram::{Shape, Style};
    let clean = |text: &str, limit: usize| crate::space::one_line(text, limit);
    let group = |group: Option<u8>| {
        group
            .and_then(|group| diagram.groups.get(usize::from(group)))
            .map(|group| {
                let label = if group.label.is_empty() {
                    &group.id
                } else {
                    &group.label
                };
                format!(", in group \"{}\"", clean(label, 32))
            })
            .unwrap_or_default()
    };
    if let Some(index) = node.and_then(|node| diagram.node(node)) {
        let found = &diagram.nodes[index];
        let shape = match found.shape {
            Shape::Sphere => "sphere",
            Shape::Cylinder => "cylinder",
            Shape::Prism => "prism",
            Shape::Diamond => "diamond",
            Shape::Card => "card",
            _ => "box",
        };
        let mut about = format!("{shape}{}", group(found.group));
        let edges: Vec<String> = diagram
            .edges
            .iter()
            .filter_map(|edge| {
                let (way, other) = if usize::from(edge.to) == index {
                    ("from", edge.from)
                } else if usize::from(edge.from) == index {
                    ("to", edge.to)
                } else {
                    return None;
                };
                let other = diagram.nodes.get(usize::from(other))?;
                let label = if edge.label.is_empty() {
                    String::new()
                } else {
                    format!(" \"{}\"", clean(&edge.label, 24))
                };
                Some(format!("{way} {}{label}", clean(&other.label, 32)))
            })
            // And what it points at in the world, by its reference.
            .chain(
                diagram
                    .world_edges
                    .iter()
                    .filter(|edge| usize::from(edge.from) == index)
                    .filter_map(|edge| diagram.world.get(usize::from(edge.to)))
                    .map(|target| {
                        format!(
                            "to {} \"{}\"",
                            target.reference.kind.name(),
                            clean(&target.reference.name, tomori_protocol::world::NAME_BYTES)
                        )
                    }),
            )
            .collect();
        if !edges.is_empty() {
            let more = edges.len().saturating_sub(NODE_EDGES);
            about.push_str(&format!(
                "; edges: {}",
                edges[..edges.len().min(NODE_EDGES)].join(", ")
            ));
            if more > 0 {
                about.push_str(&format!(" and {more} more"));
            }
        }
        if !found.detail.is_empty() {
            about.push_str(&format!("; detail: \"{}\"", clean(&found.detail, 240)));
        }
        if !found.reference.is_empty() {
            about.push_str(&format!("; ref: \"{}\"", clean(&found.reference, 256)));
        }
        // An id holds nothing to clean, but its letters, digits, `-`, `_` and `.`.
        let named = format!("node \"{}\" (id {})", clean(&found.label, 32), found.id);
        return Some((named, about));
    }
    if let Some(index) = world_edge.map(usize::from)
        && let Some(name) = diagram.world_edge_name(index, false)
    {
        // Both its ends: its node, as the agent wrote it, and what it points at, as the agent named it.
        let found = &diagram.world_edges[index];
        let node = &diagram.nodes[usize::from(found.from)];
        let target = &diagram.world[usize::from(found.to)];
        return Some((
            format!("world edge \"{}\" (world edge {index})", clean(&name, 96)),
            format!(
                "from node {} to {} \"{}\"",
                node.id,
                target.reference.kind.name(),
                clean(&target.reference.name, tomori_protocol::world::NAME_BYTES)
            ),
        ));
    }
    let index = usize::from(edge?);
    let name = diagram.edge_name(index)?;
    let found = &diagram.edges[index];
    let style = match found.style {
        Style::Dashed => "dashed",
        Style::Dotted => "dotted",
        Style::Thick => "thick",
        Style::Flow => "a flow",
        _ => "solid",
    };
    Some((
        format!("edge \"{}\" (edge {index})", clean(&name, 96)),
        style.to_owned(),
    ))
}

/// `panels`, as a client is sent them: the diagrams of as many volumes as it states it draws, and of as many nodes
/// together, the first in order, without their world edges unless it draws them, `world_edges`, and every other volume
/// as its outline, a text panel, as a client that draws none is sent every one. Either way, the outline lists the world
/// edges.
pub fn within_volumes(
    panels: Vec<Panel>,
    diagrams: bool,
    world_edges: bool,
    limits: &Limits,
) -> Vec<Panel> {
    let (mut volumes, mut nodes) = (0, 0);
    panels
        .into_iter()
        .map(|mut panel| {
            if let Some(diagram) = &panel.diagram {
                let count = diagram.nodes.len() as u32;
                if diagrams && volumes < limits.volumes && nodes + count <= limits.nodes {
                    volumes += 1;
                    nodes += count;
                    if !world_edges && !diagram.world.is_empty() {
                        let mut plain = (**diagram).clone();
                        plain.world.clear();
                        plain.world_edges.clear();
                        panel.diagram = Some(Box::new(plain));
                    }
                } else {
                    panel.diagram = None;
                }
            }
            panel
        })
        .collect()
}

#[cfg(test)]
mod tests;
