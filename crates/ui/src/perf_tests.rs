//! Messungen des Bildbudgets (AGENTS.md: ein Bild deutlich unter 2 ms bei 1000 Spuren).
//!
//! Gemessen wird die CPU-Zeit eines Bildes so, wie die App sie hat: `Context::run_ui` mit dem
//! Projektfenster und danach die Tessellierung, ohne AccessKit und ohne GPU. Jedes Bild scrollt,
//! damit nichts aus dem letzten Bild übrig bleibt.
//!
//! `cargo test -p studiocraft-ui --release perf_ -- --ignored --nocapture --test-threads=1`

use super::*;
use studiocraft_project::{Note, TrackKind};

const SIZE: egui::Vec2 = egui::vec2(1920.0, 1080.0);
const FRAMES: usize = 300;

/// Ergebnis einer Messung in Millisekunden.
struct Timing {
    mean: f64,
    median: f64,
    p95: f64,
    max: f64,
}

/// Misst `FRAMES` Bilder; `step` darf vor jedem Bild den Zustand ändern und Eingaben liefern.
fn measure(
    mut w: ProjectWindow,
    mut step: impl FnMut(&mut ProjectWindow, usize) -> Vec<egui::Event>,
) -> Timing {
    let ctx = egui::Context::default();
    let input = |events: Vec<egui::Event>| egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)),
        events,
        ..Default::default()
    };
    // Einschwingen: Schriften, Texturen, Layout-Speicher.
    for i in 0..20 {
        let ev = step(&mut w, i);
        let mut out = ctx.run_ui(input(ev), |ui| w.ui(ui));
        out.textures_delta.clear();
        let _ = ctx.tessellate(out.shapes, out.pixels_per_point);
    }
    let mut times = Vec::with_capacity(FRAMES);
    for i in 0..FRAMES {
        let ev = step(&mut w, i + 20);
        let t = std::time::Instant::now();
        let mut out = ctx.run_ui(input(ev), |ui| w.ui(ui));
        out.textures_delta.clear();
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(prims);
    }
    times.sort_by(f64::total_cmp);
    Timing {
        mean: times.iter().sum::<f64>() / times.len() as f64,
        median: times[times.len() / 2],
        p95: times[times.len() * 95 / 100],
        max: times[times.len() - 1],
    }
}

fn report(name: &str, t: &Timing) {
    println!(
        "PERF {name:<40} Mittel {:>6.2} ms  Median {:>6.2}  p95 {:>6.2}  max {:>6.2}",
        t.mean, t.median, t.p95, t.max
    );
}

/// Mausrad über einem Punkt.
fn wheel(at: egui::Pos2, delta: egui::Vec2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(at),
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
        },
    ]
}

/// Untere Zone mit fester Höhe.
fn lower(w: &mut ProjectWindow, zone: LowerZone, height: f32) {
    w.show_lower = true;
    w.lower = zone;
    w.project.layout.lower_height = height;
    w.apply_layout(w.project.layout.clone());
}

/// Ein Instrument-Part mit `n` Noten (Achtel, wechselnde Tonhöhen).
fn big_part(w: &mut ProjectWindow, n: usize) -> EventId {
    let mut p = Project::default();
    let t = p.add_track(TrackKind::Instrument, "Synth");
    let len = (n as u64 * 480).max(3840);
    let part = p.create_event(t, 0, len).unwrap();
    p.event_mut(part).unwrap().notes = (0..n)
        .map(|i| Note {
            pitch: 48 + (i * 7 % 24) as u8,
            velocity: 64 + (i % 60) as u8,
            start: i as i64 * 480,
            length: 480,
            spelling: 0,
        })
        .collect();
    w.project = p;
    part
}

#[test]
#[ignore = "Messung, kein Funktionstest"]
fn perf_project_window() {
    for n in [100, 1000] {
        let mut w = ProjectWindow::new(Project::demo(n), None, String::new());
        w.show_lower = false;
        w.view.zoom(0.5, None);
        let t = measure(w, |w, _| {
            w.view.scroll_y = (w.view.scroll_y + 7.0) % 2000.0;
            w.view.scroll_ticks = (w.view.scroll_ticks + 50.0) % 100_000.0;
            vec![]
        });
        report(&format!("Projektfenster {n} Spuren"), &t);
    }
}

#[test]
#[ignore = "Messung, kein Funktionstest"]
fn perf_mixer() {
    for n in [100, 1000] {
        let mut w = ProjectWindow::new(Project::demo(n), None, String::new());
        lower(&mut w, LowerZone::Mixer, 520.0);
        let t = measure(w, |_, i| {
            let dx = if (i / 100) % 2 == 0 { -40.0 } else { 40.0 };
            wheel(egui::pos2(900.0, 800.0), egui::vec2(dx, 0.0))
        });
        report(&format!("Mixer {n} Kanäle"), &t);
    }
}

#[test]
#[ignore = "Messung, kein Funktionstest"]
fn perf_key_editor() {
    for n in [2_000, 20_000] {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let part = big_part(&mut w, n);
        w.open_key_editor(part);
        lower(&mut w, LowerZone::Editor, 600.0);
        let t = measure(w, |w, _| {
            w.key_editor.view.scroll_ticks =
                (w.key_editor.view.scroll_ticks + 240.0) % (n as f64 * 480.0);
            vec![]
        });
        report(&format!("Key-Editor {n} Noten"), &t);
    }
}

#[test]
#[ignore = "Messung, kein Funktionstest"]
fn perf_score_editor() {
    for n in [500, 4_000] {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let part = big_part(&mut w, n);
        w.open_key_editor(part);
        lower(&mut w, LowerZone::Score, 600.0);
        let t = measure(w, |_, i| {
            let dy = if (i / 100) % 2 == 0 { -60.0 } else { 60.0 };
            wheel(egui::pos2(900.0, 800.0), egui::vec2(0.0, dy))
        });
        report(&format!("Noten-Editor {n} Noten"), &t);
    }
}

#[test]
#[ignore = "Messung, kein Funktionstest"]
fn perf_list_editor() {
    for n in [2_000, 20_000] {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let part = big_part(&mut w, n);
        w.open_key_editor(part);
        w.selection = vec![part];
        lower(&mut w, LowerZone::List, 600.0);
        let t = measure(w, |_, i| {
            let dy = if (i / 100) % 2 == 0 { -60.0 } else { 60.0 };
            wheel(egui::pos2(900.0, 800.0), egui::vec2(0.0, dy))
        });
        report(&format!("List-Editor {n} Events"), &t);
    }
}

#[test]
#[ignore = "Messung, kein Funktionstest"]
fn perf_video_track() {
    // 300 s Video, ein Keyframe je Sekunde: 300 Thumbnails.
    let bytes = crate::video::tests::webm(3000);
    for (zoom, name) in [(1.0, "nah"), (0.02, "ganzes Video")] {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.show_lower = false;
        let t = w
            .project
            .add_video_track("Film", "/test/Film.webm", 600 * 960, None);
        let thumbs = crate::video::thumbnails(&bytes).unwrap();
        let count = thumbs.len();
        w.videos.insert("/test/Film.webm", thumbs);
        w.selected_track = Some(t);
        w.view.zoom(zoom, None);
        let m = measure(w, |w, _| {
            w.view.scroll_ticks = (w.view.scroll_ticks + 50.0) % 100_000.0;
            vec![]
        });
        report(&format!("Videospur {} Thumbs, {name}", count), &m);
    }
}

/// Zahl der Shapes eines Bildes (nach dem Einschwingen).
fn shape_count(mut w: ProjectWindow) -> usize {
    let ctx = egui::Context::default();
    let mut count = 0;
    for _ in 0..3 {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)),
                ..Default::default()
            },
            |ui| w.ui(ui),
        );
        out.textures_delta.clear();
        count = out.shapes.len();
    }
    count
}

/// Rückfall-Test fürs Bildbudget: Key- und Drum-Editor zeichnen nur sichtbare Noten. Gezählt werden
/// Shapes statt Millisekunden, damit der Test in der CI nicht wackelt.
#[test]
fn key_editor_paints_only_visible_notes() {
    let shapes = |n: usize, drums: bool| {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let part = big_part(&mut w, n);
        if drums {
            w.project.tracks[0].drum_map = Some(studiocraft_project::DrumMap::general_midi());
        }
        w.open_key_editor(part);
        w.key_editor.selection = (0..n).step_by(3).collect();
        lower(&mut w, LowerZone::Editor, 600.0);
        shape_count(w)
    };
    let (small, big) = (shapes(200, false), shapes(20_000, false));
    // Sichtbar sind in beiden Fällen dieselben Noten: die Zahl der Shapes wächst nicht mit.
    assert!(
        big <= small + 50,
        "20 000 Noten: {big} Shapes, 200 Noten: {small} Shapes"
    );
    // Ebenso der Drum-Editor (Rauten).
    let (small, big) = (shapes(200, true), shapes(20_000, true));
    assert!(
        big <= small + 50,
        "Drum-Editor, 20 000 Noten: {big} Shapes, 200 Noten: {small} Shapes"
    );
}
