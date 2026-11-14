use super::ProcessGuardError;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::mem::{MaybeUninit, size_of};
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROC_UID_ONLY: u32 = 4;
const PROC_PPID_ONLY: u32 = 6;
const PROC_FLAG_INEXIT: u32 = 4;
const SZOMB: u32 = 5;
const MAX_RECORDS: usize = 131_072;
const MAX_ANCESTRY_DEPTH: usize = 16;
const MAX_ARG_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Executable {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl Executable {
    fn read(path: &Path) -> io::Result<Self> {
        let path = fs::canonicalize(path)?;
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "executable is not a file",
            ));
        }
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    fn bundle(&self) -> Option<&Path> {
        let macos = self.path.parent()?;
        let contents = macos.parent()?;
        let bundle = contents.parent()?;
        (macos.file_name()? == "MacOS"
            && contents.file_name()? == "Contents"
            && bundle.extension()? == "app")
            .then_some(bundle)
    }

    fn contains_helper(&self, candidate: &Self) -> bool {
        if self == candidate {
            return true;
        }
        let Some(bundle) = self.bundle() else {
            return false;
        };
        let Some(helper_bundle) = candidate.bundle() else {
            return false;
        };
        helper_bundle
            .parent()
            .is_some_and(|parent| parent == bundle.join("Contents/Frameworks"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProcessIdentity {
    pub(super) pid: u32,
    pub(super) created_at: SystemTime,
    parent: u32,
    uid: libc::uid_t,
    executable: Executable,
}

fn invalid_data() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "incomplete macOS process information",
    )
}

fn io_failure(result: i32) -> io::Error {
    if result <= 0 {
        io::Error::last_os_error()
    } else {
        invalid_data()
    }
}

fn absent(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ESRCH)
}

fn ownership_error(operation: &'static str, error: io::Error) -> ProcessGuardError {
    ProcessGuardError::OwnershipUnknown.context(operation, error)
}

fn process_error(operation: &'static str, error: io::Error) -> ProcessGuardError {
    ProcessGuardError::ProcessInspectionUnknown.context(operation, error)
}

fn own_uid() -> libc::uid_t {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

fn bsd_info(pid: u32) -> io::Result<libc::proc_bsdinfo> {
    let raw_pid = i32::try_from(pid).map_err(|_| invalid_data())?;
    if raw_pid <= 0 {
        return Err(invalid_data());
    }

    let mut data = MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let capacity = size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: data points to an aligned output buffer of the exact flavor size.
    let read = unsafe {
        libc::proc_pidinfo(
            raw_pid,
            libc::PROC_PIDTBSDINFO,
            0,
            data.as_mut_ptr().cast(),
            capacity,
        )
    };
    if read != capacity {
        return Err(io_failure(read));
    }
    // SAFETY: libproc returned a complete C process record.
    let data = unsafe { data.assume_init() };
    if data.pbi_pid != pid {
        return Err(invalid_data());
    }
    if data.pbi_status == SZOMB || data.pbi_flags & PROC_FLAG_INEXIT != 0 {
        return Err(io::Error::from_raw_os_error(libc::ESRCH));
    }
    Ok(data)
}

fn process_start(info: &libc::proc_bsdinfo) -> io::Result<SystemTime> {
    if info.pbi_start_tvsec == 0 || info.pbi_start_tvusec >= 1_000_000 {
        return Err(invalid_data());
    }
    UNIX_EPOCH
        .checked_add(Duration::from_secs(info.pbi_start_tvsec))
        .and_then(|start| start.checked_add(Duration::from_micros(info.pbi_start_tvusec)))
        .ok_or_else(invalid_data)
}

fn process_path(pid: u32) -> io::Result<PathBuf> {
    let pid = i32::try_from(pid).map_err(|_| invalid_data())?;
    let mut bytes = [0; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the fixed, writable buffer has the documented libproc path size.
    let size = unsafe { libc::proc_pidpath(pid, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    if size <= 0 || size as usize >= bytes.len() {
        return Err(io_failure(size));
    }
    let path = Path::new(OsStr::from_bytes(&bytes[..size as usize]));
    if !path.is_absolute() || path.as_os_str().as_bytes().contains(&0) {
        return Err(invalid_data());
    }
    Ok(path.to_owned())
}

impl ProcessIdentity {
    fn read(pid: u32) -> io::Result<Self> {
        let before = bsd_info(pid)?;
        let uid = before.pbi_uid;
        if uid != own_uid() || before.pbi_ruid != uid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "process UID differs",
            ));
        }

        let created_at = process_start(&before)?;
        let executable = Executable::read(&process_path(pid)?)?;
        let after = bsd_info(pid)?;
        if after.pbi_uid != uid
            || after.pbi_ruid != uid
            || after.pbi_ppid != before.pbi_ppid
            || process_start(&after)? != created_at
            || Executable::read(&process_path(pid)?)? != executable
        {
            return Err(invalid_data());
        }
        Ok(Self {
            pid,
            created_at,
            parent: before.pbi_ppid,
            uid,
            executable,
        })
    }

    fn confirm(&self) -> Result<(), ProcessGuardError> {
        let current = Self::read(self.pid)
            .map_err(|error| ownership_error("rechecking process identity", error))?;
        if current != *self {
            return Err(ProcessGuardError::OwnershipMismatch);
        }
        Ok(())
    }
}

pub(super) fn launched_process_state(pid: u32) -> String {
    match ProcessIdentity::read(pid) {
        Ok(_) => format!("PID {pid} is still running"),
        Err(error) if absent(&error) => format!("PID {pid} has exited"),
        Err(error) => format!("could not inspect PID {pid}: {}", error.kind()),
    }
}

pub(super) fn verified_process_identity(
    pid: u32,
    launched_after: SystemTime,
    executable: &Path,
) -> Result<ProcessIdentity, ProcessGuardError> {
    let expected = Executable::read(executable)
        .map_err(|error| ownership_error("resolving expected executable", error))?;
    let identity = ProcessIdentity::read(pid)
        .map_err(|error| ownership_error("reading process identity", error))?;
    require_start(&identity, launched_after)?;
    if identity.executable != expected {
        return Err(ProcessGuardError::ExecutableMismatch);
    }
    identity.confirm()?;
    Ok(identity)
}

fn require_start(
    identity: &ProcessIdentity,
    launched_after: SystemTime,
) -> Result<(), ProcessGuardError> {
    // Darwin records timeval microseconds, while SystemTime can carry nanoseconds.
    let elapsed = launched_after
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProcessGuardError::OwnershipUnknown)?;
    let earliest = UNIX_EPOCH
        .checked_add(Duration::new(
            elapsed.as_secs(),
            elapsed.subsec_micros() * 1_000,
        ))
        .ok_or(ProcessGuardError::OwnershipUnknown)?;
    if identity.created_at < earliest {
        return Err(ProcessGuardError::OwnershipMismatch);
    }
    Ok(())
}

fn list_records<T: Default>(
    minimum: usize,
    query: impl Fn(*mut libc::c_void, i32) -> i32,
) -> io::Result<Vec<T>> {
    let mut capacity = minimum
        .max(32)
        .checked_next_power_of_two()
        .ok_or_else(invalid_data)?;
    while capacity <= MAX_RECORDS {
        let mut records: Vec<T> = std::iter::repeat_with(T::default).take(capacity).collect();
        let bytes = records.len() * size_of::<T>();
        let read = query(
            records.as_mut_ptr().cast(),
            i32::try_from(bytes).map_err(|_| invalid_data())?,
        );
        let read = usize::try_from(read).map_err(|_| io::Error::last_os_error())?;
        if read > bytes || read % size_of::<T>() != 0 {
            return Err(invalid_data());
        }
        if read < bytes {
            records.truncate(read / size_of::<T>());
            return Ok(records);
        }
        capacity *= 2;
    }
    Err(invalid_data())
}

fn user_pids() -> io::Result<Vec<u32>> {
    // SAFETY: NULL/zero asks libproc for the current size. The next call
    // receives a writable buffer of positive pid_t slots.
    let bytes = unsafe { libc::proc_listpids(PROC_UID_ONLY, own_uid(), ptr::null_mut(), 0) };
    if bytes <= 0 {
        return Err(io_failure(bytes));
    }
    let pids = list_records::<i32>(
        bytes as usize / size_of::<i32>() + 32,
        |buffer, capacity| unsafe {
            libc::proc_listpids(PROC_UID_ONLY, own_uid(), buffer, capacity)
        },
    )?;
    if pids.is_empty() {
        return Err(invalid_data());
    }
    Ok(pids
        .into_iter()
        .filter(|&pid| pid > 0)
        .map(|pid| pid as u32)
        .collect())
}

fn matching_processes(executable: &Executable) -> io::Result<Vec<ProcessIdentity>> {
    matching_processes_except(executable, None)
}

fn matching_processes_except(
    executable: &Executable,
    excluded_pid: Option<u32>,
) -> io::Result<Vec<ProcessIdentity>> {
    let mut processes = Vec::new();
    for pid in user_pids()? {
        if excluded_pid == Some(pid) {
            continue;
        }
        let path = match process_path(pid) {
            Ok(path) => path,
            Err(error) if absent(&error) => continue,
            Err(error) => return Err(error),
        };
        if path != executable.path {
            // APFS may return the directory spelling of a case-insensitive
            // lookup. Resolve names before deciding when paths differ.
            match fs::canonicalize(&path) {
                Ok(path) if path == executable.path => {}
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            }
        }
        match ProcessIdentity::read(pid) {
            Ok(identity) if identity.executable == *executable => processes.push(identity),
            Ok(_) => continue,
            Err(error) if absent(&error) => continue,
            Err(error) => return Err(error),
        }
    }
    let parents: std::collections::HashMap<_, _> = processes
        .iter()
        .map(|process| (process.pid, process.created_at))
        .collect();
    processes.retain(|process| {
        !parents
            .get(&process.parent)
            .is_some_and(|created_at| *created_at <= process.created_at)
    });
    Ok(processes)
}

pub(super) fn unique_running_identity(
    path: &Path,
) -> Result<Option<ProcessIdentity>, ProcessGuardError> {
    let executable = Executable::read(path)
        .map_err(|error| process_error("resolving expected executable", error))?;
    let mut candidates = matching_processes(&executable)
        .map_err(|error| process_error("finding matching application process", error))?;
    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop()),
        _ => Err(ProcessGuardError::ProcessInspectionUnknown.context(
            "finding matching application process",
            "more than one matching root process",
        )),
    }
}

pub(super) fn is_executable_running(path: &Path) -> Result<bool, ProcessGuardError> {
    let expected = Executable::read(path)
        .map_err(|error| process_error("resolving expected executable", error))?;
    Ok(!matching_processes(&expected)
        .map_err(|error| process_error("checking for an existing macOS process", error))?
        .is_empty())
}

pub(super) fn is_executable_running_except(
    path: &Path,
    excluded_pid: u32,
) -> Result<bool, ProcessGuardError> {
    let expected = Executable::read(path)
        .map_err(|error| process_error("resolving expected executable", error))?;
    Ok(!matching_processes_except(&expected, Some(excluded_pid))
        .map_err(|error| process_error("checking for another macOS process", error))?
        .is_empty())
}

// libproc's socket_fdinfo is a fixed-size Darwin C ABI record declared in
// <sys/proc_info.h>. libc does not expose it. Declare the read fields and
// reserve the remainder of the C struct and its protocol union.
#[repr(C)]
struct SocketFdInfo {
    open_flags: u32,
    file_status: u32,
    file_tail: [u64; 2],
    stat: [u64; 17],
    socket_id: u64,
    pcb: u64,
    socket_type: i32,
    protocol: i32,
    family: i32,
    options: u16,
    socket_fields: [u8; 66],
    kind: i32,
    reserved: u32,
    foreign_port: i32,
    local_port: i32,
    generation: u64,
    ip_fields: [u8; 32],
    local_address: [u8; 16],
    address_tail: [u8; 16],
    tcp_state: i32,
    union_tail: [u8; 444],
}

const _: () = assert!(size_of::<SocketFdInfo>() == 792);
#[derive(Default)]
#[repr(C)]
struct ProcessFd {
    fd: i32,
    kind: u32,
}

const PROC_PIDFDSOCKETINFO: i32 = 3;
const SOCKINFO_TCP: i32 = 2;
const TSI_S_LISTEN: i32 = 1;
const PROC_FP_SHARED: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LocalListener {
    fd: i32,
    socket_id: u64,
    generation: u64,
}

fn listener_at_fd(pid: u32, fd: i32, port: u16) -> io::Result<Option<LocalListener>> {
    let mut info = MaybeUninit::<SocketFdInfo>::uninit();
    let bytes = size_of::<SocketFdInfo>() as i32;
    // SAFETY: the buffer is aligned and sized to the Darwin socket_fdinfo ABI.
    let read = unsafe {
        libc::proc_pidfdinfo(
            pid as i32,
            fd,
            PROC_PIDFDSOCKETINFO,
            info.as_mut_ptr().cast(),
            bytes,
        )
    };
    if read != bytes {
        return Err(io_failure(read));
    }
    // SAFETY: the reported size equals the complete ABI struct and all fields accept any bits.
    let info = unsafe { info.assume_init() };
    if info.kind != SOCKINFO_TCP
        || info.protocol != libc::IPPROTO_TCP
        || info.tcp_state != TSI_S_LISTEN
        || u16::try_from(info.local_port).ok().map(u16::from_be) != Some(port)
    {
        return Ok(None);
    }
    let ipv4_loopback = match info.family {
        libc::AF_INET => info.local_address[12..] == Ipv4Addr::LOCALHOST.octets(),
        // Darwin reports v4-mapped local addresses in the IPv4 field.
        libc::AF_INET6 if info.ip_fields[8] == 1 => {
            info.local_address[12..] == Ipv4Addr::LOCALHOST.octets()
        }
        _ => false,
    };
    if !ipv4_loopback {
        return Ok(None);
    }
    if info.options & libc::SO_REUSEPORT as u16 != 0
        || info.file_status & PROC_FP_SHARED != 0
        || info.socket_id == 0
        || info.generation == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "listener socket is shared or ambiguous",
        ));
    }
    Ok(Some(LocalListener {
        fd,
        socket_id: info.socket_id,
        generation: info.generation,
    }))
}

fn listener_in_process(
    process: &ProcessIdentity,
    port: u16,
) -> Result<Option<LocalListener>, ProcessGuardError> {
    let before = bsd_info(process.pid)
        .map_err(|error| ownership_error("reading listener process", error))?;
    let sockets =
        list_records::<ProcessFd>(before.pbi_nfiles as usize + 32, |buffer, capacity| unsafe {
            libc::proc_pidinfo(
                process.pid as i32,
                libc::PROC_PIDLISTFDS,
                0,
                buffer,
                capacity,
            )
        })
        .map_err(|error| ownership_error("listing process file descriptors", error))?;
    let mut found = None;
    for socket in sockets {
        if socket.kind != libc::PROX_FDTYPE_SOCKET as u32 {
            continue;
        }
        match listener_at_fd(process.pid, socket.fd, port) {
            Ok(Some(listener)) if found.is_none() => found = Some(listener),
            Ok(Some(_)) => return Err(ProcessGuardError::OwnershipUnknown),
            Ok(None) => {}
            // An unrelated descriptor can close between the two snapshots.
            Err(error) if error.raw_os_error() == Some(libc::EBADF) => {}
            Err(error) => return Err(ownership_error("reading process socket", error)),
        }
    }
    process.confirm()?;
    Ok(found)
}

fn verified_listener_for_root(root: &ProcessIdentity, port: u16) -> Result<(), ProcessGuardError> {
    if port == 0 {
        return Err(ProcessGuardError::OwnershipUnknown);
    }
    let mut candidates = vec![(root.clone(), Vec::new())];
    let mut found = None;
    while let Some((process, mut ancestors)) = candidates.pop() {
        if let Some(listener) = listener_in_process(&process, port)? {
            if found.is_some() {
                return Err(ProcessGuardError::OwnershipUnknown);
            }
            found = Some((process.clone(), ancestors.clone(), listener));
        }
        if ancestors.len() >= MAX_ANCESTRY_DEPTH {
            continue;
        }
        let children = list_records::<i32>(32, |buffer, capacity| unsafe {
            libc::proc_listpids(PROC_PPID_ONLY, process.pid, buffer, capacity)
        })
        .map_err(|error| ownership_error("listing child processes", error))?;
        ancestors.push(process.clone());
        for pid in children.into_iter().filter(|pid| *pid > 0) {
            let child = match ProcessIdentity::read(pid as u32) {
                Ok(child) => child,
                Err(error) if absent(&error) => continue,
                Err(error) => return Err(ownership_error("reading child process identity", error)),
            };
            if child.parent == process.pid
                && child.uid == root.uid
                && child.created_at >= process.created_at
                && !ancestors.iter().any(|ancestor| ancestor.pid == child.pid)
                && root.executable.contains_helper(&child.executable)
            {
                candidates.push((child, ancestors.clone()));
            }
        }
    }

    let Some((owner, ancestors, listener)) = found else {
        return Err(ProcessGuardError::OwnershipUnknown);
    };
    for ancestor in ancestors {
        ancestor.confirm()?;
    }
    owner.confirm()?;
    let refreshed = listener_at_fd(owner.pid, listener.fd, port)
        .map_err(|error| ownership_error("rechecking listener socket", error))?;
    if refreshed != Some(listener) {
        return Err(ProcessGuardError::OwnershipMismatch);
    }
    owner.confirm()?;
    root.confirm()
}

pub(super) fn verify_listener_owner(
    port: u16,
    pid: u32,
    launched_after: SystemTime,
) -> Result<(), ProcessGuardError> {
    let root = ProcessIdentity::read(pid)
        .map_err(|error| ownership_error("reading launched process identity", error))?;
    require_start(&root, launched_after)?;
    verified_listener_for_root(&root, port)
}

pub(super) fn verified_listener_process(
    port: u16,
    path: &Path,
) -> Result<ProcessIdentity, ProcessGuardError> {
    let expected = Executable::read(path)
        .map_err(|error| ownership_error("resolving expected executable", error))?;
    let roots = matching_processes(&expected)
        .map_err(|error| ownership_error("finding matching processes", error))?;
    let mut verified = None;
    for root in roots {
        match verified_listener_for_root(&root, port) {
            Ok(()) if verified.is_none() => verified = Some(root),
            Ok(()) => return Err(ProcessGuardError::OwnershipUnknown),
            Err(ProcessGuardError::OwnershipUnknown) => {}
            Err(error) => return Err(error),
        }
    }
    verified.ok_or(ProcessGuardError::OwnershipUnknown)
}

fn configured_debug_port(process: &ProcessIdentity) -> Result<Option<u16>, ProcessGuardError> {
    process.confirm()?;
    let mut size: libc::c_int = 0;
    let mut size_bytes = size_of::<libc::c_int>();
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: output is a writable int, with the matching length.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut size as *mut i32).cast(),
            &mut size_bytes,
            ptr::null_mut(),
            0,
        )
    };
    if result != 0
        || size_bytes != size_of::<libc::c_int>()
        || !(size_of::<i32>()..=MAX_ARG_BYTES).contains(&(size as usize))
    {
        return Err(ProcessGuardError::OwnershipUnknown);
    }
    let mut bytes = vec![0; size as usize];
    let mut length = bytes.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, process.pid as i32];
    // SAFETY: output has the validated capacity and is local to this exact-image
    // query. No raw arguments or environment entries are logged or copied out.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            bytes.as_mut_ptr().cast(),
            &mut length,
            ptr::null_mut(),
            0,
        )
    };
    if result != 0 || length > bytes.len() {
        return Err(ownership_error(
            "reading verified process debug switches",
            io::Error::last_os_error(),
        ));
    }
    process.confirm()?;
    parse_debug_switches(&bytes[..length]).ok_or(ProcessGuardError::OwnershipUnknown)
}

fn parse_debug_switches(bytes: &[u8]) -> Option<Option<u16>> {
    let argc = i32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?);
    let args = bytes.get(4..)?;
    let executable_end = args.iter().position(|&byte| byte == 0)?;
    let rest = args.get(executable_end + 1..)?;
    let argv_start = rest.iter().position(|&byte| byte != 0)?;
    let mut argv = rest[argv_start..].split(|&byte| byte == 0);
    let count = usize::try_from(argc).ok()?.checked_sub(1)?;
    argv.next()?; // argv[0]
    let mut port = None;
    let mut address = None;
    for _ in 0..count {
        let arg = argv.next()?;
        if arg == b"--" {
            break;
        }
        if arg == b"--type" || arg.starts_with(b"--type=") {
            return Some(None);
        }
        if let Some(value) = arg.strip_prefix(b"--remote-debugging-port=") {
            let value: u16 = std::str::from_utf8(value).ok()?.parse().ok()?;
            if value == 0 || port.replace(value).is_some() {
                return None;
            }
        }
        if let Some(value) = arg.strip_prefix(b"--remote-debugging-address=") {
            if address.replace(value).is_some() {
                return None;
            }
        }
    }
    Some(port.filter(|_| address == Some(b"127.0.0.1")))
}

pub(super) fn discover_listener_port(path: &Path) -> Result<Option<u16>, ProcessGuardError> {
    let expected = Executable::read(path)
        .map_err(|error| ownership_error("resolving expected executable", error))?;
    let roots = matching_processes(&expected)
        .map_err(|error| ownership_error("finding matching processes", error))?;
    let mut discovered = None;
    for root in roots {
        let Some(port) = configured_debug_port(&root)? else {
            continue;
        };
        match verified_listener_for_root(&root, port) {
            Ok(()) if discovered.is_none() => discovered = Some(port),
            Ok(()) => return Err(ProcessGuardError::OwnershipUnknown),
            Err(ProcessGuardError::OwnershipUnknown) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(discovered)
}

pub(super) struct ProcessObserver {
    identity: ProcessIdentity,
    queue: OwnedFd,
    exit: std::sync::Mutex<Option<u32>>,
}

impl ProcessObserver {
    pub(super) fn new(identity: ProcessIdentity) -> Result<Self, ProcessGuardError> {
        identity.confirm()?;
        // SAFETY: kqueue creates a new owned descriptor, or returns -1.
        let queue = unsafe { libc::kqueue() };
        if queue < 0 {
            return Err(process_error(
                "opening process exit observer",
                io::Error::last_os_error(),
            ));
        }
        // SAFETY: a nonnegative kqueue return is ours to close.
        let queue = unsafe { OwnedFd::from_raw_fd(queue) };
        // SAFETY: queue is an owned descriptor and F_SETFD takes one int argument.
        if unsafe { libc::fcntl(queue.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(process_error(
                "configuring process exit observer",
                io::Error::last_os_error(),
            ));
        }
        let event = libc::kevent {
            ident: identity.pid as usize,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT | libc::NOTE_EXITSTATUS,
            data: 0,
            udata: ptr::null_mut(),
        };
        // SAFETY: changelist is one initialized event, and no event buffer is requested.
        if unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                &event,
                1,
                ptr::null_mut(),
                0,
                ptr::null(),
            )
        } < 0
        {
            return Err(process_error(
                "registering process exit observer",
                io::Error::last_os_error(),
            ));
        }
        // Register between identity checks so a reused PID cannot be observed.
        identity.confirm()?;
        Ok(Self {
            identity,
            queue,
            exit: std::sync::Mutex::new(None),
        })
    }

    fn exit_status(&self) -> Result<Option<u32>, ProcessGuardError> {
        let mut cached = self
            .exit
            .lock()
            .map_err(|_| ProcessGuardError::ProcessInspectionUnknown)?;
        if cached.is_some() {
            return Ok(*cached);
        }
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let mut observed = MaybeUninit::<libc::kevent>::uninit();
        // SAFETY: the output is one aligned kevent, with a valid zero timeout.
        let result = unsafe {
            libc::kevent(
                self.queue.as_raw_fd(),
                ptr::null(),
                0,
                observed.as_mut_ptr(),
                1,
                &timeout,
            )
        };
        match result {
            0 => Ok(None),
            1 => {
                // SAFETY: kevent wrote one complete event.
                let observed = unsafe { observed.assume_init() };
                if observed.ident != self.identity.pid as usize
                    || observed.filter != libc::EVFILT_PROC
                    || observed.flags & libc::EV_ERROR != 0
                    || observed.fflags & libc::NOTE_EXIT == 0
                    || observed.fflags & libc::NOTE_EXITSTATUS == 0
                {
                    return Err(ProcessGuardError::ProcessInspectionUnknown);
                }
                let status = observed.data as i32;
                let code = if libc::WIFEXITED(status) {
                    libc::WEXITSTATUS(status)
                } else if libc::WIFSIGNALED(status) {
                    128 + libc::WTERMSIG(status)
                } else {
                    return Err(ProcessGuardError::ProcessInspectionUnknown);
                };
                *cached = Some(code as u32);
                Ok(*cached)
            }
            _ => Err(process_error(
                "reading process exit observer",
                io::Error::last_os_error(),
            )),
        }
    }

    pub(super) fn verify_listener(&self, port: u16) -> Result<(), ProcessGuardError> {
        self.verify_running()?;
        verified_listener_for_root(&self.identity, port)?;
        self.verify_running()
    }

    pub(super) fn verify_running(&self) -> Result<(), ProcessGuardError> {
        if let Some(code) = self.exit_status()? {
            return Err(ProcessGuardError::Detailed(format!(
                "attached official process exited: pid={}; exitCode={code}",
                self.identity.pid
            )));
        }
        self.identity.confirm()
    }

    pub(super) async fn wait_for_exit(&self) -> Result<u32, ProcessGuardError> {
        loop {
            if let Some(code) = self.exit_status()? {
                return Ok(code);
            }
            if let Err(error) = self.identity.confirm() {
                return self.exit_status()?.ok_or(error);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
