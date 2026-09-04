//! Which agent CLIs newt knows how to drive.

/// A coding-agent CLI.
///
/// The discriminants cross the C ABI and are mirrored by `AgentKind` in
/// `macos/Sources/NewtKit/TabTree.swift`. Append rather than reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
}

impl AgentKind {
    /// Every agent newt knows about, whether or not it is installed.
    pub const ALL: &'static [AgentKind] = &[AgentKind::Claude];

    /// Name of the executable to look for.
    pub fn program_name(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
        }
    }

    /// Name shown to a person.
    pub fn display_name(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
        }
    }

    pub const fn as_u8(self) -> u8 {
        match self {
            AgentKind::Claude => 0,
        }
    }

    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(AgentKind::Claude),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_through_their_abi_values() {
        for kind in AgentKind::ALL {
            assert_eq!(AgentKind::from_u8(kind.as_u8()), Some(*kind));
        }
        assert_eq!(AgentKind::from_u8(200), None);
    }
}
