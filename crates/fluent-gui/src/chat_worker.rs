//! Background worker for local chat inference.
//!
//! Chat has its own thread and engine so it never blocks translation or the
//! GUI, and so a chat model is never mistaken for a translation model.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use fluent_core::{ChatEngine, ChatMessage, ChatOptions, ChatReply, Error};

pub enum Command {
    Send {
        job: u64,
        dir: PathBuf,
        model_label: String,
        messages: Vec<ChatMessage>,
        options: ChatOptions,
    },
    Unload,
    Shutdown,
}

pub enum Event {
    Loading {
        job: u64,
    },
    Loaded {
        job: u64,
    },
    Unloaded,
    Delta {
        job: u64,
        text: String,
    },
    Done {
        job: u64,
        reply: ChatReply,
    },
    Failed {
        job: u64,
        message: String,
        cancelled: bool,
    },
}

pub struct ChatWorkerHandle {
    pub tx: Sender<Command>,
    pub rx: Receiver<Event>,
    pub cancel: Arc<AtomicBool>,
}

impl ChatWorkerHandle {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_cancel = Arc::clone(&cancel);
        let _ = std::thread::Builder::new()
            .name("fluent-chat".to_string())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || worker_loop(rx, event_tx, ctx, thread_cancel));
        Self {
            tx,
            rx: event_rx,
            cancel,
        }
    }

    pub fn submit(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl Drop for ChatWorkerHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Command::Shutdown);
    }
}

fn send(tx: &Sender<Event>, ctx: &egui::Context, event: Event) {
    let _ = tx.send(event);
    ctx.request_repaint();
}

fn worker_loop(
    rx: Receiver<Command>,
    tx: Sender<Event>,
    ctx: egui::Context,
    cancel: Arc<AtomicBool>,
) {
    let mut engine: Option<(PathBuf, ChatEngine)> = None;
    while let Ok(command) = rx.recv() {
        match command {
            Command::Send {
                job,
                dir,
                model_label,
                messages,
                options,
            } => {
                cancel.store(false, Ordering::SeqCst);
                let reload = engine
                    .as_ref()
                    .map(|(current, _)| current != &dir)
                    .unwrap_or(true);
                if reload {
                    engine = None;
                    send(&tx, &ctx, Event::Loading { job });
                    match ChatEngine::load(&dir, model_label, &cancel) {
                        Ok(loaded) => {
                            send(&tx, &ctx, Event::Loaded { job });
                            engine = Some((dir, loaded));
                        }
                        Err(error) => {
                            let cancelled = matches!(error, Error::Cancelled);
                            send(
                                &tx,
                                &ctx,
                                Event::Failed {
                                    job,
                                    message: error.to_string(),
                                    cancelled,
                                },
                            );
                            continue;
                        }
                    }
                }
                let Some((_, active)) = engine.as_mut() else {
                    continue;
                };
                let outcome = active.reply(
                    &messages,
                    options,
                    &mut |delta| {
                        send(
                            &tx,
                            &ctx,
                            Event::Delta {
                                job,
                                text: delta.to_string(),
                            },
                        );
                    },
                    &cancel,
                );
                match outcome {
                    Ok(reply) => send(&tx, &ctx, Event::Done { job, reply }),
                    Err(error) => {
                        let cancelled = matches!(error, Error::Cancelled);
                        send(
                            &tx,
                            &ctx,
                            Event::Failed {
                                job,
                                message: error.to_string(),
                                cancelled,
                            },
                        );
                    }
                }
            }
            Command::Unload => {
                cancel.store(true, Ordering::SeqCst);
                engine = None;
                send(&tx, &ctx, Event::Unloaded);
            }
            Command::Shutdown => break,
        }
    }
}
