//! Real local-chat integration, gated by `FLUENT_CHAT_MODEL`.
//!
//! ```text
//! FLUENT_CHAT_MODEL="$HOME/.local/share/fluent/models/qwen2.5-1.5b-instruct" \
//!     cargo test --release -p fluent-core --test chat -- --nocapture
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use fluent_core::{ChatEngine, ChatMessage, ChatOptions, ChatRole};

fn model_dir() -> Option<PathBuf> {
    std::env::var_os("FLUENT_CHAT_MODEL")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

#[test]
fn chat_replies_deterministically_and_cancels() {
    let Some(dir) = model_dir() else {
        eprintln!("skipping: set FLUENT_CHAT_MODEL to run chat tests");
        return;
    };
    let cancel = AtomicBool::new(false);
    let mut engine = ChatEngine::load(&dir, "Qwen2.5 test", &cancel).expect("load chat model");

    let messages = vec![ChatMessage::user(
        "Reply with one short sentence about the ocean.",
    )];
    let mut streamed = String::new();
    let first = engine
        .reply(
            &messages,
            ChatOptions::default(),
            &mut |delta| streamed.push_str(delta),
            &cancel,
        )
        .expect("first reply");
    println!(
        "first: {} tokens, {:?}, {:?} tok/s, text={:?}",
        first.generated_tokens, first.first_token, first.duration, first.text
    );
    assert!(!first.text.trim().is_empty());
    assert_eq!(first.text, streamed);
    assert!(!first.context_truncated);

    // Greedy decoding plus a fixed repetition penalty is reproducible.
    let second = engine
        .reply(&messages, ChatOptions::default(), &mut |_| {}, &cancel)
        .expect("repeat");
    assert_eq!(first.text, second.text);
    assert_eq!(first.generated_tokens, second.generated_tokens);

    // A follow-up sees the previous turn.
    let mut conversation = messages.clone();
    conversation.push(ChatMessage::assistant(first.text.clone()));
    conversation.push(ChatMessage::user("And why is it salty?"));
    let follow_up = engine
        .reply(&conversation, ChatOptions::default(), &mut |_| {}, &cancel)
        .expect("follow-up");
    assert!(!follow_up.text.trim().is_empty());
    assert!(follow_up.prompt_tokens > first.prompt_tokens);
    assert_ne!(follow_up.text, first.text);

    // A long generation can be cancelled after the first streamed delta.
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = Arc::clone(&cancel);
    let mut fired = false;
    let long = vec![ChatMessage::user("Write a long story about the ocean.")];
    let error = engine
        .reply(
            &long,
            ChatOptions {
                max_new_tokens: 512,
                ..Default::default()
            },
            &mut |_| {
                if !fired {
                    fired = true;
                    trigger.store(true, Ordering::SeqCst);
                }
            },
            &cancel,
        )
        .expect_err("cancellation should stop generation");
    assert!(matches!(error, fluent_core::Error::Cancelled));
    assert!(
        fired,
        "at least one delta should arrive before cancellation"
    );

    // Reloading produces the same deterministic answer.
    drop(engine);
    let cancel = AtomicBool::new(false);
    let mut reloaded = ChatEngine::load(&dir, "Qwen2.5 test", &cancel).expect("reload");
    let again = reloaded
        .reply(&messages, ChatOptions::default(), &mut |_| {}, &cancel)
        .expect("reload reply");
    assert_eq!(first.text, again.text);

    // A tiny context budget reports truncation instead of silently dropping.
    let small = reloaded
        .reply(
            &[ChatMessage::user("Tell me a long story about a library.")],
            ChatOptions {
                max_context: 40,
                max_new_tokens: 16,
                ..Default::default()
            },
            &mut |_| {},
            &cancel,
        )
        .expect("small context");
    assert!(small.context_truncated);
}

#[test]
fn chat_prompt_roles_are_ordered() {
    let messages = [
        ChatMessage::user("First"),
        ChatMessage::assistant("Second"),
        ChatMessage::user("Third"),
    ];
    let roles: Vec<ChatRole> = messages.iter().map(|message| message.role).collect();
    assert_eq!(
        roles,
        vec![ChatRole::User, ChatRole::Assistant, ChatRole::User]
    );
}
