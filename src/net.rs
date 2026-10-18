//! Internal Unix-domain-socket alias.
//!
//! Unix builds use the standard library sockets. Windows builds use
//! `uds_windows`, which implements the same std-style API over the
//! AF_UNIX support shipped with Windows 10 1803 and newer. Production
//! modules must import socket types from here instead of
//! `std::os::unix::net` so both platforms compile from one source tree.

#[cfg(unix)]
pub(crate) use std::os::unix::net::{UnixListener, UnixStream};

#[cfg(windows)]
pub(crate) use uds_windows::{UnixListener, UnixStream};
