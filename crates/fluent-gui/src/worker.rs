use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use fluent_core::{
    DeviceChoice, Engine, EngineConfig, EngineStatus, Error, Progress, TranslationRequest,
    TranslationResult,
};

pub enum Command {
    Translate {
        job: u64,
        request: TranslationRequest,
        config: EngineConfig,
    },
    Load {
        job: u64,
        config: EngineConfig,
    },
    Unload,
    Shutdown,
}

pub enum Event {
    Started {
        job: u64,
        loading: bool,
    },
    Progress {
        job: u64,
        progress: Progress,
    },
    Loaded {
        job: u64,
        status: EngineStatus,
    },
    Unloaded,
    Translated {
        job: u64,
        result: TranslationResult,
    },
    Failed {
        job: u64,
        message: String,
        cancelled: bool,
    },
}

pub struct WorkerHandle {
    pub tx: Sender<Command>,
    pub rx: Receiver<Event>,
    pub cancel: Arc<AtomicBool>,
}

impl WorkerHandle {
    pub fn spawn(ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_cancel = Arc::clone(&cancel);

        let _ = std::thread::Builder::new()
            .name("fluent-worker".to_string())
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

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Command::Shutdown);
    }
}

fn worker_loop(
    rx: Receiver<Command>,
    tx: Sender<Event>,
    ctx: egui::Context,
    cancel: Arc<AtomicBool>,
) {
    let mut engine: Option<Engine> = None;

    while let Ok(command) = rx.recv() {
        match command {
            Command::Translate {
                job,
                request,
                config,
            } => {
                cancel.store(false, Ordering::SeqCst);

                let reload = match &engine {
                    Some(engine) => requires_reload(engine.config(), &config),
                    None => true,
                };

                if reload {
                    engine = None;
                    send(&tx, &ctx, Event::Started { job, loading: true });
                    let mut fresh = Engine::new(config.clone());
                    let loaded = fresh.load(
                        &mut |progress| {
                            send(&tx, &ctx, Event::Progress { job, progress });
                        },
                        &cancel,
                    );
                    match loaded {
                        Ok(()) => {
                            send(
                                &tx,
                                &ctx,
                                Event::Loaded {
                                    job,
                                    status: fresh.status(),
                                },
                            );
                            engine = Some(fresh);
                        }
                        Err(error) => {
                            send(&tx, &ctx, failure(job, error));
                            continue;
                        }
                    }
                }

                let Some(active) = engine.as_mut() else {
                    continue;
                };
                {
                    let current = active.config_mut();
                    current.mode = config.mode;
                    current.num_beams = config.num_beams;
                    current.max_length = config.max_length;
                    current.max_new_tokens = config.max_new_tokens;
                }

                send(
                    &tx,
                    &ctx,
                    Event::Started {
                        job,
                        loading: false,
                    },
                );
                let outcome = active.translate(
                    &request,
                    &mut |progress| {
                        send(&tx, &ctx, Event::Progress { job, progress });
                    },
                    &cancel,
                );
                match outcome {
                    Ok(result) => {
                        send(&tx, &ctx, Event::Translated { job, result });
                    }
                    Err(error) => send(&tx, &ctx, failure(job, error)),
                }
            }
            Command::Load { job, config } => {
                cancel.store(false, Ordering::SeqCst);
                engine = None;
                send(&tx, &ctx, Event::Started { job, loading: true });
                let mut fresh = Engine::new(config);
                let loaded = fresh.load(
                    &mut |progress| {
                        send(&tx, &ctx, Event::Progress { job, progress });
                    },
                    &cancel,
                );
                match loaded {
                    Ok(()) => {
                        send(
                            &tx,
                            &ctx,
                            Event::Loaded {
                                job,
                                status: fresh.status(),
                            },
                        );
                        engine = Some(fresh);
                    }
                    Err(error) => send(&tx, &ctx, failure(job, error)),
                }
            }
            Command::Unload => {
                cancel.store(false, Ordering::SeqCst);
                engine = None;
                send(&tx, &ctx, Event::Unloaded);
            }
            Command::Shutdown => break,
        }
    }
}

fn failure(job: u64, error: Error) -> Event {
    let cancelled = matches!(error, Error::Cancelled);
    Event::Failed {
        job,
        message: error.to_string(),
        cancelled,
    }
}

fn send(tx: &Sender<Event>, ctx: &egui::Context, event: Event) {
    let _ = tx.send(event);
    ctx.request_repaint();
}

fn requires_reload(current: &EngineConfig, wanted: &EngineConfig) -> bool {
    current.model_dir != wanted.model_dir
        || current.device != wanted.device
        || current.precision != wanted.precision
        || device_kind(current.device) != device_kind(wanted.device)
}

fn device_kind(choice: DeviceChoice) -> u8 {
    match choice {
        DeviceChoice::Auto => 0,
        DeviceChoice::Cpu => 1,
        DeviceChoice::Cuda(_) => 2,
        DeviceChoice::Metal(_) => 3,
    }
}
