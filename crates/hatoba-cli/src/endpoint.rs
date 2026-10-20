//! Where the running app listens for the `hatoba` command: a named pipe on Windows, named after
//! the user's SID and open to that user only; a Unix socket in the app's data folder elsewhere.
//! Debug builds use their own, so a development build runs beside the installed app.

use std::io;
use std::path::PathBuf;

pub use imp::*;

const SUFFIX: &str = if cfg!(debug_assertions) { "-dev" } else { "" };

#[cfg(unix)]
mod imp {
    use super::*;

    /// `cli.sock` in the app's data folder, which is Tauri's `app_local_data_dir`.
    pub fn endpoint(identifier: &str) -> io::Result<PathBuf> {
        let data = dirs::data_local_dir()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data folder"))?;
        Ok(data.join(identifier).join(format!("cli{SUFFIX}.sock")))
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;

    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        EqualSid, GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER, TokenUser,
    };
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::core::{PCWSTR, PWSTR};

    use super::*;

    /// `\\.\pipe\<identifier>.cli.<SID>`. Pipe names are shared by every user of the computer.
    pub fn endpoint(identifier: &str) -> io::Result<PathBuf> {
        Ok(PathBuf::from(format!(
            r"\\.\pipe\{identifier}.cli.{}{SUFFIX}",
            user_sid()?
        )))
    }

    /// The current user's SID, as `S-1-5-21-…`.
    pub fn user_sid() -> io::Result<String> {
        // SAFETY: the pseudo handle of the current process needs no closing.
        let user = TokenSid::of(unsafe { GetCurrentProcess() })?;
        let mut text = PWSTR::null();
        // SAFETY: the SID points into `user`'s buffer, which outlives the call; the string is
        // freed with LocalFree as the function requires.
        unsafe {
            ConvertSidToStringSidW(user.sid(), &mut text)?;
            let sid = text.to_string();
            LocalFree(Some(HLOCAL(text.0.cast())));
            sid.map_err(|e| io::Error::other(e.to_string()))
        }
    }

    /// Fails unless the other end of `pipe`, a client's pipe, is a process of the current user:
    /// another user could have made a pipe of the same name first.
    pub fn check_server(pipe: &File) -> io::Result<()> {
        let mut pid = 0u32;
        // SAFETY: `pipe` is an open pipe handle for the duration of the calls.
        let process = unsafe {
            GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid)?;
            Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?)
        };
        let theirs = TokenSid::of(process.0)?;
        // SAFETY: as in `user_sid`.
        let ours = TokenSid::of(unsafe { GetCurrentProcess() })?;
        // SAFETY: both SIDs point into buffers that live until the end of the function.
        if unsafe { EqualSid(theirs.sid(), ours.sid()) }.is_ok() {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the pipe belongs to another user",
            ))
        }
    }

    /// Security attributes for the pipe the app creates: only the current user may open it.
    pub struct OwnerOnly {
        descriptor: PSECURITY_DESCRIPTOR,
        attributes: SECURITY_ATTRIBUTES,
    }

    // SAFETY: the descriptor is never changed after it is made, and freed only on drop.
    unsafe impl Send for OwnerOnly {}
    unsafe impl Sync for OwnerOnly {}

    impl OwnerOnly {
        pub fn new() -> io::Result<Box<Self>> {
            // A protected DACL with one entry: all access for this user.
            let sddl: Vec<u16> = format!("D:P(A;;GA;;;{})", user_sid()?)
                .encode_utf16()
                .chain([0])
                .collect();
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            // SAFETY: `sddl` is NUL-terminated and lives through the call.
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    PCWSTR(sddl.as_ptr()),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    None,
                )?;
            }
            Ok(Box::new(Self {
                descriptor,
                attributes: SECURITY_ATTRIBUTES {
                    nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: descriptor.0,
                    bInheritHandle: false.into(),
                },
            }))
        }

        /// A `SECURITY_ATTRIBUTES` pointer for `CreateNamedPipeW`, valid while `self` lives.
        pub fn as_ptr(&self) -> *mut c_void {
            (&raw const self.attributes).cast_mut().cast()
        }
    }

    impl Drop for OwnerOnly {
        fn drop(&mut self) {
            // SAFETY: the descriptor came from LocalAlloc in the conversion above.
            unsafe {
                LocalFree(Some(HLOCAL(self.descriptor.0)));
            }
        }
    }

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: the handle is owned and closed once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// The `TOKEN_USER` of a process's token, which holds the user's SID.
    struct TokenSid(Vec<u64>);

    impl TokenSid {
        fn of(process: HANDLE) -> io::Result<Self> {
            let mut token = HANDLE::default();
            // SAFETY: `token` receives a handle that `Handle` closes.
            unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token)? };
            let token = Handle(token);
            let mut len = 0u32;
            // SAFETY: the first call only asks for the size; the buffer is u64-aligned for
            // TOKEN_USER and as long as the second call needs.
            unsafe {
                let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut len);
                let mut buf = vec![0u64; (len as usize).div_ceil(8)];
                GetTokenInformation(
                    token.0,
                    TokenUser,
                    Some(buf.as_mut_ptr().cast()),
                    len,
                    &mut len,
                )?;
                Ok(Self(buf))
            }
        }

        fn sid(&self) -> windows::Win32::Security::PSID {
            // SAFETY: the buffer holds a TOKEN_USER, whose SID points into the buffer.
            unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_belongs_to_the_app_and_the_build() {
        let path = endpoint("app.example.test").unwrap();
        let text = path.to_string_lossy();
        assert!(text.contains("app.example.test"), "{text}");
        assert_eq!(text.contains("-dev"), cfg!(debug_assertions), "{text}");
        #[cfg(windows)]
        assert!(text.starts_with(r"\\.\pipe\") && text.contains(&user_sid().unwrap()));
    }
}
