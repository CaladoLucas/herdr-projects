//! Verified local creation origins and best-effort spawn tokens, bound to a socket incarnation and immutable terminals.
use super::cli::{fake, kids};
use crate::json::*;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::{fs::{FileTypeExt, MetadataExt}, net::UnixStream};
use std::time::Duration;

/// The actual local endpoint, not a session name or a CLI configuration default. Replacement of a socket invalidates
/// the origin even if its pathname and pane IDs are reused. Terminal IDs additionally reject an in-place restart.
fn endpoint(socket: &str) -> Option<Value> {
    let path = std::fs::canonicalize(socket).ok()?;
    let m = std::fs::metadata(&path).ok()?;
    if !m.file_type().is_socket() { return None; }
    Some(json!({ "path": path, "device": m.dev(), "inode": m.ino(), "ctime": m.ctime(), "ctimeNs": m.ctime_nsec() }))
}
/// Herdr closes each connection after one response. Every request pins the same endpoint incarnation and kernel
/// peer identity; no CLI fallback, machine selection, or reconnect to a different server is allowed.
struct Connection { socket: String, runtime: Value, peer: i64 }
impl Connection {
    fn open(socket: &str) -> Option<Self> { Some(Self { socket: socket.into(), runtime: endpoint(socket)?, peer: 0 }) }
    fn call(&mut self, method: &str, params: Value) -> Option<Value> {
        if endpoint(&self.socket)? != self.runtime { return None; }
        let result = if let Some(f) = fake() {
            let peer = f.peer_pid()?;
            if self.peer != 0 && self.peer != peer { return None; }
            self.peer = peer;
            f.ask(method, &params).ok()?
        } else {
            let s = UnixStream::connect(&self.socket).ok()?;
            s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
            s.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
            let peer = peer_pid(&s)?;
            if self.peer != 0 && self.peer != peer { return None; }
            self.peer = peer;
            // Check again after connecting and before writing, including the metadata request itself.
            if endpoint(&self.socket)? != self.runtime { return None; }
            let mut s = BufReader::new(s);
            writeln!(s.get_mut(), "{}", json!({ "id": "projects-origin", "method": method, "params": params })).ok()?;
            let mut line = String::new();
            s.read_line(&mut line).ok()?;
            let r: Value = serde_json::from_str(&line).ok()?;
            if r.at("error").is_object() { return None; }
            r.at("result").clone()
        };
        (endpoint(&self.socket)? == self.runtime).then_some(result)
    }
    fn pane(&mut self, id: &str) -> Option<Value> {
        let r = self.call("pane.get", json!({ "pane_id": id }))?;
        let p = r.at("pane");
        (p.s("pane_id") == id && !p.s("terminal_id").is_empty()).then(|| p.clone())
    }
}
/// A claimed pane must actually contain this caller. This is verification of an explicit claim, never discovery of
/// a parent from process names, a workspace or a coordinator. Unknown process ancestry fails closed for presentation.
fn contains_process(root: i64, caller: i64) -> bool {
    if root <= 1 || caller <= 1 { return false; }
    let mut todo = vec![root];
    let mut seen = std::collections::HashSet::new();
    while let Some(pid) = todo.pop() {
        if pid == caller { return true; }
        if !seen.insert(pid) { continue; }
        if seen.len() > 512 { return false; }
        let Ok(children) = children_of(pid) else { return false };
        todo.extend(children);
    }
    false
}
/// BSD pgrep hides its own ancestors by default: those are precisely the chain being verified here.
fn children_of(pid: i64) -> Result<Vec<i64>, ()> {
    if fake().is_some() { return kids(pid); }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("pgrep").args(["-a", "-P", &pid.to_string()]).output().map_err(|_| ())?;
        if out.status.code().unwrap_or(2) > 1 { return Err(()); }
        Ok(String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse().ok()).collect())
    }
    #[cfg(not(target_os = "macos"))]
    { kids(pid) }
}
/// Capture once at the foreground delegation boundary, before a runner can inherit a different pane. No explicit
/// socket or pane, an inaccessible runtime, or an unverifiable caller means no visual relationship.
pub fn capture(socket: &str) -> Option<Value> {
    let pane = std::env::var("HERDR_PANE_ID").ok().filter(|p| !p.is_empty())?;
    if std::env::var("HERDR_SOCKET_PATH").ok().as_deref() != Some(socket) { return None; }
    let runtime = endpoint(socket)?;
    let mut c = Connection::open(socket)?;
    let p = c.pane(&pane)?;
    let info = c.call("pane.process_info", json!({ "pane_id": pane }))?;
    let shell = info.at("process_info").i("shell_pid");
    if !contains_process(c.peer, shell) || !contains_process(shell, std::process::id() as i64) { return None; }
    if endpoint(socket)? != runtime || c.pane(&pane)?.at("terminal_id") != p.at("terminal_id") { return None; }
    Some(json!({ "socket": socket, "runtime": runtime, "serverPid": c.peer, "pane": pane, "terminal": p.at("terminal_id") }))
}
/// Best effort, source-owned and additive. Validate both exact terminals before writing; never clear anybody's
/// tokens, and never make an execution failure out of unavailable presentation metadata.
pub fn stamp(origin: &Value, socket: &str, pane: &str, terminal: &str) -> Option<()> {
    if origin.s("socket") != socket || pane.is_empty() || terminal.is_empty() || origin.s("pane") == pane || origin.s("terminal").is_empty() { return None; }
    let runtime = endpoint(socket)?;
    if origin.at("runtime") != &runtime { return None; }
    let mut c = Connection::open(socket)?;
    c.peer = origin.i("serverPid");
    if c.peer <= 1 { return None; }
    let parent = c.pane(origin.s("pane"))?;
    let child = c.pane(pane)?;
    if parent.s("terminal_id") != origin.s("terminal") || child.s("terminal_id") != terminal || parent.s("terminal_id") == terminal { return None; }
    if endpoint(socket)? != runtime { return None; }
    // Herdr's tokens are pane-global even though writes have a source. Do not take over an existing claim.
    let tokens = child.at("tokens");
    if ["spawn_origin", "spawned_by", "spawned_by_terminal"].iter().any(|k| tokens.get(*k).is_some()) { return None; }
    // Bind the evidence itself to the expected child. A delayed pane-ID write may land on a replacement, but it
    // cannot describe that replacement. Never also emit legacy tokens: their consumers cannot check the child.
    let evidence = json!([1, terminal, origin.s("terminal")]).to_string();
    // Herdr 0.9.5 truncates token values at 80 characters. Bytes are conservative for UTF-8; never truncate evidence.
    if evidence.len() > 80 { return None; }
    c.call("pane.report_metadata", json!({ "pane_id": pane, "source": "plugin:herdr-projects", "tokens": {
        "spawn_origin": evidence
    } }))?;
    if c.pane(origin.s("pane"))?.s("terminal_id") != origin.s("terminal") { return None; }
    let after = c.pane(pane)?;
    let tokens = after.at("tokens");
    if after.s("terminal_id") != terminal || tokens.s("spawn_origin") != evidence ||
        ["spawned_by", "spawned_by_terminal"].iter().any(|k| tokens.get(*k).is_some()) { return None; }
    Some(())
}

/// Kernel identity of the process serving this connection. A forwarded socket is not proof of a local runtime:
/// capture also verifies that the reported shell descends from this peer, so remote PID coincidences do not count.
fn peer_pid(stream: &UnixStream) -> Option<i64> {
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "linux")]
    unsafe {
        let mut cred: libc::ucred = std::mem::zeroed();
        let mut len = std::mem::size_of_val(&cred) as libc::socklen_t;
        if libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut _, &mut len) != 0 { return None; }
        return (cred.pid > 1).then_some(cred.pid as i64);
    }
    #[cfg(target_os = "macos")]
    unsafe {
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of_val(&pid) as libc::socklen_t;
        if libc::getsockopt(stream.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID, &mut pid as *mut _ as *mut _, &mut len) != 0 { return None; }
        return (pid > 1).then_some(pid as i64);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    { let _ = stream; None }
}

#[cfg(test)]
mod tests {
    #[test]
    fn real_programs_process_query_includes_its_callers_ancestors() {
        let parent = unsafe { libc::getppid() } as i64;
        assert!(super::contains_process(parent, std::process::id() as i64));
    }
}
