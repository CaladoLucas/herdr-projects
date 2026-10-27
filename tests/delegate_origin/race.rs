//! Spawn evidence binds the expected child even if a pane is replaced inside the metadata write; claims are not repaired.
use super::*;
use std::{cell::Cell, rc::Rc};
use sw::herdr::cli::{Fake, HerdrError, Ran};

struct RacingHerdr {
    inner: Rc<common::herdr::FakeHerdr>,
    child: String,
    before_write: Value,
    after_write: Value,
    during_read: Value,
    writes: Cell<usize>,
}
impl RacingHerdr {
    fn patch(&self, patch: &Value) {
        let mut s = self.inner.st.borrow_mut();
        let p = s["panes"].as_array_mut().unwrap().iter_mut().find(|p| p.s("pane_id") == self.child).unwrap();
        for (k, v) in patch.as_object().into_iter().flatten() { p[k] = v.clone(); }
    }
}
impl Fake for RacingHerdr {
    fn run(&self, a: &[String], input: &str) -> Ran { self.inner.run(a, input) }
    fn peer_pid(&self) -> Option<i64> { self.inner.peer_pid() }
    fn ask(&self, method: &str, params: &Value) -> Result<Value, HerdrError> {
        if method == "pane.get" && params.s("pane_id") == self.child { self.patch(&self.during_read); }
        if method == "pane.report_metadata" {
            self.writes.set(self.writes.get() + 1);
            self.patch(&self.before_write);
        }
        let result = self.inner.ask(method, params);
        if method == "pane.report_metadata" { self.patch(&self.after_write); }
        result
    }
}
fn scene(w: &World) -> (Value, Value) {
    caller(w);
    let origin = sw::herdr::origin::capture(&w.socket).unwrap();
    let child = sw::herdr::Herdr::new(Some(w.socket.clone())).workspace_create(&w.root_str(), "child A", &[]).unwrap().at("root_pane").clone();
    (origin, child)
}
fn race(w: &World, child: &Value, before_write: Value, after_write: Value, during_read: Value) -> Rc<RacingHerdr> {
    let f = Rc::new(RacingHerdr { inner: w.fake.clone(), child: child.t("pane_id"), before_write, after_write, during_read, writes: Cell::new(0) });
    sw::herdr::cli::set_fake(Some(f.clone()));
    f
}
fn stamp(w: &World, origin: &Value, child: &Value) -> Option<()> {
    sw::herdr::origin::stamp(origin, &w.socket, child.s("pane_id"), child.s("terminal_id"))
}
fn tokens(w: &World, child: &Value) -> Value {
    w.state().arr("panes").iter().find(|p| p.s("pane_id") == child.s("pane_id")).unwrap().at("tokens").clone()
}
#[test]
fn replacement_exactly_at_write_receives_only_evidence_bound_to_original_child() {
    let w = common::world();
    let (origin, child) = scene(&w);
    let f = race(&w, &child, json!({"terminal_id": "replacement-B"}), json!({}), json!({}));
    assert!(stamp(&w, &origin, &child).is_none());
    assert_eq!(f.writes.get(), 1);
    let t = tokens(&w, &child);
    assert_eq!(t.as_object().unwrap().len(), 1, "no legacy fallback may describe B");
    let evidence: Value = serde_json::from_str(t.s("spawn_origin")).expect("a versioned JSON string");
    assert_eq!(evidence, json!([1, child.at("terminal_id"), origin.at("terminal")]));
    assert_ne!(evidence[1].as_str().unwrap(), "replacement-B", "consumer must reject this observation for B");
}
#[test]
fn all_claim_keys_observed_at_the_child_read_are_preserved_even_when_malformed() {
    let w = common::world();
    let (origin, child) = scene(&w);
    for key in ["spawn_origin", "spawned_by", "spawned_by_terminal"] {
        for value in [json!("foreign claim"), json!(""), Value::Null] {
            let claims = json!({key: value, "other": "untouched"});
            let f = race(&w, &child, json!({}), json!({}), json!({"tokens": claims}));
            assert!(stamp(&w, &origin, &child).is_none());
            assert_eq!(f.writes.get(), 0);
            assert_eq!(tokens(&w, &child), claims);
        }
    }
}
#[test]
fn a_concurrent_legacy_claim_is_never_overwritten_by_the_versioned_write() {
    let w = common::world();
    let (origin, child) = scene(&w);
    let f = race(&w, &child, json!({"tokens": {"spawned_by": "foreign", "spawned_by_terminal": "foreign-terminal", "other": "kept"}}), json!({}), json!({}));
    let _ = stamp(&w, &origin, &child);
    assert_eq!(f.writes.get(), 1);
    let t = tokens(&w, &child);
    assert_eq!(t.s("spawned_by"), "foreign");
    assert_eq!(t.s("spawned_by_terminal"), "foreign-terminal");
    assert_eq!(t.s("other"), "kept");
    assert!(t.has("spawn_origin"));
}
#[test]
fn a_concurrent_versioned_claim_seen_after_write_is_left_intact_and_reports_conflict() {
    let w = common::world();
    let (origin, child) = scene(&w);
    let rival = json!({"spawn_origin": "concurrent writer", "other": "kept"});
    let f = race(&w, &child, json!({}), json!({"tokens": rival}), json!({}));
    assert!(stamp(&w, &origin, &child).is_none());
    assert_eq!(f.writes.get(), 1, "no cleanup, retry or claim restoration");
    assert_eq!(tokens(&w, &child), rival);
}

#[test]
fn serialized_tuple_limit_counts_utf8_bytes_and_never_truncates_or_writes_over_limit() {
    let w = common::world();
    let (origin, mut child) = scene(&w);
    let overhead = json!([1, "", origin.at("terminal")]).to_string().len();
    for terminal in ["x".repeat(80 - overhead), "x".repeat(81 - overhead), "é".repeat((80 - overhead) / 2 + 1), "\"".repeat(80 - overhead)] {
        child["terminal_id"] = json!(terminal);
        let f = race(&w, &child, json!({}), json!({}), json!({}));
        f.patch(&json!({"terminal_id": terminal, "tokens": {}}));
        let encoded = json!([1, terminal, origin.at("terminal")]).to_string();
        if encoded.len() <= 80 {
            assert_eq!(stamp(&w, &origin, &child), Some(()));
            assert_eq!(f.writes.get(), 1);
            assert_eq!(tokens(&w, &child).s("spawn_origin"), encoded);
        } else {
            assert!(stamp(&w, &origin, &child).is_none());
            assert_eq!(f.writes.get(), 0, "oversized serialized UTF-8 must not reach Herdr");
            assert_eq!(tokens(&w, &child), json!({}));
        }
    }
}
