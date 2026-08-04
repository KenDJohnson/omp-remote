use std::{
    ffi::{OsStr, OsString},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<OsString>,
    pub(crate) current_dir: Option<PathBuf>,
    pub(crate) startup_timeout: Duration,
    pub(crate) request_timeout: Duration,
    pub(crate) shutdown_timeout: Duration,
    pub(crate) event_capacity: NonZeroUsize,
}

impl RuntimeConfig {
    #[must_use]
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            current_dir: None,
            startup_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(60),
            shutdown_timeout: Duration::from_secs(5),
            event_capacity: NonZeroUsize::new(256).expect("the default event capacity is non-zero"),
        }
    }

    #[must_use]
    pub fn arg(mut self, argument: impl AsRef<OsStr>) -> Self {
        self.args.push(argument.as_ref().to_owned());
        self
    }

    #[must_use]
    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args.extend(
            arguments
                .into_iter()
                .map(|argument| argument.as_ref().to_owned()),
        );
        self
    }

    #[must_use]
    pub fn current_dir(mut self, current_dir: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(current_dir.into());
        self
    }

    #[must_use]
    pub fn startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    #[must_use]
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    #[must_use]
    pub fn shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    #[must_use]
    pub fn event_capacity(mut self, capacity: NonZeroUsize) -> Self {
        self.event_capacity = capacity;
        self
    }

    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }
}
