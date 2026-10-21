//! Compile-time configuration: [`CompileOptions`], the
//! [`AssertMessageAnnotations`] introspected-assert setting and the
//! [`SandboxOs`] a session asks for files and the environment.

use std::num::NonZeroU32;
/// Options controlling how Monty behavior diverges from plain CPython.
///
/// Consumed when code is compiled: a `MontyRun` bakes the choices into the
/// program at construction, while a `MontyRepl` stores them so every snippet
/// fed to the session compiles the same way. The VM of each run reads them
/// too, for what it resolves at runtime.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct CompileOptions {
    /// Give failed `assert` statements pytest-style introspected messages,
    /// deliberately diverging from CPython; see `limitations/assert.md`.
    /// On by default with a 120-byte operand-repr truncation.
    pub assert_message_annotations: AssertMessageAnnotations,
    /// Whether the session has an operating system to ask for files and the
    /// environment. The host's by default; see `limitations/os.md`.
    pub os: SandboxOs,
}

/// The operating system a session asks for files and the environment, set by
/// [`CompileOptions::os`].
///
/// A session without one lacks every name that only asks it, so code that
/// would suspend with such an `OsFunctionCall` fails inside the sandbox
/// instead. The clock, sleeps and random seeding are `AutoOsCalls` policies
/// and do not change here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SandboxOs {
    /// The host's: `os`, `open()` and the disk methods of `pathlib.Path`
    /// suspend with an `OsFunctionCall` that the host answers.
    #[default]
    Host,
    /// None: `import os` raises `ModuleNotFoundError`, `open` is no builtin,
    /// and the disk methods of `pathlib.Path`, `open` among them, raise
    /// `AttributeError`. The pure path methods and `Path.cwd()` stay.
    Absent,
}

/// Controls the pytest-style introspected `assert` failure messages of
/// [`CompileOptions::assert_message_annotations`].
///
/// The choice is baked in at compile time (whether the introspecting opcodes
/// are emitted) but the truncation limit is applied at runtime, so it also
/// travels with serialized sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AssertMessageAnnotations {
    /// Disable introspection; bare asserts use CPython's empty message.
    Off,
    /// Retain at most this many UTF-8 bytes per operand before any `…` suffix.
    /// Non-zero because `0` encodes [`Off`](Self::Off) on the wire.
    MaxBytes(NonZeroU32),
}

impl AssertMessageAnnotations {
    /// Operand-repr truncation used by [`Default`] and `From<bool>`.
    pub const DEFAULT_MAX_BYTES: NonZeroU32 = NonZeroU32::new(120).expect("120 is non-zero");

    /// Whether the compiler should emit introspecting assert opcodes.
    #[must_use]
    pub fn enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    /// Returns the wire value: `0` when disabled, otherwise the UTF-8 byte cap.
    #[must_use]
    pub fn max_bytes(self) -> u32 {
        match self {
            Self::Off => 0,
            Self::MaxBytes(n) => n.get(),
        }
    }

    /// Decodes the wire value: `0` is off and any other value is the byte cap.
    #[must_use]
    pub fn from_max_bytes(value: u32) -> Self {
        match NonZeroU32::new(value) {
            Some(n) => Self::MaxBytes(n),
            None => Self::Off,
        }
    }
}

impl Default for AssertMessageAnnotations {
    fn default() -> Self {
        Self::MaxBytes(Self::DEFAULT_MAX_BYTES)
    }
}

impl From<bool> for AssertMessageAnnotations {
    /// `true` enables the 120-byte default; `false` disables annotations.
    fn from(enabled: bool) -> Self {
        if enabled { Self::default() } else { Self::Off }
    }
}
