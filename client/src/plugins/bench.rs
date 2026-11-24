use super::host::{self, Cmd, Fetch, Sink, Spec, UiOp};
use super::manifest::Manifest;
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

const STATE: &str = r#"{"track":{"id":"a","uri":"spotify:track:0s3nnoMeVWz3989MkNQiRf","name":"Song","album":"Album","duration":180000,"image":"","artists":[{"id":"b","name":"Artist"}]},"isPlaying":true,"position":1000,"duration":180000,"volume":0.5,"shuffle":false,"repeat":"off","context":null}"#;

fn harness() -> (mpsc::Sender<Cmd>, mpsc::Receiver<(Instant, UiOp)>) {
    let (out_tx, out_rx) = mpsc::channel();
    let sink: Sink = Arc::new(move |op| {
        let _ = out_tx.send((Instant::now(), op));
    });
    let fetch: Fetch = Arc::new(|_, done| done(Ok("{}".into())));
    (host::spawn(sink, fetch), out_rx)
}

fn load(tx: &mpsc::Sender<Cmd>, rx: &mpsc::Receiver<(Instant, UiOp)>, id: &str, perms: &[&str], script: &str) -> Duration {
    let manifest = Manifest {
        id: id.into(),
        name: id.into(),
        main: "index.js".into(),
        permissions: perms.iter().map(|p| p.to_string()).collect(),
        ..Manifest::default()
    };
    let storage_dir = std::env::temp_dir().join(format!("rustify-bench-{}", std::process::id()));
    let started = Instant::now();
    tx.send(Cmd::Load(Box::new(Spec { manifest, script: script.into(), storage_dir, state: None }))).unwrap();
    loop {
        match rx.recv_timeout(Duration::from_secs(5)).unwrap().1 {
            UiOp::Started(p) if p == id => return started.elapsed(),
            UiOp::Failed { plugin, error } if plugin == id => panic!("{error}"),
            _ => {}
        }
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn report(name: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let mean = samples.iter().sum::<Duration>() / samples.len() as u32;
    println!(
        "{name}: n={} mean={:?} p50={:?} p95={:?} p99={:?} max={:?}",
        samples.len(),
        mean,
        percentile(&samples, 0.5),
        percentile(&samples, 0.95),
        percentile(&samples, 0.99),
        samples.last().unwrap()
    );
}

fn drain(rx: &mpsc::Receiver<(Instant, UiOp)>) {
    while rx.try_recv().is_ok() {}
}

#[test]
#[ignore]
fn dispatch_latency() {
    let (tx, rx) = harness();
    let took = load(&tx, &rx, "echo", &[], "Rustify.Player.addEventListener('songchange', d => console.log(d.track.name))");
    println!("load: {took:?}");
    let mut samples = Vec::new();
    for _ in 0..400 {
        std::thread::sleep(Duration::from_millis(10));
        let sent = Instant::now();
        tx.send(Cmd::Emit("songchange".into(), STATE.into())).unwrap();
        loop {
            let (at, op) = rx.recv_timeout(Duration::from_secs(2)).unwrap();
            if matches!(&op, UiOp::Log { text, .. } if text == "Song") {
                samples.push(at - sent);
                break;
            }
        }
    }
    report("event -> plugin -> UI, 1 plugin", samples);
    tx.send(Cmd::Shutdown).unwrap();
}

#[test]
#[ignore]
fn dispatch_with_many_plugins() {
    let (tx, rx) = harness();
    for i in 0..20 {
        load(&tx, &rx, &format!("busy-{i:02}"), &[], "let n = 0; Rustify.Player.addEventListener('onprogress', d => { n += d.position; JSON.stringify(d); })");
    }
    load(&tx, &rx, "probe", &[], "Rustify.Player.addEventListener('songchange', () => console.log('hit'))");
    drain(&rx);
    let mut samples = Vec::new();
    for _ in 0..300 {
        std::thread::sleep(Duration::from_millis(10));
        for kind in ["state", "onprogress"] {
            tx.send(Cmd::Emit(kind.into(), STATE.into())).unwrap();
        }
        let sent = Instant::now();
        tx.send(Cmd::Emit("songchange".into(), STATE.into())).unwrap();
        loop {
            let (at, op) = rx.recv_timeout(Duration::from_secs(2)).unwrap();
            if matches!(&op, UiOp::Log { text, .. } if text == "hit") {
                samples.push(at - sent);
                break;
            }
        }
    }
    report("songchange behind 40 events, 21 plugins", samples);
    tx.send(Cmd::Shutdown).unwrap();
}

#[test]
#[ignore]
fn sustained_event_load() {
    let (tx, rx) = harness();
    let page = "const page = Rustify.UI.Page('p', 'P', 'bolt'); let ticks = 0;
        Rustify.Player.addEventListener('onprogress', d => {
            ticks++;
            if (ticks % 10 === 0) page.render(Array.from({ length: 50 }, (_, i) => ({ type: 'row', id: 'r' + i, text: d.track.name + i, sub: String(d.position) })));
        });";
    for i in 0..5 {
        load(&tx, &rx, &format!("page-{i}"), &[], page);
    }
    drain(&rx);
    let events = 10_000;
    let started = Instant::now();
    for _ in 0..events {
        tx.send(Cmd::Emit("onprogress".into(), STATE.into())).unwrap();
    }
    tx.send(Cmd::Emit("songchange".into(), STATE.into())).unwrap();
    tx.send(Cmd::Unload("page-0".into())).unwrap();
    let mut renders = 0;
    loop {
        match rx.recv_timeout(Duration::from_secs(30)).unwrap().1 {
            UiOp::Render { .. } => renders += 1,
            UiOp::Stopped(p) if p == "page-0" => break,
            _ => {}
        }
    }
    let elapsed = started.elapsed();
    println!(
        "{events} progress events x 5 plugins: {elapsed:?} total, {:?} per event, {:.0} events/s, {renders} page renders delivered",
        elapsed / events,
        events as f64 / elapsed.as_secs_f64()
    );
    tx.send(Cmd::Shutdown).unwrap();
}

#[test]
#[ignore]
fn load_and_unload_cost() {
    let (tx, rx) = harness();
    let script = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/plugins/now-playing-tools/index.js")).unwrap();
    let mut samples = Vec::new();
    for i in 0..50 {
        let id = format!("now-playing-{i:02}");
        samples.push(load(&tx, &rx, &id, &["player", "spotify"], &script));
        tx.send(Cmd::Unload(id)).unwrap();
    }
    report("load example plugin (new engine + prelude + script)", samples);
    tx.send(Cmd::Shutdown).unwrap();
}
