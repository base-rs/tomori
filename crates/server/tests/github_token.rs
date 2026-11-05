// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Finding the GitHub environment's token with a command, as `gh auth token`, in a process of its own: a child started
//! beside the other tests would hold their locked state directories open. See `design/github.md`.

use std::process::Command;

use tomori_server::github::Token;

const TOKEN: &str = "ghp_FAKETOKENFORTESTS0123456789";

#[test]
fn a_token_is_found_with_a_command_whose_failures_never_show_it() {
    let printed = Token::find(
        None,
        Command::new("sh").args(["-c", &format!("echo {TOKEN}")]),
    )
    .unwrap();
    assert_eq!(printed, Token::new(TOKEN).unwrap());
    // An empty variable falls back to the command.
    let printed = Token::find(
        Some("".into()),
        Command::new("sh").args(["-c", &format!("echo {TOKEN}")]),
    )
    .unwrap();
    assert_eq!(printed, Token::new(TOKEN).unwrap());
    let errors = [
        (Command::new("/nonexistent/gh"), "(running gh failed: "),
        (Command::new("false"), "(`gh auth token` failed)"),
        (Command::new("true"), "(`gh auth token` printed no token)"),
    ];
    for (mut command, said) in errors {
        let error = Token::find(None, &mut command).unwrap_err();
        assert!(error.starts_with("--github-repository needs a token: set GITHUB_TOKEN, or sign in with `gh auth login`"), "{error}");
        assert!(error.contains(said), "{error}");
        assert!(!error.contains(TOKEN));
    }
    // The server's own lookup reads the environment and runs `gh`, whatever they hold here, and never shows a token.
    if let Err(error) = Token::from_environment() {
        assert!(
            error.starts_with("--github-repository needs a token"),
            "{error}"
        );
    }
    // A command that prints something that is not a token, such as an error on its standard output, is refused without repeating it.
    let error = Token::find(
        None,
        Command::new("sh").args(["-c", "echo 'not logged in'"]),
    )
    .unwrap_err();
    assert!(
        error.ends_with("(`gh auth token` printed no token)"),
        "{error}"
    );
    assert!(!error.contains("logged"));
}
