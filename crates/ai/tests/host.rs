//! The agent's command host against a real headless engine session.

use filmcraft_ai::agent::CommandHost;
use filmcraft_engine::Session;
use serde_json::{Value, json};

#[test]
fn host_searches_describes_and_executes_real_commands() {
    let mut s = Session::default();
    let hits = s.search_commands("inspect", 10);
    let list = hits["commands"].as_array().expect("commands array");
    assert!(list.iter().any(|c| c["id"] == "sequence.inspect"), "{hits}");
    assert_eq!(s.is_mutating("sequence.inspect"), Some(false));
    assert_eq!(s.is_mutating("file.openDemoProject"), Some(true));
    assert_eq!(s.is_mutating("nope.nope"), None);
    let d = s.describe_command("sequence.inspect").expect("describe");
    assert_eq!(d["id"], "sequence.inspect");

    <Session as CommandHost>::execute(&mut s, "file.openDemoProject", &json!({})).expect("demo project");
    let v: Value = <Session as CommandHost>::execute(&mut s, "sequence.inspect", &json!({})).expect("inspect after demo");
    assert!(!v.is_null());
}
