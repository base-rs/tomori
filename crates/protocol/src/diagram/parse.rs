// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A `diagram` block's body, shaped like a Mermaid flowchart, a line at a time: nodes with their shapes from their
//! brackets, `@{ … }` attributes, edges with their styles and labels, `subgraph … end` groups, `classDef` and `class`,
//! and `%%` comments. A line that cannot be read is skipped, saying so, and the rest read. See `design/diagrams.md`.
use super::{
    Arrow, Attributes, Builder, Diagram, EdgeDraft, GroupKind, Layout, Shape, Style, Warning,
    clean, group_kind,
};

/// The diagram `body`, a `diagram` block's body after its header, describes, and what was left out or changed reading
/// it, each with its line, counted from 1 within `body`.
///
/// ```
/// use tomori_protocol::diagram::{Shape, Style, parse};
///
/// let (diagram, warnings) = parse("octree((Octree))@{ size: 3, colour: teal }\nrenderer[Renderer] -.-> octree : queries\n");
/// assert!(warnings.is_empty());
/// assert_eq!(diagram.nodes[0].shape, Shape::Sphere);
/// assert_eq!(diagram.nodes[0].size, 3);
/// assert_eq!((diagram.edges[0].from, diagram.edges[0].to), (1, 0));
/// assert_eq!((diagram.edges[0].style, diagram.edges[0].label.as_str()), (Style::Dashed, "queries"));
/// ```
pub fn parse(body: &str) -> (Diagram, Vec<Warning>) {
    let mut reader = Reader::default();
    for (index, line) in body.lines().enumerate() {
        reader.builder.at_line(index + 1);
        if let Err(why) = reader.line(line.trim().trim_end_matches(';').trim_end()) {
            reader.builder.warn(why);
        }
    }
    reader.finish()
}

/// What is read of a block so far.
#[derive(Default)]
struct Reader {
    builder: Builder,
    /// The groups the line being read is in, the innermost last.
    open: Vec<usize>,
    /// Each class `classDef` names, and its attributes.
    classes: Vec<(String, Attributes, usize)>,
    /// Each node a class was given to, its class and the line it was given on.
    given: Vec<(String, String, usize)>,
    /// Whether a line other than a comment has been read, so that only a first line says `flowchart` or `graph`.
    begun: bool,
}

impl Reader {
    fn group(&self) -> Option<usize> {
        self.open.last().copied()
    }

    fn line(&mut self, line: &str) -> Result<(), String> {
        if line.is_empty() || line.starts_with("%%") {
            return Ok(());
        }
        let first = !std::mem::replace(&mut self.begun, true);
        let (word, rest) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(word, rest)| (word, rest.trim()));
        match word {
            "flowchart" | "graph" if first => {
                if let Some(layout) = Layout::parse(line) {
                    self.builder.set_layout(layout);
                }
                Ok(())
            }
            "subgraph" => {
                let (id, label) = subgraph(rest);
                let index = self
                    .builder
                    .group(&id, &label, GroupKind::Cluster, self.group());
                self.open.extend(index);
                Ok(())
            }
            "end" if rest.is_empty() => self
                .open
                .pop()
                .map(|_| ())
                .ok_or_else(|| "end without a subgraph to end".to_owned()),
            "direction" if self.open.is_empty() => {
                let layout = Layout::parse(rest)
                    .ok_or_else(|| format!("\"{}\" is not a direction", clean(rest)))?;
                self.builder.set_layout(layout);
                Ok(())
            }
            // A group's own direction is taken from its parent's in this version.
            "direction" => Ok(()),
            "kind:" => {
                let kind = group_kind(rest).ok_or_else(|| {
                    format!(
                        "\"{}\" is not a kind of group: a cluster, layer or container",
                        clean(rest)
                    )
                })?;
                let group = self.group().ok_or("kind: outside a subgraph")?;
                self.builder.group_kind(group, kind);
                Ok(())
            }
            "classDef" => {
                let (name, attributes) = rest
                    .split_once(char::is_whitespace)
                    .ok_or("classDef needs a name and attributes")?;
                let attributes = self.attributes(attributes);
                let line = self.line_number();
                self.classes.push((name.to_owned(), attributes, line));
                Ok(())
            }
            "class" => {
                let (nodes, name) = rest
                    .rsplit_once(char::is_whitespace)
                    .ok_or("class needs nodes and a class")?;
                let line = self.line_number();
                for node in nodes
                    .split(',')
                    .map(str::trim)
                    .filter(|node| !node.is_empty())
                {
                    self.given.push((node.to_owned(), name.to_owned(), line));
                }
                Ok(())
            }
            "style" | "linkStyle" | "click" => Err(format!(
                "{word} lines are not drawn: give a node a colour with @{{ colour: teal }}"
            )),
            _ => self.statement(line),
        }
    }

    /// The line being read.
    fn line_number(&self) -> usize {
        self.builder.line.unwrap_or_default()
    }

    /// The attributes of `text`, a list of `key: value` separated by commas, each value in double quotes where it holds
    /// a comma, warning of each that cannot be set.
    fn attributes(&mut self, text: &str) -> Attributes {
        let mut attributes = Attributes::default();
        for pair in split_unquoted(text, ',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let Some((key, value)) = pair.split_once(':') else {
                self.builder
                    .warn(format!("\"{}\" is not key: value", clean(pair)));
                continue;
            };
            if let Err(why) = attributes.set(key, unquoted(value.trim())) {
                self.builder.warn(why);
            }
        }
        attributes
    }

    /// A line of nodes and the edges between them, such as `a[A] --> b & c : label`, read whole before anything of it
    /// is declared, so that a line that cannot be read declares nothing.
    fn statement(&mut self, line: &str) -> Result<(), String> {
        let mut cursor = Cursor::new(line);
        let mut ends: Vec<Vec<Mention>> = Vec::new();
        let mut links: Vec<EdgeDraft> = Vec::new();
        loop {
            let mut mentions = vec![cursor.mention()?];
            while cursor.eat_space() && cursor.eat("&") {
                cursor.eat_space();
                mentions.push(cursor.mention()?);
            }
            ends.push(mentions);
            cursor.eat_space();
            if cursor.done() {
                break;
            }
            if cursor.eat(":") {
                let label = unquoted(cursor.rest().trim()).to_owned();
                let last = links.last_mut().ok_or("a label after no edge")?;
                if last.label.is_empty() {
                    last.label = label;
                }
                break;
            }
            links.push(cursor.link()?);
            if !cursor.eat_space() {
                return Err(format!("\"{}\" ends in an edge to nothing", clean(line)));
            }
        }
        let group = self.group();
        for mention in ends.iter().flatten() {
            if mention.shape.is_none() {
                self.builder.mentioned(&mention.id);
            }
            let attributes = match &mention.attributes {
                Some(text) => self.attributes(text),
                None => Attributes::default(),
            };
            let attributes = Attributes {
                label: mention.label.clone(),
                shape: mention.shape,
                ..Attributes::default()
            }
            .or(&attributes);
            self.builder.node(&mention.id, attributes, group);
            if let Some(class) = &mention.class {
                let line = self.line_number();
                self.given.push((mention.id.clone(), class.clone(), line));
            }
        }
        for (index, link) in links.iter().enumerate() {
            for from in &ends[index] {
                for to in &ends[index + 1] {
                    // Both sides' product may be millions: once the edges read are full, each is only counted.
                    if self.builder.edges_full() {
                        self.builder.edge_past();
                        continue;
                    }
                    let mut edge = link.clone();
                    edge.from.clone_from(&from.id);
                    edge.to.clone_from(&to.id);
                    self.builder.edge(edge, group);
                }
            }
        }
        Ok(())
    }

    fn finish(mut self) -> (Diagram, Vec<Warning>) {
        for (node, class, line) in std::mem::take(&mut self.given) {
            self.builder.at_line(line);
            match self.classes.iter().find(|(name, ..)| *name == class) {
                Some((_, attributes, _)) if self.builder.has_node(&node) => {
                    self.builder.node(&node, attributes.clone(), None);
                }
                Some(_) => self
                    .builder
                    .warn(format!("\"{}\" is not a node", clean(&node))),
                None => self
                    .builder
                    .warn(format!("\"{}\" is not a class", clean(&class))),
            }
        }
        if !self.open.is_empty() {
            self.builder.line = None;
            self.builder.warn(format!(
                "{} subgraph left open, ended at the end",
                self.open.len()
            ));
        }
        self.builder.finish()
    }
}

/// A subgraph's id and label from what follows `subgraph`: `ID [Label]`, `ID["Label"]`, `ID` or `"Label"`.
fn subgraph(text: &str) -> (String, String) {
    match text.find('[') {
        Some(at) => {
            let id = text[..at].trim();
            let label = unquoted(text[at + 1..].trim_end().trim_end_matches(']').trim());
            let id = if id.is_empty() { label } else { id };
            (id.to_owned(), label.to_owned())
        }
        None => {
            let label = unquoted(text);
            (label.to_owned(), label.to_owned())
        }
    }
}

/// `text` without the double quotes about it, if it has them.
fn unquoted(text: &str) -> &str {
    text.strip_prefix('"')
        .and_then(|text| text.strip_suffix('"'))
        .unwrap_or(text)
}

/// `text` split at each `separator` outside double quotes.
fn split_unquoted(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut quoted) = (0, false);
    for (at, c) in text.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == separator && !quoted {
            parts.push(&text[start..at]);
            start = at + c.len_utf8();
        }
    }
    parts.push(&text[start..]);
    parts
}

/// A node as a line mentions it: its id, and the label, shape, attributes and class it gives it there.
struct Mention {
    id: String,
    label: Option<String>,
    shape: Option<Shape>,
    attributes: Option<String>,
    class: Option<String>,
}

/// The brackets of each shape, as Mermaid's: the opening and closing, longest first.
const BRACKETS: [(&str, &str, Shape); 10] = [
    ("(((", ")))", Shape::Sphere),
    ("((", "))", Shape::Sphere),
    ("([", "])", Shape::Box),
    ("[(", ")]", Shape::Cylinder),
    ("[[", "]]", Shape::Card),
    ("{{", "}}", Shape::Prism),
    ("[", "]", Shape::Box),
    ("(", ")", Shape::Box),
    ("{", "}", Shape::Diamond),
    (">", "]", Shape::Box),
];

/// Where a line is being read.
struct Cursor<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, at: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }

    fn done(&self) -> bool {
        self.at == self.text.len()
    }

    /// Skip spaces, returning whether anything follows them.
    fn eat_space(&mut self) -> bool {
        let rest = self.rest();
        self.at += rest.len() - rest.trim_start().len();
        !self.done()
    }

    /// Skip `text` if it is next.
    fn eat(&mut self, text: &str) -> bool {
        let found = self.rest().starts_with(text);
        if found {
            self.at += text.len();
        }
        found
    }

    /// Everything up to the next `close`, skipping it, or `None` where it does not close.
    fn until(&mut self, close: &str) -> Option<&'a str> {
        let rest = self.rest();
        let end = rest.find(close)?;
        self.at += end + close.len();
        Some(&rest[..end])
    }

    /// A node's id: letters, digits, `_`, and `-` or `.` before one of those, so that `a-->b` is `a`, an edge and `b`.
    fn id(&mut self) -> &'a str {
        let rest = self.rest();
        let bytes = rest.as_bytes();
        let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        let mut end = 0;
        while end < bytes.len()
            && (word(bytes[end])
                || (matches!(bytes[end], b'-' | b'.')
                    && bytes.get(end + 1).is_some_and(|next| word(*next))))
        {
            end += 1;
        }
        self.at += end;
        &rest[..end]
    }

    fn mention(&mut self) -> Result<Mention, String> {
        let id = self.id();
        if id.is_empty() {
            return Err(format!("cannot read \"{}\" as a node", clean(self.rest())));
        }
        let mut mention = Mention {
            id: id.to_owned(),
            label: None,
            shape: None,
            attributes: None,
            class: None,
        };
        if let Some((open, close, shape)) = BRACKETS
            .iter()
            .find(|(open, ..)| self.rest().starts_with(open))
        {
            self.at += open.len();
            let label = self
                .until(close)
                .ok_or_else(|| format!("{open} is not closed by {close}"))?;
            mention.label = Some(unquoted(label.trim()).to_owned());
            mention.shape = Some(*shape);
        }
        if self.eat("@{") {
            let inside = self.until("}").ok_or("@{ is not closed by }")?;
            mention.attributes = Some(inside.to_owned());
        }
        if self.eat(":::") {
            mention.class = Some(self.id().to_owned());
        }
        Ok(mention)
    }

    /// An edge's link, such as `-->`, `-.->|label|` or `-- label -->`, as its style, arrows and label.
    fn link(&mut self) -> Result<EdgeDraft, String> {
        let run = self.run();
        if run.is_empty() {
            return Err(format!("cannot read \"{}\" as an edge", clean(self.rest())));
        }
        let mut edge = EdgeDraft::new("", "");
        let token = if matches!(run, "--" | "-." | "==" | "~~") && self.rest().starts_with(' ') {
            // `-- label -->`: the label, then the link it opened.
            let rest = self.rest();
            let close = ["-->", "---", ".->", "-.-", "==>", "===", "~~>", "~~~"]
                .iter()
                .filter_map(|close| rest.find(close))
                .min()
                .ok_or_else(|| format!("\"{}\" is not closed", clean(run)))?;
            edge.label = rest[..close].trim().to_owned();
            self.at += close;
            format!("{run}{}", self.run())
        } else {
            run.to_owned()
        };
        let (style, direction) =
            link(&token).ok_or_else(|| format!("\"{}\" is not an edge", clean(&token)))?;
        edge.style = style;
        edge.direction = direction;
        if self.eat("|") {
            let label = self.until("|").ok_or("|label is not closed by |")?;
            if edge.label.is_empty() {
                edge.label = unquoted(label.trim()).to_owned();
            }
        }
        Ok(edge)
    }

    /// The run of `<`, `>`, `-`, `.`, `=` and `~` next.
    fn run(&mut self) -> &'a str {
        let rest = self.rest();
        let end = rest
            .find(|c: char| !matches!(c, '<' | '>' | '-' | '.' | '=' | '~'))
            .unwrap_or(rest.len());
        self.at += end;
        &rest[..end]
    }
}

/// The style and arrows of a link written as `token`: dashes for solid, a dot among them for dashed, `=` for thick and
/// `~` for a flow, `<` at its start and `>` at its end for arrowheads.
fn link(token: &str) -> Option<(Style, Arrow)> {
    let back = token.starts_with('<');
    let forward = token.ends_with('>');
    let line = token.trim_start_matches('<').trim_end_matches('>');
    // An edge without an arrowhead takes three, as `---`, so that `--` alone opens a label.
    let style = if line.len() < if back || forward { 2 } else { 3 } {
        return None;
    } else if line.bytes().all(|byte| byte == b'-') {
        Style::Solid
    } else if line.bytes().all(|byte| matches!(byte, b'-' | b'.')) && line.starts_with('-') {
        Style::Dashed
    } else if line.bytes().all(|byte| byte == b'=') {
        Style::Thick
    } else if line.bytes().all(|byte| byte == b'~') {
        Style::Flow
    } else {
        return None;
    };
    let direction = match (back, forward) {
        (true, true) => Arrow::Both,
        (true, false) => Arrow::Back,
        (false, true) => Arrow::Forward,
        (false, false) => Arrow::None,
    };
    Some((style, direction))
}

#[cfg(test)]
mod tests;
