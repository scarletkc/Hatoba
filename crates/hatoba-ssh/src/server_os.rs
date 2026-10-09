//! The server's operating system, as far as its identification string tells (HOST-11).

/// An operating system recognised from an SSH identification string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerOs {
    Ubuntu,
    Debian,
    Raspbian,
    FreeBsd,
    NetBsd,
    Windows,
}

impl ServerOs {
    /// The stable lowercase id that is stored and handed to the UI, such as `ubuntu`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ubuntu => "ubuntu",
            Self::Debian => "debian",
            Self::Raspbian => "raspbian",
            Self::FreeBsd => "freebsd",
            Self::NetBsd => "netbsd",
            Self::Windows => "windows",
        }
    }
}

/// Comment prefixes that OS packages of OpenSSH append to the version, checked in order.
const COMMENT_PREFIXES: [(&str, ServerOs); 4] = [
    ("raspbian", ServerOs::Raspbian),
    ("debian", ServerOs::Debian),
    ("freebsd", ServerOs::FreeBsd),
    ("netbsd", ServerOs::NetBsd),
];

/// Recognises the operating system from an identification string such as
/// `SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5` (RFC 4253 §4.2:
/// `SSH-protoversion-softwareversion SP comments`).
///
/// Only servers that say which system they run on are recognised: the OpenSSH packages of
/// Ubuntu, Debian, Raspbian, FreeBSD and NetBSD, which add a comment, and Win32-OpenSSH, which
/// names itself `OpenSSH_for_Windows`. Plain `OpenSSH_9.x` (Fedora, Arch, macOS, OpenBSD and
/// others), Dropbear and every other server give `None`.
pub fn server_os(server_id: &str) -> Option<ServerOs> {
    let (_proto, software) = server_id.strip_prefix("SSH-")?.split_once('-')?;
    let software = software.to_ascii_lowercase();
    let mut words = software.split_whitespace();
    if words.next()?.starts_with("openssh_for_windows") {
        return Some(ServerOs::Windows);
    }
    let comments: Vec<&str> = words.collect();
    // Ubuntu's Debian revision always carries `ubuntu`, also where the comment still said
    // `Debian` (12.04 and older), so it is checked before the prefixes.
    if comments.iter().any(|w| w.contains("ubuntu")) {
        return Some(ServerOs::Ubuntu);
    }
    comments.iter().find_map(|w| {
        COMMENT_PREFIXES
            .iter()
            .find(|(prefix, _)| w.starts_with(prefix))
            .map(|&(_, os)| os)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(id: &str) -> Option<&'static str> {
        server_os(id).map(ServerOs::as_str)
    }

    #[test]
    fn distribution_builds_are_recognised() {
        for (id, want) in [
            ("SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5", "ubuntu"),
            ("SSH-2.0-OpenSSH_8.9p1 Ubuntu-3ubuntu0.10", "ubuntu"),
            ("SSH-2.0-OpenSSH_8.2p1 Ubuntu-4ubuntu0.11", "ubuntu"),
            ("SSH-2.0-OpenSSH_7.6p1 Ubuntu-4ubuntu0.7", "ubuntu"),
            ("SSH-2.0-OpenSSH_5.9p1 Debian-5ubuntu1.10", "ubuntu"),
            ("SSH-1.99-OpenSSH_5.3p1 Debian-3ubuntu7", "ubuntu"),
            ("SSH-2.0-OpenSSH_9.2p1 Debian-2+deb12u3", "debian"),
            ("SSH-2.0-OpenSSH_8.4p1 Debian-5+deb11u3", "debian"),
            ("SSH-2.0-OpenSSH_7.9p1 Debian-10+deb10u2", "debian"),
            ("SSH-2.0-OpenSSH_6.0p1 Debian-4+deb7u2", "debian"),
            ("SSH-2.0-OpenSSH_7.9p1 Raspbian-10+deb10u2", "raspbian"),
            ("SSH-2.0-OpenSSH_8.4p1 Raspbian-5+deb11u3", "raspbian"),
            ("SSH-2.0-OpenSSH_9.3 FreeBSD-20230719", "freebsd"),
            ("SSH-2.0-OpenSSH_7.8 FreeBSD-20180909", "freebsd"),
            (
                "SSH-2.0-OpenSSH_9.6 NetBSD_Secure_Shell-20240104-hpn13v14-lpk",
                "netbsd",
            ),
            ("SSH-2.0-OpenSSH_for_Windows_9.5", "windows"),
            ("SSH-2.0-OpenSSH_for_Windows_8.1", "windows"),
            ("SSH-2.0-OpenSSH_for_Windows_7.7", "windows"),
        ] {
            assert_eq!(os(id), Some(want), "{id}");
        }
    }

    #[test]
    fn servers_that_do_not_say_are_unknown() {
        for id in [
            "SSH-2.0-OpenSSH_9.6",
            "SSH-2.0-OpenSSH_8.7",
            "SSH-2.0-OpenSSH_9.8",
            "SSH-2.0-OpenSSH_10.0",
            "SSH-2.0-dropbear_2022.83",
            "SSH-2.0-dropbear",
            "SSH-2.0-ROSSSH",
            "SSH-2.0-Cisco-1.25",
            "SSH-2.0-Go",
            "SSH-2.0-",
            "SSH-2.0",
            "OpenSSH_9.6p1 Ubuntu-3ubuntu13.5",
            "",
        ] {
            assert_eq!(os(id), None, "{id}");
        }
    }

    #[test]
    fn only_comments_and_the_windows_build_count() {
        // The OS is only taken from the comments, never from the software version itself.
        assert_eq!(os("SSH-2.0-debian_ssh"), None);
        assert_eq!(os("SSH-2.0-OpenSSH_9.6p1 ubuntu"), Some("ubuntu"));
        assert_eq!(os("SSH-2.0-OpenSSH_9.6 for_Windows"), None);
    }
}
