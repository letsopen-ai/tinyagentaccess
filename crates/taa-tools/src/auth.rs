//! Bearer-token material for `/mcp` and the observer (TINY-PROPOSAL, docs/design.md "Authentication").
//!
//! Two independent secrets live in owner-only files next to each other:
//! the agent token (`mcp-token`) and the observer secret (`observe-token`).
//! Each file holds 32 random bytes as 64 lowercase hex characters plus one
//! optional trailing newline. Secrets are never logged or put in the process
//! environment; callers print file *paths*.
use crate::config::Config;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use subtle::ConstantTimeEq;

pub const AGENT_FILE: &str = "mcp-token";
pub const OBSERVER_FILE: &str = "observe-token";
const SECRET_BYTES: usize = 32;
const SECRET_HEX_LEN: usize = SECRET_BYTES * 2;
/// Hex plus `\r\n` fits well inside this; anything larger is not a token file.
const MAX_FILE_BYTES: u64 = 128;

/// The two loaded secrets. `Debug` never shows them.
#[derive(Clone)]
pub struct Auth {
    agent: String,
    observer: Option<String>,
}
impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Auth { .. }")
    }
}
impl Auth {
    /// There is no unauthenticated constructor: tests pass a literal token.
    pub fn new(agent: impl Into<String>, observer: Option<String>) -> Self {
        Self {
            agent: agent.into(),
            observer,
        }
    }
    pub fn agent_matches(&self, presented: &str) -> bool {
        ct_eq(&self.agent, presented)
    }
    /// False when no observer secret is loaded (fail closed).
    pub fn observer_matches(&self, presented: &str) -> bool {
        self.observer
            .as_deref()
            .is_some_and(|s| ct_eq(s, presented))
    }
    pub fn observer_secret(&self) -> Option<&str> {
        self.observer.as_deref()
    }
}

fn ct_eq(expected: &str, presented: &str) -> bool {
    !expected.is_empty() && bool::from(expected.as_bytes().ct_eq(presented.as_bytes()))
}

/// Where the secrets live and whether the agent path was named explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenPaths {
    pub agent: PathBuf,
    pub observer: PathBuf,
    /// An explicit path is never created, so a typo cannot become a new secret.
    pub explicit: bool,
}

/// The default agent token path, shared by every instance of this user.
pub fn default_token_path() -> Result<PathBuf, String> {
    taa_platform::state_dir().map(|d| d.join(AGENT_FILE))
}

/// `--token-file` beats `[auth] token_file`, which beats the default path.
/// The observer secret always sits beside the agent token.
pub fn resolve_paths(cli: Option<&Path>, config: &Config) -> Result<TokenPaths, String> {
    let named = match cli {
        Some(p) => Some(p.to_path_buf()),
        None if !config.auth.token_file.is_empty() => Some(PathBuf::from(&config.auth.token_file)),
        None => None,
    };
    let (agent, explicit) = match named {
        Some(p) if !p.is_absolute() => return Err("token file path must be absolute".into()),
        Some(p) => (p, true),
        None => (default_token_path()?, false),
    };
    let dir = agent.parent().ok_or("token file path has no parent")?;
    Ok(TokenPaths {
        observer: dir.join(OBSERVER_FILE),
        agent,
        explicit,
    })
}

/// Loads the agent token (creating only the default path) and, when asked,
/// the observer secret (created on demand beside it). The two credentials
/// must be distinct files with distinct values; the check runs before the
/// observer secret is returned to any caller that might print it.
pub fn load(paths: &TokenPaths, observer: bool) -> Result<Auth, String> {
    if observer {
        ensure_distinct_paths(paths)?;
    }
    let agent = load_secret(&paths.agent, !paths.explicit, "agent token")?;
    let observer = if observer {
        let secret = load_secret(&paths.observer, true, "observer secret")?;
        ensure_distinct_credentials(paths, &secret)?;
        Some(secret)
    } else {
        None
    };
    Ok(Auth { agent, observer })
}

/// Loads (creating if needed) only the observer secret, for `observe-url`.
/// Applies the same agent/observer separation checks as `load`, without
/// creating the agent token.
pub fn load_observer_secret(paths: &TokenPaths) -> Result<String, String> {
    ensure_distinct_paths(paths)?;
    let secret = load_secret(&paths.observer, true, "observer secret")?;
    ensure_distinct_credentials(paths, &secret)?;
    Ok(secret)
}

const COLLISION: &str = "the agent token and the observer secret must be different files with different values; give --token-file / [auth] token_file a name other than `observe-token`, and do not link or copy one file to the other";

fn ensure_distinct_paths(paths: &TokenPaths) -> Result<(), String> {
    if paths.agent == paths.observer {
        return Err(format!(
            "agent token and observer secret share the path {}: {COLLISION}",
            paths.agent.display()
        ));
    }
    Ok(())
}

/// Rejects the same file reached through links and equal secret values.
/// Never includes secret bytes in an error.
fn ensure_distinct_credentials(paths: &TokenPaths, observer_secret: &str) -> Result<(), String> {
    let (Ok(agent), Ok(observer)) = (open_read(&paths.agent), open_read(&paths.observer)) else {
        // No agent file yet (or it cannot be opened): nothing to collide with.
        return Ok(());
    };
    if let (Some(a), Some(b)) = (file_id(&agent), file_id(&observer)) {
        if a == b {
            return Err(format!(
                "agent token file {} and observer secret file {} are the same file: {COLLISION}",
                paths.agent.display(),
                paths.observer.display()
            ));
        }
    }
    // An unreadable or malformed agent file fails its own load; only a valid
    // value can collide.
    if let Ok(agent_secret) = read_secret(&paths.agent) {
        if ct_eq(&agent_secret, observer_secret) {
            return Err(format!(
                "agent token file {} and observer secret file {} hold the same value: {COLLISION}",
                paths.agent.display(),
                paths.observer.display()
            ));
        }
    }
    Ok(())
}

/// `http://<addr>/observe#<secret>`. The fragment is never sent to a server.
pub fn observer_url(addr: &str, secret: &str) -> String {
    format!("http://{addr}/observe#{secret}")
}

fn load_secret(path: &Path, create: bool, what: &str) -> Result<String, String> {
    if create {
        ensure_parent(path).map_err(|e| format!("cannot prepare {what} directory: {e}"))?;
        match create_secret(path) {
            Ok(secret) => return Ok(secret),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("cannot create {what} file {}: {e}", path.display())),
        }
    }
    read_secret(path).map_err(|e| match e {
        ReadError::Missing => format!(
            "{what} file {} does not exist (explicit token paths are never created; omit the option to use the default path)",
            path.display()
        ),
        ReadError::Other(e) => format!("{what} file {}: {e}", path.display()),
    })
}

enum ReadError {
    Missing,
    Other(String),
}

fn read_secret(path: &Path) -> Result<String, ReadError> {
    let mut file = open_read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ReadError::Missing
        } else {
            ReadError::Other(format!("cannot open: {e}"))
        }
    })?;
    check_open_file(&file).map_err(ReadError::Other)?;
    let mut text = String::new();
    (&mut file)
        .take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| ReadError::Other(format!("cannot read: {e}")))?;
    parse_secret(&text).map_err(ReadError::Other)
}

/// Accepts exactly 64 lowercase hex characters and one optional trailing
/// `\n` or `\r\n`. Never echoes the content.
fn parse_secret(text: &str) -> Result<String, String> {
    let body = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    if body.len() == SECRET_HEX_LEN
        && body
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(body.into())
    } else {
        Err(format!(
            "malformed secret (expected {SECRET_HEX_LEN} lowercase hex characters and at most one trailing newline, no other whitespace); run `tinyagentaccess rotate-token`"
        ))
    }
}

fn random_secret() -> std::io::Result<String> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Writes `secret` to a unique owner-only sibling file and syncs it.
fn write_private_temp(path: &Path, secret: &str) -> std::io::Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::other("token file path has no file name"))?;
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", &random_secret()?[..16]));
    let written = create_owner_only(&temporary).and_then(|mut f| {
        f.write_all(format!("{secret}\n").as_bytes())?;
        f.sync_all()
    });
    match written {
        Ok(()) => Ok(temporary),
        Err(e) => {
            let _ = fs::remove_file(&temporary);
            Err(e)
        }
    }
}

/// Publishes a complete owner-only file at `path` without replacing an
/// existing one: the secret is written and synced to a temporary sibling,
/// then linked (Unix) or moved without replacement (Windows) onto the final
/// path. A reader can therefore never see an empty or partial token, and a
/// failed first write leaves no malformed file at `path`. If `path` already
/// exists the temporary file is removed and `AlreadyExists` is returned so
/// the caller reads the winner.
fn create_secret(path: &Path) -> std::io::Result<String> {
    let secret = random_secret()?;
    let temporary = write_private_temp(path, &secret)?;
    let published = publish_new(&temporary, path);
    let _ = fs::remove_file(&temporary);
    published.map(|()| secret)
}

/// Replaces the agent token with a fresh one: a sibling temporary file is
/// created owner-only and renamed over the target. An explicit path must
/// already exist. The observer secret is not touched.
pub fn rotate_agent_token(paths: &TokenPaths) -> Result<(), String> {
    let path = &paths.agent;
    if paths.explicit {
        if fs::metadata(path).is_err() {
            return Err(format!(
                "agent token file {} does not exist (explicit token paths are never created)",
                path.display()
            ));
        }
    } else {
        ensure_parent(path).map_err(|e| format!("cannot prepare token directory: {e}"))?;
    }
    let secret = random_secret().map_err(|e| e.to_string())?;
    let result = write_private_temp(path, &secret).and_then(|temporary| {
        fs::rename(&temporary, path).inspect_err(|_| {
            let _ = fs::remove_file(&temporary);
        })
    });
    result.map_err(|e| format!("cannot rotate {}: {e}", path.display()))
}

/// How restrictive a secret file's access control is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    OwnerOnly,
    TooOpen,
    Unknown,
}
impl Access {
    pub fn label(self) -> &'static str {
        match self {
            Access::OwnerOnly => "owner-only",
            Access::TooOpen => "too-open",
            Access::Unknown => "unknown",
        }
    }
}
/// Read-only facts about a secret file for `doctor`; never reads the content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub exists: bool,
    pub access: Access,
    /// Unix mode as octal, for example `0600`.
    pub mode: Option<String>,
}
pub fn inspect(path: &Path) -> FileStatus {
    let missing = FileStatus {
        exists: false,
        access: Access::Unknown,
        mode: None,
    };
    let Ok(file) = open_read(path) else {
        return FileStatus {
            exists: path.exists(),
            ..missing
        };
    };
    FileStatus {
        exists: true,
        access: if check_open_file(&file).is_ok() {
            Access::OwnerOnly
        } else {
            access_of(&file)
        },
        mode: mode_of(&file),
    }
}

#[cfg(unix)]
mod sys {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    pub fn ensure_parent(path: &Path) -> std::io::Result<()> {
        match path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir),
            _ => Ok(()),
        }
    }
    pub fn create_owner_only(path: &Path) -> std::io::Result<fs::File> {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
    /// Link-then-unlink never replaces an existing destination.
    pub fn publish_new(temporary: &Path, path: &Path) -> std::io::Result<()> {
        fs::hard_link(temporary, path)
    }
    pub fn file_id(file: &fs::File) -> Option<(u64, u64)> {
        file.metadata().ok().map(|m| (m.dev(), m.ino()))
    }
    pub fn open_read(path: &Path) -> std::io::Result<fs::File> {
        // O_NONBLOCK keeps a FIFO at the token path from hanging startup.
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
    }
    pub fn check_open_file(file: &fs::File) -> Result<(), String> {
        let meta = file.metadata().map_err(|e| format!("cannot stat: {e}"))?;
        if !meta.is_file() {
            return Err("not a regular file".into());
        }
        if meta.uid() != unsafe { libc::geteuid() } {
            return Err("not owned by the current user".into());
        }
        if meta.mode() & 0o077 != 0 {
            return Err(format!(
                "permissions {:04o} are too open; run `chmod 600` on it or `tinyagentaccess rotate-token`",
                meta.mode() & 0o777
            ));
        }
        Ok(())
    }
    pub fn access_of(file: &fs::File) -> Access {
        match file.metadata() {
            Ok(m) if m.mode() & 0o077 != 0 => Access::TooOpen,
            // Wrong owner or not a regular file: refused at startup, not "open".
            _ => Access::Unknown,
        }
    }
    pub fn mode_of(file: &fs::File) -> Option<String> {
        file.metadata()
            .ok()
            .map(|m| format!("{:04o}", m.mode() & 0o7777))
    }
}

#[cfg(windows)]
mod sys {
    //! Owner-only means a protected DACL with one allow entry for the current
    //! user, set at creation through `SECURITY_ATTRIBUTES`; the creation
    //! descriptor also names the current user as owner so an elevated token
    //! does not make Administrators the owner. Existing files pass only when
    //! the file owner is the current user and every allow entry belongs to a
    //! trusted principal (see `dacl_access`).
    use super::*;
    use std::os::windows::{ffi::OsStrExt, io::FromRawHandle};
    use std::ptr::null_mut;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE},
        Security::{
            AclSizeInformation,
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
            },
            GetAce, GetAclInformation, GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE,
            ACE_HEADER, ACL, ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION,
            OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
            TOKEN_USER,
        },
        Storage::FileSystem::{
            CreateDirectoryW, CreateFileW, GetFileInformationByHandle, MoveFileExW,
            BY_HANDLE_FILE_INFORMATION, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    // ACE_HEADER.AceType values (winnt.h); windows-sys does not export them.
    const ACCESS_ALLOWED_ACE_TYPE: u32 = 0;
    const ACCESS_DENIED_ACE_TYPE: u32 = 1;
    const ACCESS_ALLOWED_CALLBACK_ACE_TYPE: u32 = 9;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }
    fn wide_str(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    unsafe fn sid_string(sid: *mut core::ffi::c_void) -> std::io::Result<String> {
        let mut out: *mut u16 = null_mut();
        if ConvertSidToStringSidW(sid, &mut out) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut len = 0;
        while *out.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(out, len));
        LocalFree(out.cast());
        Ok(s)
    }
    fn current_user_sid() -> std::io::Result<String> {
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len);
            // u64 storage keeps the TOKEN_USER pointers aligned.
            let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
            let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
            let result = if ok == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                sid_string((*(buf.as_ptr() as *const TOKEN_USER)).User.Sid)
            };
            CloseHandle(token);
            result
        }
    }
    /// Runs `f` with attributes carrying a protected, owner-only DACL.
    fn with_attributes<T>(
        inherit: bool,
        f: impl FnOnce(*const SECURITY_ATTRIBUTES) -> std::io::Result<T>,
    ) -> std::io::Result<T> {
        let ace_flags = if inherit { "OICI" } else { "" };
        let user = current_user_sid()?;
        let sddl = wide_str(&format!("O:{user}D:P(A;{ace_flags};FA;;;{user})"));
        unsafe {
            let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let result = f(&attributes);
            LocalFree(descriptor);
            result
        }
    }
    pub fn ensure_parent(path: &Path) -> std::io::Result<()> {
        let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) else {
            return Ok(());
        };
        if dir.is_dir() {
            return Ok(());
        }
        if let Some(grandparent) = dir.parent() {
            fs::create_dir_all(grandparent)?;
        }
        let name = wide(dir);
        with_attributes(true, |attributes| {
            if unsafe { CreateDirectoryW(name.as_ptr(), attributes) } == 0 {
                let e = std::io::Error::last_os_error();
                return if dir.is_dir() { Ok(()) } else { Err(e) };
            }
            Ok(())
        })
    }
    pub fn create_owner_only(path: &Path) -> std::io::Result<fs::File> {
        let name = wide(path);
        with_attributes(false, |attributes| unsafe {
            let handle = CreateFileW(
                name.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ,
                attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            );
            if handle == INVALID_HANDLE_VALUE {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(fs::File::from_raw_handle(handle.cast()))
            }
        })
    }
    /// Moves without replacing: fails with `AlreadyExists` if `path` exists.
    pub fn publish_new(temporary: &Path, path: &Path) -> std::io::Result<()> {
        let (from, to) = (wide(temporary), wide(path));
        // dwFlags = 0: no MOVEFILE_REPLACE_EXISTING.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
            let e = std::io::Error::last_os_error();
            // ERROR_FILE_EXISTS (80) / ERROR_ALREADY_EXISTS (183)
            return Err(if matches!(e.raw_os_error(), Some(80 | 183)) {
                std::io::ErrorKind::AlreadyExists.into()
            } else {
                e
            });
        }
        Ok(())
    }
    /// Volume serial number plus file index identify one file across links.
    pub fn file_id(file: &fs::File) -> Option<(u64, u64)> {
        use std::os::windows::io::AsRawHandle;
        unsafe {
            let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) == 0 {
                return None;
            }
            Some((
                u64::from(info.dwVolumeSerialNumber),
                (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            ))
        }
    }
    pub fn open_read(path: &Path) -> std::io::Result<fs::File> {
        fs::File::open(path)
    }
    /// How `dacl_access` judged a file.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Judgement {
        Access(Access),
        ForeignOwner,
    }
    /// Reads the owner and the DACL from the same open handle.
    ///
    /// The owner must be the current user; anything else fails closed.
    /// Trusted allow-ACE principals: the current user SID, `S-1-5-18`
    /// (SYSTEM), `S-1-5-32-544` (Administrators), and `S-1-3-4`
    /// (OWNER RIGHTS). OWNER RIGHTS stands for whoever owns the file, so it
    /// is only trusted because the owner was confirmed to be the current
    /// user first. A missing DACL is everyone; unknown ACE types cannot be
    /// judged.
    fn dacl_access(file: &fs::File) -> std::io::Result<Judgement> {
        use std::os::windows::io::AsRawHandle;
        let user = current_user_sid()?;
        unsafe {
            let mut owner: *mut core::ffi::c_void = null_mut();
            let mut dacl: *mut ACL = null_mut();
            let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
            let status = GetSecurityInfo(
                file.as_raw_handle().cast(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            );
            if status != 0 {
                return Err(std::io::Error::from_raw_os_error(status as i32));
            }
            let result = (|| {
                if owner.is_null() || sid_string(owner)? != user {
                    return Ok(Judgement::ForeignOwner);
                }
                let judged = |a| -> std::io::Result<Judgement> { Ok(Judgement::Access(a)) };
                // A missing DACL grants everyone full access.
                if dacl.is_null() {
                    return judged(Access::TooOpen);
                }
                let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
                if GetAclInformation(
                    dacl,
                    (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                    std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                    AclSizeInformation,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                let trusted = [user.as_str(), "S-1-5-18", "S-1-5-32-544", "S-1-3-4"];
                for index in 0..info.AceCount {
                    let mut ace: *mut core::ffi::c_void = null_mut();
                    if GetAce(dacl, index, &mut ace) == 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    let header = ace as *const ACE_HEADER;
                    match (*header).AceType as u32 {
                        ACCESS_ALLOWED_ACE_TYPE | ACCESS_ALLOWED_CALLBACK_ACE_TYPE => {
                            let allowed = ace as *const ACCESS_ALLOWED_ACE;
                            let sid = std::ptr::addr_of!((*allowed).SidStart) as *mut _;
                            if !trusted.contains(&sid_string(sid)?.as_str()) {
                                return judged(Access::TooOpen);
                            }
                        }
                        // Deny entries only restrict access.
                        ACCESS_DENIED_ACE_TYPE => {}
                        // Object or other allow types: cannot judge them.
                        _ => return judged(Access::Unknown),
                    }
                }
                judged(Access::OwnerOnly)
            })();
            LocalFree(descriptor);
            result
        }
    }
    pub fn check_open_file(file: &fs::File) -> Result<(), String> {
        match dacl_access(file) {
            Ok(Judgement::Access(Access::OwnerOnly)) => Ok(()),
            Ok(Judgement::Access(Access::TooOpen)) => Err("access control list allows other users; restrict it to your account or run `tinyagentaccess rotate-token`".into()),
            Ok(Judgement::Access(Access::Unknown)) => Err("cannot judge the access control list of this file".into()),
            Ok(Judgement::ForeignOwner) => Err("not owned by the current user".into()),
            Err(e) => Err(format!("cannot read the access control list: {e}")),
        }
    }
    pub fn access_of(file: &fs::File) -> Access {
        match dacl_access(file) {
            Ok(Judgement::Access(a)) => a,
            _ => Access::Unknown,
        }
    }
    pub fn mode_of(_file: &fs::File) -> Option<String> {
        None
    }
}
use sys::{
    access_of, check_open_file, create_owner_only, ensure_parent, file_id, mode_of, open_read,
    publish_new,
};

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn paths(dir: &Path, explicit: bool) -> TokenPaths {
        TokenPaths {
            agent: dir.join("state/tinyagentaccess").join(AGENT_FILE),
            observer: dir.join("state/tinyagentaccess").join(OBSERVER_FILE),
            explicit,
        }
    }

    #[test]
    fn secret_format_is_strict() {
        for ok in [GOOD.to_string(), format!("{GOOD}\n"), format!("{GOOD}\r\n")] {
            assert_eq!(parse_secret(&ok).unwrap(), GOOD);
        }
        for bad in [
            String::new(),
            "\n".into(),
            GOOD[..63].into(),
            format!("{GOOD}0"),
            format!("{GOOD}\n\n"),
            format!(" {GOOD}"),
            format!("{GOOD} "),
            format!("{GOOD}\r"),
            format!("{GOOD}\n "),
            GOOD.to_uppercase(),
            format!("{}g", &GOOD[..63]),
        ] {
            let e = parse_secret(&bad).unwrap_err();
            assert!(!e.contains(GOOD), "error must not echo the secret");
        }
    }

    #[test]
    fn comparison_rejects_wrong_empty_and_other_length() {
        let auth = Auth::new(GOOD, Some("b".repeat(64)));
        assert!(auth.agent_matches(GOOD));
        assert!(!auth.agent_matches(&GOOD[..63]));
        assert!(!auth.agent_matches(&format!("{GOOD}0")));
        assert!(!auth.agent_matches(""));
        assert!(!auth.agent_matches(&"b".repeat(64)));
        assert!(auth.observer_matches(&"b".repeat(64)));
        assert!(!auth.observer_matches(GOOD));
        assert!(!Auth::new(GOOD, None).observer_matches(""));
        assert!(!Auth::new("", None).agent_matches(""));
        assert_eq!(format!("{auth:?}"), "Auth { .. }");
    }

    #[cfg(unix)]
    #[test]
    fn explicit_path_beats_config_and_must_be_absolute() {
        let mut config = Config::default();
        config.auth.token_file = "/cfg/mcp-token".into();
        let p = resolve_paths(None, &config).unwrap();
        assert_eq!(p.agent, Path::new("/cfg/mcp-token"));
        assert_eq!(p.observer, Path::new("/cfg/observe-token"));
        assert!(p.explicit);
        let p = resolve_paths(Some(Path::new("/cli/t")), &config).unwrap();
        assert_eq!(p.agent, Path::new("/cli/t"));
        assert!(resolve_paths(Some(Path::new("relative")), &config).is_err());
        let d = resolve_paths(None, &Config::default());
        if let Ok(d) = d {
            assert!(!d.explicit);
            assert!(d.agent.ends_with("tinyagentaccess/mcp-token"));
            assert_eq!(d.observer.parent(), d.agent.parent());
        }
    }

    #[test]
    fn explicit_missing_path_is_an_error_and_not_created() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), true);
        fs::create_dir_all(p.agent.parent().unwrap()).unwrap();
        let e = load(&p, false).unwrap_err();
        assert!(e.contains("does not exist"), "{e}");
        assert!(!p.agent.exists());
        assert!(rotate_agent_token(&p).is_err());
        assert!(!p.agent.exists());
    }

    #[test]
    fn default_path_is_created_once_and_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), false);
        let first = load(&p, true).unwrap();
        assert!(first.agent_matches(fs::read_to_string(&p.agent).unwrap().trim_end()));
        let observer = first.observer_secret().unwrap().to_owned();
        assert_eq!(observer.len(), 64);
        assert!(!first.agent_matches(&observer));
        let second = load(&p, true).unwrap();
        assert!(second.agent_matches(fs::read_to_string(&p.agent).unwrap().trim_end()));
        assert_eq!(second.observer_secret(), Some(observer.as_str()));
        assert_eq!(fs::read_to_string(&p.agent).unwrap().len(), 65);
        assert_eq!(load_observer_secret(&p).unwrap(), observer);
    }

    #[test]
    fn disabled_observer_creates_no_observer_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), false);
        assert!(load(&p, false).unwrap().observer_secret().is_none());
        assert!(!p.observer.exists());
    }

    #[test]
    fn malformed_file_fails_startup_without_echoing_it() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), false);
        load(&p, false).unwrap();
        fs::write(&p.agent, "secret-looking-but-bad\n").unwrap();
        restrict(&p.agent);
        let e = load(&p, false).unwrap_err();
        assert!(
            e.contains("malformed") && !e.contains("secret-looking"),
            "{e}"
        );
    }

    #[test]
    fn rotate_replaces_the_token_and_keeps_it_private() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), false);
        let old = load(&p, false).unwrap();
        let before = fs::read_to_string(&p.agent).unwrap();
        rotate_agent_token(&p).unwrap();
        let after = fs::read_to_string(&p.agent).unwrap();
        assert_ne!(before, after);
        assert_eq!(after.len(), 65);
        let new = load(&p, false).unwrap();
        assert!(!new.agent_matches(before.trim_end()));
        assert!(old.agent_matches(before.trim_end()));
        assert_eq!(inspect(&p.agent).access, Access::OwnerOnly);
        let leftovers: Vec<_> = fs::read_dir(p.agent.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, [std::ffi::OsString::from(AGENT_FILE)]);
    }

    #[test]
    fn rotate_works_for_an_existing_explicit_file() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = paths(tmp.path(), false);
        load(&p, false).unwrap();
        p.explicit = true;
        let before = fs::read_to_string(&p.agent).unwrap();
        rotate_agent_token(&p).unwrap();
        assert_ne!(before, fs::read_to_string(&p.agent).unwrap());
    }

    #[test]
    fn inspect_reports_without_reading_content() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), false);
        let s = inspect(&p.agent);
        assert!(!s.exists && s.mode.is_none());
        load(&p, false).unwrap();
        let s = inspect(&p.agent);
        assert!(s.exists);
        assert_eq!(s.access, Access::OwnerOnly);
    }

    #[cfg(unix)]
    fn restrict(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[cfg(not(unix))]
    fn restrict(_: &Path) {}

    #[cfg(unix)]
    #[test]
    fn unix_files_are_created_0600_in_a_0700_directory() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), false);
        load(&p, true).unwrap();
        let mode = |q: &Path| fs::metadata(q).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&p.agent), 0o600);
        assert_eq!(mode(&p.observer), 0o600);
        assert_eq!(mode(p.agent.parent().unwrap()), 0o700);
        let s = inspect(&p.agent);
        assert_eq!(s.mode.as_deref(), Some("0600"));
    }

    #[cfg(unix)]
    #[test]
    fn weaker_permissions_refuse_to_start() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), false);
        load(&p, true).unwrap();
        for mode in [0o640, 0o604, 0o660, 0o644, 0o666, 0o601] {
            fs::set_permissions(&p.agent, fs::Permissions::from_mode(mode)).unwrap();
            let e = load(&p, false).unwrap_err();
            assert!(e.contains("too open"), "{mode:o}: {e}");
            let s = inspect(&p.agent);
            assert_eq!(s.access, Access::TooOpen);
        }
        fs::set_permissions(&p.agent, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&p.observer, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load(&p, false).is_ok());
        assert!(load(&p, true).unwrap_err().contains("observer secret"));
        // Stricter than 0600 is fine.
        fs::set_permissions(&p.observer, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(load(&p, true).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn non_regular_files_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = paths(tmp.path(), true);
        p.agent = tmp.path().to_path_buf();
        let e = load(&p, false).unwrap_err();
        assert!(e.contains("not a regular file"), "{e}");
    }

    fn explicit_pair(dir: &Path, agent_name: &str) -> TokenPaths {
        let d = dir.join("state/tinyagentaccess");
        fs::create_dir_all(&d).unwrap();
        let config = Config::default();
        resolve_paths(Some(&d.join(agent_name)), &config).unwrap()
    }

    #[cfg(unix)]
    fn assert_rejected(p: &TokenPaths, what: &str) {
        let e = load(p, true)
            .err()
            .unwrap_or_else(|| panic!("{what}: load"));
        assert!(e.contains("different"), "{what}: {e}");
        assert!(!e.contains(GOOD), "{what}: error must not echo a secret");
        let e = load_observer_secret(p).expect_err(what);
        assert!(e.contains("different"), "{what}: {e}");
    }

    #[cfg(unix)]
    #[test]
    fn same_path_for_agent_and_observer_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let p = explicit_pair(tmp.path(), OBSERVER_FILE);
        assert_eq!(p.agent, p.observer);
        fs::write(&p.agent, format!("{GOOD}\n")).unwrap();
        restrict(&p.agent);
        assert_rejected(&p, "same path");
        // The agent alone is still loadable when the observer is off.
        assert!(load(&p, false).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_and_hard_link_aliases_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), false);
        load(&p, false).unwrap();
        std::os::unix::fs::symlink(&p.agent, &p.observer).unwrap();
        assert_rejected(&p, "symlink");
        fs::remove_file(&p.observer).unwrap();
        fs::hard_link(&p.agent, &p.observer).unwrap();
        assert_rejected(&p, "hard link");
    }

    #[cfg(unix)]
    #[test]
    fn distinct_files_with_equal_values_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), false);
        load(&p, false).unwrap();
        let value = fs::read_to_string(&p.agent).unwrap();
        fs::write(&p.observer, &value).unwrap();
        restrict(&p.observer);
        let e = load(&p, true).unwrap_err();
        assert!(
            e.contains("same value") && !e.contains(value.trim_end()),
            "{e}"
        );
        assert!(load_observer_secret(&p).unwrap_err().contains("same value"));
    }

    #[test]
    fn distinct_credentials_load_and_observe_url_does_not_create_the_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let p = explicit_pair(tmp.path(), "mcp-token");
        // observe-url with a missing explicit agent file has nothing to collide with.
        let secret = load_observer_secret(&p).unwrap();
        assert!(!p.agent.exists());
        assert_eq!(secret.len(), 64);
    }

    #[test]
    fn rotate_does_not_touch_the_observer_secret_or_link_the_files() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), false);
        let a = load(&p, true).unwrap();
        let observer = fs::read_to_string(&p.observer).unwrap();
        rotate_agent_token(&p).unwrap();
        assert_eq!(fs::read_to_string(&p.observer).unwrap(), observer);
        assert_ne!(fs::read_to_string(&p.agent).unwrap(), observer);
        assert!(load(&p, true).is_ok());
        assert_eq!(inspect(&p.agent).access, Access::OwnerOnly);
        assert!(a.observer_secret().is_some());
    }

    #[test]
    fn simultaneous_first_loads_agree_and_leave_no_malformed_file() {
        for _ in 0..20 {
            let tmp = tempfile::tempdir().unwrap();
            let p = paths(tmp.path(), false);
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let (p, barrier) = (p.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        let a = load(&p, true).expect("concurrent first load");
                        (a.agent.clone(), a.observer.clone().unwrap())
                    })
                })
                .collect();
            let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            assert!(results.iter().all(|r| *r == results[0]));
            for path in [&p.agent, &p.observer] {
                assert_eq!(fs::read_to_string(path).unwrap().len(), 65);
            }
            let leftovers = fs::read_dir(p.agent.parent().unwrap()).unwrap().count();
            assert_eq!(leftovers, 2, "temporary files must be cleaned up");
        }
    }

    #[test]
    fn publish_never_replaces_an_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = paths(tmp.path(), false);
        ensure_parent(&p.agent).unwrap();
        fs::write(&p.agent, "winner").unwrap();
        let e = create_secret(&p.agent).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&p.agent).unwrap(), "winner");
        assert_eq!(fs::read_dir(p.agent.parent().unwrap()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    mod windows_native {
        use super::*;

        fn icacls(path: &Path, args: &[&str]) -> bool {
            std::process::Command::new("icacls")
                .arg(path)
                .args(args)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        fn fresh(dir: &Path) -> PathBuf {
            let p = dir.join("native-token");
            let mut f = create_owner_only(&p).unwrap();
            f.write_all(format!("{GOOD}\n").as_bytes()).unwrap();
            p
        }

        #[test]
        fn restrictive_creation_is_accepted() {
            let tmp = tempfile::tempdir().unwrap();
            let p = fresh(tmp.path());
            let f = open_read(&p).unwrap();
            check_open_file(&f).unwrap();
            assert_eq!(inspect(&p).access, Access::OwnerOnly);
        }

        #[test]
        fn a_broad_grant_is_rejected() {
            let tmp = tempfile::tempdir().unwrap();
            let p = fresh(tmp.path());
            // *S-1-1-0 is Everyone.
            assert!(
                icacls(&p, &["/grant", "*S-1-1-0:R"]),
                "icacls /grant failed"
            );
            let f = open_read(&p).unwrap();
            assert!(check_open_file(&f).unwrap_err().contains("other users"));
            assert_eq!(inspect(&p).access, Access::TooOpen);
            assert!(load(
                &TokenPaths {
                    agent: p.clone(),
                    observer: tmp.path().join("obs"),
                    explicit: true
                },
                false
            )
            .is_err());
        }

        #[test]
        fn a_file_owned_by_someone_else_is_rejected() {
            let tmp = tempfile::tempdir().unwrap();
            let p = fresh(tmp.path());
            // Changing the owner needs a privilege or group the runner may
            // lack; skip only this case then. The owner check itself is
            // always compiled in.
            if !icacls(&p, &["/setowner", "*S-1-5-32-544"]) {
                eprintln!("skipped: cannot change the file owner here");
                return;
            }
            let f = open_read(&p).unwrap();
            assert_eq!(
                check_open_file(&f).unwrap_err(),
                "not owned by the current user"
            );
        }
    }
}
