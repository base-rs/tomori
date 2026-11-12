# `tomori-highlight`

Syntax highlighting for [Tomori](https://github.com/base-rs/tomori)'s code panels, on the server. Given a block of code and its language, it returns spans of a small set of shared token kinds, `tomori_protocol::TokenKind`: keyword, string, comment, number, function, type, variable, operator, punctuation, constant, attribute and plain. A client is sent the text and its spans, never colours, and maps the kinds onto one palette, so headsets never parse code, and code reads alike whichever highlighter classified it.

- **tree-sitter** highlights Rust, Ruby, JavaScript, TypeScript and TSX, Python, JSON, YAML, Markdown (its block structure), Bash and TOML, each with its official grammar crate and that grammar's own highlights query.
- **syntect** highlights every other language its default syntaxes know, such as C, C++, C#, Go, Java, HTML, CSS, SQL, PHP, Haskell or Lua.
- Any other language, or none, is plain.

Code is untrusted: only its first 16 KiB are highlighted, within 50 ms of its grammar or syntax being ready, which it is after the first use or `prepare`, and at most 4,096 spans are returned, each inside one line, so that a pathological block costs little and leaves the rest plain.

## Usage

```sh
cargo add tomori-highlight
```

```rust
use tomori_highlight::{Engine, Span, TokenKind, highlight};

let highlighted = highlight("rust", "fn main() {}");
assert_eq!(highlighted.engine, Engine::TreeSitter);
assert_eq!(highlighted.spans[0], Span::new(0, 2, TokenKind::Keyword));
```

`tomori-server` uses it, with its default `highlight` feature, to project the agent's code blocks as code panels; see `design/projection.md`. Its interfaces change with the shared code toolkit ([#147](https://github.com/base-rs/tomori/issues/147)).

## Licence

The code is under the [MIT License](license.md). Its dependencies are under the MIT License too: tree-sitter, tree-sitter-highlight and each grammar crate, by their authors, and syntect, whose default syntaxes, which it embeds, are Sublime Text's syntax definitions from [sublimehq/Packages](https://github.com/sublimehq/Packages), under that repository's permissive licence ("Permission to copy, use, modify, sell and distribute this software is granted").

## See Also

- [Tomori](https://github.com/base-rs/tomori), the conversational spatial workspace these crates belong to.
- [`tomori-server`](https://crates.io/crates/tomori-server), which highlights the agent's code panels with it.
- [`tomori-protocol`](https://crates.io/crates/tomori-protocol), whose `TokenKind` and `Span` it returns.
