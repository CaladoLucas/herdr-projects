//! Verified delegation origin survives queues and retries without guessing a parent or replacing sidebar tokens.
use crate::common::{self, *};
use sw::json::*;

fn caller(w: &World) -> Value {
    let h = sw::herdr::Herdr::new(Some(w.socket.clone()));
    let p = h.workspace_create(&w.root_str(), "creator", &[]).unwrap().at("root_pane").clone();
    unsafe { std::env::set_var("HERDR_PANE_ID", p.s("pane_id")); }
    w.patch(|s| s["originCaller"] = p.at("pane_id").clone());
    p
}
fn spawn_writes(w: &World) -> Vec<Value> {
    w.state().arr("metadata").iter().filter(|m| m.to_string().contains("spawned_by")).cloned().collect()
}
#[test]
fn verified_origin_is_saved_before_background_and_stamped_on_child() {
    let w = common::world();
    w.script(json!({"worker": [{"status": "working"}, {"status": "working"}]}));
    let parent = caller(&w);
    let id = w.add("auxiliary");
    delegate(&id);
    let origin = task(&id).at("creationOrigin").clone();
    assert_eq!(origin.s("pane"), parent.s("pane_id"));
    assert_eq!(origin.s("terminal"), parent.s("terminal_id"));
    unsafe { std::env::remove_var("HERDR_PANE_ID"); }
    let t = settle(&id);
    let writes = spawn_writes(&w);
    assert!(!writes.is_empty());
    assert!(writes.iter().all(|m| m.s("pane") == t.at("link").s("pane")));
    assert!(writes.iter().any(|m| m.to_string().contains(parent.s("terminal_id"))));
}
#[test]
fn missing_origin_is_frozen_and_never_borrowed_from_the_child() {
    let w = common::world();
    w.script(json!({"worker": [{"status": "working"}, {"status": "working"}]}));
    let id = w.add("standalone");
    delegate(&id);
    assert!(task(&id).has("creationOrigin"));
    assert!(task(&id).at("creationOrigin") == &json!(false));
    caller(&w);
    settle(&id);
    assert!(spawn_writes(&w).is_empty());
}
#[test]
fn claimed_pane_without_caller_process_is_not_a_parent() {
    let w = common::world();
    w.script(json!({"worker": [{"status": "working"}, {"status": "working"}]}));
    caller(&w);
    w.patch(|s| s["originCaller"] = Value::Null);
    let id = w.add("unverified");
    delegate(&id);
    settle(&id);
    assert!(task(&id).at("creationOrigin") == &json!(false));
    assert!(spawn_writes(&w).is_empty());
}

#[test]
fn queue_uses_original_creator_even_when_another_pane_starts_it() {
    let w = common::world_with(Opts { repo: true, settings: json!({"board.agents": 1}), ..Default::default() });
    let blocker = w.add("occupy slot");
    delegate(&blocker);
    let parent = caller(&w);
    let id = w.add("queued auxiliary");
    delegate(&id);
    assert_eq!(task(&id).s("stage"), "queued");
    let origin = task(&id).at("creationOrigin").clone();
    caller(&w);
    // Simulate the normal slot claim without changing any user/global settings.
    sw::store::tasks::update(&id, |t| { sw::hook::ask_start(t); Ok(sw::store::tasks::Change::Write) }).unwrap();
    sw::delegate::delegate_job(&id);
    assert_eq!(task(&id).at("creationOrigin"), &origin);
    assert!(spawn_writes(&w).iter().any(|m| m.to_string().contains(parent.s("terminal_id"))));
}
#[test]
fn parent_reuse_before_queued_launch_is_a_visual_noop() {
    let w = common::world();
    w.script(json!({"worker": [{"status": "working"}, {"status": "working"}]}));
    let parent = caller(&w);
    let id = w.add("stale creator");
    delegate(&id);
    w.patch(|s| { for p in s["panes"].as_array_mut().unwrap() { if p.s("pane_id") == parent.s("pane_id") { p["terminal_id"] = json!("replacement"); } } });
    let t = settle(&id);
    assert!(t.b("startedAgentAt"));
    assert!(spawn_writes(&w).is_empty());
}
#[test]
fn runtime_replacement_with_identical_pane_ids_cannot_reparent() {
    let w = common::world();
    w.script(json!({"worker": [{"status": "working"}, {"status": "working"}]}));
    caller(&w);
    let id = w.add("new runtime");
    delegate(&id);
    std::fs::remove_file(&w.socket).unwrap();
    let _replacement = std::os::unix::net::UnixListener::bind(&w.socket).unwrap();
    let t = settle(&id);
    assert!(t.b("startedAgentAt"));
    assert!(spawn_writes(&w).is_empty());
}
#[test]
fn wrong_connection_and_reused_child_terminal_are_not_written() {
    let w = common::world();
    w.script(json!({"worker": [{"status": "working"}, {"status": "working"}]}));
    caller(&w);
    let id = w.add("bound destination");
    delegate(&id);
    let t = settle(&id);
    let origin = t.at("creationOrigin");
    let link = t.at("link");
    w.patch(|s| s["metadata"] = json!([]));
    let other = w.base.join("other.sock");
    let _other = std::os::unix::net::UnixListener::bind(&other).unwrap();
    assert!(sw::herdr::origin::stamp(origin, other.to_str().unwrap(), link.s("pane"), link.s("terminal")).is_none());
    assert!(sw::herdr::origin::stamp(origin, &w.socket, link.s("pane"), "old-child-terminal").is_none());
    assert!(spawn_writes(&w).is_empty());
}
#[test]
fn metadata_failure_never_fails_execution_and_retry_keeps_origin() {
    let w = common::world();
    w.script(json!({"worker": [{"status": "working"}, {"status": "working"}]}));
    let parent = caller(&w);
    let id = w.add("nonfatal stamp");
    delegate(&id);
    w.patch(|s| s["metadataFails"] = json!(true));
    let t = settle(&id);
    assert!(t.b("startedAgentAt"));
    assert!(spawn_writes(&w).is_empty());
    let origin = t.at("creationOrigin").clone();
    caller(&w);
    w.patch(|s| {
        s["metadataFails"] = json!(false);
        for p in s["panes"].as_array_mut().unwrap() { if p.s("pane_id") == t.at("link").s("pane") { p["agent"] = Value::Null; } }
    });
    sw::delegate::restart_job(&id);
    assert_eq!(task(&id).at("creationOrigin"), &origin);
    assert!(spawn_writes(&w).iter().any(|m| m.to_string().contains(parent.s("terminal_id"))));
}
#[test]
fn sidebar_and_other_sources_keep_their_tokens() {
    let w = common::world();
    w.script(json!({"worker": [{"status": "working"}, {"status": "working"}]}));
    let parent = caller(&w);
    let id = w.add("preserve metadata");
    delegate(&id);
    let t = settle(&id);
    w.patch(|s| { for p in s["panes"].as_array_mut().unwrap() { if p.s("pane_id") == t.at("link").s("pane") { p["tokens"]["other"] = json!("untouched"); p["tokens"]["progress"] = json!("42"); } } });
    sw::herdr::origin::stamp(t.at("creationOrigin"), &w.socket, t.at("link").s("pane"), t.at("link").s("terminal"));
    assert_eq!(w.pane_of(&t).at("tokens").s("progress"), "42");
    sw::flow::sidebar::tell_sidebar(&t);
    let p = w.pane_of(&t);
    assert_eq!(p.at("tokens").s("spawned_by"), parent.s("pane_id"));
    assert_eq!(p.at("tokens").s("spawned_by_terminal"), parent.s("terminal_id"));
    assert_eq!(p.at("tokens").s("other"), "untouched");
    assert!(spawn_writes(&w).iter().all(|m| m.to_string().contains("plugin:herdr-projects")));
}
#[test]
fn a_missing_or_unreadable_parent_does_not_block_the_worker() {
    let w = common::world();
    w.script(json!({"worker": [{"status": "working"}, {"status": "working"}]}));
    let parent = caller(&w);
    let id = w.add("parent gone");
    delegate(&id);
    w.patch(|s| s["panes"].as_array_mut().unwrap().retain(|p| p.s("pane_id") != parent.s("pane_id")));
    assert!(settle(&id).b("startedAgentAt"));
    assert!(spawn_writes(&w).is_empty());
}
#[test]
fn unavailable_capture_is_frozen_and_explicit_socket_is_required() {
    let w = common::world();
    w.script(json!({"worker": [{"status": "working"}, {"status": "working"}]}));
    caller(&w);
    w.patch(|s| s["processInfoFails"] = json!(true));
    let id = w.add("unreadable caller");
    delegate(&id);
    assert!(task(&id).at("creationOrigin") == &json!(false));
    w.patch(|s| s["processInfoFails"] = json!(false));
    unsafe { std::env::remove_var("HERDR_SOCKET_PATH"); }
    assert!(sw::herdr::origin::capture(&w.socket).is_none());
    unsafe { std::env::set_var("HERDR_SOCKET_PATH", &w.socket); }
    settle(&id);
    assert!(spawn_writes(&w).is_empty());
}

#[test]
fn foreign_spawn_claim_and_wrong_server_identity_are_preserved() {
    let w = common::world();
    w.script(json!({"worker": [{"status": "working"}]}));
    caller(&w);
    let id = w.add("foreign tokens");
    delegate(&id);
    let t = settle(&id);
    let l = t.at("link");
    w.patch(|s| {
        s["metadata"] = json!([]);
        for p in s["panes"].as_array_mut().unwrap() { if p.s("pane_id") == l.s("pane") { p["tokens"] = json!({"spawned_by": "foreign", "spawned_by_terminal": "foreign-terminal"}); } }
    });
    assert!(sw::herdr::origin::stamp(t.at("creationOrigin"), &w.socket, l.s("pane"), l.s("terminal")).is_none());
    let mut origin = t.at("creationOrigin").clone();
    origin["serverPid"] = json!(12345);
    assert!(sw::herdr::origin::stamp(&origin, &w.socket, l.s("pane"), l.s("terminal")).is_none());
    assert_eq!(w.pane_of(&t).at("tokens").s("spawned_by"), "foreign");
    assert!(spawn_writes(&w).is_empty());
}

/// A real Unix transport with synthetic replies: closes after each request exactly as Herdr does. No program,
/// model, real session or clock wait. Drop unblocks accept and joins the thread even after an assertion fails.
struct OneReplyServer { path: std::path::PathBuf, thread: Option<std::thread::JoinHandle<()>>, writes: std::sync::Arc<std::sync::Mutex<Vec<Value>>> }
impl OneReplyServer {
    fn new(path: std::path::PathBuf) -> Self {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let writes = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        let recorded = writes.clone();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut s = BufReader::new(stream.unwrap());
                let mut line = String::new();
                if s.read_line(&mut line).unwrap_or(0) == 0 { continue; }
                let r: Value = serde_json::from_str(&line).unwrap();
                let p = r.at("params");
                let result = match r.s("method") {
                    "stop" => break,
                    "pane.get" => json!({"pane": {"pane_id": p.at("pane_id"), "terminal_id": format!("terminal-{}", p.s("pane_id"))}}),
                    "pane.process_info" => json!({"process_info": {"pane_id": p.at("pane_id"), "shell_pid": std::process::id()}}),
                    "pane.report_metadata" => { recorded.lock().unwrap().push(p.clone()); json!({"type": "ok"}) },
                    _ => panic!("unexpected request"),
                };
                writeln!(s.get_mut(), "{}", json!({"id": r.at("id"), "result": result})).unwrap();
            }
        });
        Self { path, thread: Some(thread), writes }
    }
}
impl Drop for OneReplyServer {
    fn drop(&mut self) {
        use std::io::Write;
        if let Ok(mut s) = std::os::unix::net::UnixStream::connect(&self.path) { let _ = writeln!(s, "{}", json!({"method": "stop"})); }
        let _ = self.thread.take().unwrap().join();
    }
}
#[test]
fn unix_server_closing_after_each_response_supports_capture_and_stamp() {
    let w = common::world();
    let server = OneReplyServer::new(w.base.join("o"));
    sw::herdr::cli::set_fake(None);
    let socket = server.path.to_str().unwrap();
    unsafe { std::env::set_var("HERDR_PANE_ID", "parent"); std::env::set_var("HERDR_SOCKET_PATH", socket); }
    let origin = sw::herdr::origin::capture(socket).expect("capture across one-response connections");
    assert_eq!(origin.s("terminal"), "terminal-parent");
    assert!(sw::herdr::origin::stamp(&origin, socket, "child", "wrong-terminal").is_none());
    assert!(server.writes.lock().unwrap().is_empty());
    assert!(sw::herdr::origin::stamp(&origin, socket, "child", "terminal-child").is_some());
    let writes = server.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].at("tokens"), &json!({"spawned_by": "parent", "spawned_by_terminal": "terminal-parent"}));
    assert_eq!(writes[0].s("source"), "plugin:herdr-projects");
}

#[test]
fn absent_origin_survives_retry_and_attempt_creation() {
    let w = common::world();
    let id = w.add("no initial caller");
    delegate(&id);
    caller(&w);
    sw::store::tasks::update(&id, |t| { t.put("stage", "failed"); Ok(sw::store::tasks::Change::Write) }).unwrap();
    delegate(&id);
    let t = task(&id);
    assert_eq!(t.at("creationOrigin"), &json!(false));
    let a = sw::store::attempts::create_attempt(&t, "A", "codex", Obj::new()).unwrap();
    assert_eq!(a.at("creationOrigin"), &json!(false));
    let origin = sw::herdr::origin::capture(&w.socket).unwrap();
    let mut known = t.clone(); known.put("creationOrigin", origin.clone());
    let b = sw::store::attempts::create_attempt(&known, "B", "codex", Obj::new()).unwrap();
    assert_eq!(b.at("creationOrigin"), &origin);
}
#[test]
fn legacy_queued_task_does_not_adopt_retry_caller() {
    let w = common::world();
    let id = w.add("legacy task");
    sw::store::tasks::update(&id, |t| { t.put("stage", "failed"); t.put("queuedAt", 1); Ok(sw::store::tasks::Change::Write) }).unwrap();
    caller(&w);
    delegate(&id);
    assert_eq!(task(&id).at("creationOrigin"), &json!(false));
}
