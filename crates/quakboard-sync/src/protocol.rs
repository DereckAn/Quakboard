//! Which version of the sync protocol a device speaks. Every frame carries
//! its sender's (see `transport`), so devices that can no longer work
//! together refuse each other's frames instead of misreading them, and can
//! say which one needs updating.
//!
//! The stamp travels outside the encryption, so anyone on the LAN can forge
//! it. A forged one can only get a frame dropped or show a stale notice;
//! nothing that's saved is decided from it.

use std::{fmt, io};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protocol {
    pub version: u32,
    /// Oldest version this device still works with.
    pub min_peer: u32,
}

impl Protocol {
    /// What this build speaks. Bump `version` with any change older devices
    /// would misread, and `min_peer` once older devices can't keep up.
    pub const CURRENT: Protocol = Protocol {
        version: 1,
        min_peer: 1,
    };

    /// Whether a device speaking `theirs` can work with this one.
    pub fn check(self, theirs: Protocol) -> Result<(), Incompatible> {
        if theirs.version < self.min_peer {
            Err(Incompatible::PeerTooOld)
        } else if self.version < theirs.min_peer {
            Err(Incompatible::ThisTooOld)
        } else {
            Ok(())
        }
    }
}

/// What v1.5.x devices speak; they don't send a protocol at all.
impl Default for Protocol {
    fn default() -> Self {
        Protocol {
            version: 1,
            min_peer: 1,
        }
    }
}

/// Which side of an incompatible pair needs updating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Incompatible {
    /// The other device runs a Quakboard too old for this one.
    PeerTooOld,
    /// The other device needs a newer Quakboard than this one.
    ThisTooOld,
}

impl Incompatible {
    /// The refusal inside an error from `transport::read_frame`, if that's
    /// why it failed.
    pub fn of(error: &io::Error) -> Option<Incompatible> {
        error.get_ref()?.downcast_ref().copied()
    }
}

impl fmt::Display for Incompatible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Incompatible::PeerTooOld => {
                write!(f, "the other device has an older Quakboard; update it")
            }
            Incompatible::ThisTooOld => {
                write!(
                    f,
                    "the other device needs a newer Quakboard; update this one"
                )
            }
        }
    }
}

impl std::error::Error for Incompatible {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A future device that no longer works with this build.
    const FUTURE: Protocol = Protocol {
        version: 2,
        min_peer: 2,
    };

    #[test]
    fn same_protocol_is_compatible() {
        assert_eq!(Protocol::CURRENT.check(Protocol::CURRENT), Ok(()));
    }

    #[test]
    fn device_older_than_we_accept_is_told_to_update() {
        assert_eq!(
            FUTURE.check(Protocol::default()),
            Err(Incompatible::PeerTooOld)
        );
    }

    #[test]
    fn device_that_needs_a_newer_us_says_so() {
        assert_eq!(
            Protocol::CURRENT.check(FUTURE),
            Err(Incompatible::ThisTooOld)
        );
    }

    #[test]
    fn newer_device_that_still_accepts_us_is_compatible() {
        let newer = Protocol {
            version: 2,
            min_peer: 1,
        };
        assert_eq!(Protocol::CURRENT.check(newer), Ok(()));
    }

    #[test]
    fn refusal_is_found_inside_an_io_error() {
        let error = io::Error::new(io::ErrorKind::InvalidData, Incompatible::ThisTooOld);
        assert_eq!(Incompatible::of(&error), Some(Incompatible::ThisTooOld));
    }
}
