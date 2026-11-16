use std::time::Duration;

use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

use crate::ui::App;

fn started() -> Headless {
    let mut app = Headless::start(EngineConfig::new(Root::of::<App>())).expect("the app starts");
    app.wait_for("text:READY").expect("the status line is shown");
    app
}

fn run(app: &mut Headless, line: &str) {
    app.type_text("id:composer", line).expect("the prompt takes the input");
    app.submit("id:composer").expect("the input submits");
}

#[test]
fn keys_reach_the_composer_without_focusing_it_first() {
    let mut app = started();
    for key in ["p", "i", "r", "o"] {
        app.press_key(key).expect("the key is pressed");
    }
    app.wait_for("text:piro").expect("the typed text is in the prompt");
}

#[test]
fn escape_leaves_the_composer_focused() {
    let mut app = started();
    app.press_key("escape").expect("the key is pressed");
    for key in ["o", "k"] {
        app.press_key(key).expect("the key is pressed");
    }
    app.wait_for("text:ok").expect("the typed text is in the prompt");
}

#[test]
fn double_bang_output_reaches_the_feed() {
    let mut app = started();
    run(&mut app, "!!echo hi");
    app.wait_for("text:hi").expect("the command's output is shown");
    assert!(app.shows("hi"));
}

#[test]
fn clear_removes_the_feed() {
    let mut app = started();
    run(&mut app, "!!echo gone");
    app.wait_for("text:gone").expect("the output is shown");
    run(&mut app, "/clear");
    app.wait_for_gone("text:gone", Duration::from_secs(5))
        .expect("the feed is cleared");
}

#[test]
fn up_recalls_the_previous_prompt() {
    let mut app = started();
    run(&mut app, "!!echo recalled");
    app.wait_for("text:recalled").expect("the output is shown");
    app.press_key("Up").expect("the key is pressed");
    assert_eq!(app.screen().count("text:!!echo recalled"), Ok(2));
}

#[test]
fn busy_guard_then_escape_cancels_the_command() {
    let mut app = started();
    run(&mut app, "!!sleep 30");
    app.wait_for("text:RUNNING").expect("the command runs");
    run(&mut app, "!!echo blocked");
    app.wait_for("text:Task already running, press Esc to cancel.")
        .expect("the busy guard answers");
    app.press_key("Escape").expect("the key is pressed");
    app.wait_for("text:READY").expect("the status line is ready again");
    assert!(app.screen().texts().iter().any(|t| t.starts_with("Command cancelled")));
}

#[test]
fn ctrl_j_adds_a_line_without_submitting() {
    let mut app = started();
    app.type_text("id:composer", "!!echo ok")
        .expect("the prompt takes the input");
    app.press_key("ctrl+j").expect("the key is pressed");
    app.wait(Duration::from_millis(200)).expect("the app settles");
    assert_eq!(app.screen().count("text:$ echo ok"), Ok(0));
    app.submit("id:composer").expect("the input submits");
    app.wait_for("text:ok").expect("the trimmed command runs");
}
