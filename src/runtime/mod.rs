//! Container runtime module for ZeptoClaw
//!
//! This module provides container isolation for shell command execution.
//! It supports multiple runtimes:
//! - Native: Direct execution (no isolation, uses application-level security)
//! - Docker: Docker container isolation (Linux, macOS, Windows)
//! - Apple Container: Apple's native container technology (macOS only)

pub mod docker;
pub mod native;
pub mod types;

pub use docker::DockerRuntime;
pub use native::NativeRuntime;
pub use types::{CommandOutput, ContainerConfig, ContainerRuntime, RuntimeError, RuntimeResult};
