//! End-to-end `classify` simulator: the real REPL worker over 2,000 synthetic records against a scripted fake
//! provider on a virtual clock (`classify_sim.py`; a model of the harness, not a measurement). It asserts the
//! labels, the call/token/time bounds, the log rows and the error paths, then prints the LEGACY vs NEW table
//! (`cargo test -p factr-learn --test classify_sim -- --nocapture` shows it).

#[test]
fn classify_end_to_end_simulation() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/classify_sim.py");
    let python = std::env::var("FACTR_SIM_PYTHON").unwrap_or_else(|_| "python3".into());
    let Ok(out) = std::process::Command::new(&python).arg("-I").arg(&script).output() else {
        eprintln!("skipped: no {python}");
        return;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    println!("{stdout}");
    assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("NOT A MEASUREMENT") && stdout.contains("LEGACY 0.0.3"), "{stdout}");
}
