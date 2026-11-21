//! Info-Zeile unter der Werkzeugzeile (docs/ui-parity.md, „Info-Zeile“; Spec „Editoren“,
//! gemeinsames Verhalten): zeigt die Werte des gewählten Events und lässt sie direkt ändern.
//! Eine Eingabe gilt beim Verlassen des Felds oder mit Enter; Escape verwirft sie.
//!
//! Mehrfachauswahl: Die Felder zeigen das früheste gewählte Event, die Beschriftungen sind
//! gelb. Zeitwerte wirken relativ auf alle (Anfang einen Takt später: alle einen Takt später),
//! Name und Stumm gelten für alle gleich. Jede Änderung ist ein Rückgängig-Schritt.

use crate::i18n::{tr, tr_dyn};
use egui::{Key, RichText, TextEdit, Ui};
use studiocraft_project::{EventId, Position};

use crate::ProjectWindow;

/// Eine Änderung aus der Info-Zeile.
#[derive(Debug, Clone, PartialEq)]
enum Edit {
    Name(String),
    Start(u64),
    End(u64),
    Length(u64),
    FadeIn(u64),
    FadeOut(u64),
    Mute(bool),
    /// Audio folgt dem Projekttempo (an) oder spielt 1:1 (aus).
    Musical(bool),
    /// Tempo der Datei in BPM (Audio, das dem Tempo folgt).
    FileTempo(f64),
    /// Art des Time-Stretch.
    Stretch(studiocraft_project::StretchMode),
}

/// Die gewählten Events, die es gibt, und das früheste davon (Anzeige und Bezug).
fn selected(w: &ProjectWindow) -> Option<(Vec<EventId>, EventId)> {
    let ids: Vec<EventId> = w
        .selection
        .iter()
        .copied()
        .filter(|&id| w.project.event(id).is_some())
        .collect();
    let first = ids
        .iter()
        .copied()
        .min_by_key(|&id| w.project.event(id).map(|e| (e.start.0, id)))?;
    Some((ids, first))
}

pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    // Umbrechend: in schmalen Fenstern rutschen die Felder in weitere Zeilen, statt hinter
    // dem Fensterrand zu verschwinden.
    ui.horizontal_wrapped(|ui| {
        let Some((ids, id)) = selected(w) else {
            ui.weak(tr("Keine Auswahl"));
            return;
        };
        let Some(ev) = w.project.event(id) else {
            return;
        };
        let multi = ids.len() > 1;
        if multi {
            let t = crate::theme::tokens(ui.ctx());
            ui.label(
                RichText::new(crate::i18n::trf("{n} Events", &[("n", &ids.len())])).color(t.solo),
            )
            .on_hover_text(tr("Werte des ersten Events; Zeitwerte ändern alle relativ"));
        }
        let tempo = &w.project.tempo;
        let mut edits = Vec::new();
        if let Some(name) = field(ui, tr("Name"), ev.name.clone(), 110.0, multi) {
            edits.push(Edit::Name(name));
        }
        let start = tempo.format_position(ev.start);
        if let Some(t) = field(ui, tr("Anfang"), start, 80.0, multi) {
            edits.extend(tempo.parse_position(&t).map(|p| Edit::Start(p.0)));
        }
        let end = tempo.format_position(Position(ev.end()));
        if let Some(t) = field(ui, tr("Ende"), end, 80.0, multi) {
            edits.extend(tempo.parse_position(&t).map(|p| Edit::End(p.0)));
        }
        if let Some(t) = field(ui, tr("Länge"), tempo.format_length(ev.length), 80.0, multi) {
            edits.extend(tempo.parse_length(&t).map(Edit::Length));
        }
        if let Some(t) = field(
            ui,
            tr("Fade-In"),
            tempo.format_length(ev.fade_in),
            80.0,
            multi,
        ) {
            edits.extend(tempo.parse_length(&t).map(Edit::FadeIn));
        }
        if let Some(t) = field(
            ui,
            tr("Fade-Out"),
            tempo.format_length(ev.fade_out),
            80.0,
            multi,
        ) {
            edits.extend(tempo.parse_length(&t).map(Edit::FadeOut));
        }
        let mut muted = ev.muted;
        if ui.checkbox(&mut muted, tr("Stumm")).changed() {
            edits.push(Edit::Mute(muted));
        }
        if ev.clip.is_some() {
            let mut musical = ev.musical.is_some();
            if ui
                .checkbox(&mut musical, tr("Folgt dem Tempo"))
                .on_hover_text(tr(
                    "Audio wird gedehnt, sodass es zum Projekttempo passt (Tonhöhe bleibt)",
                ))
                .changed()
            {
                edits.push(Edit::Musical(musical));
            }
            if let Some(bpm) = ev.musical
                && let Some(t) = field(ui, tr("Datei-Tempo"), format!("{bpm:.2}"), 60.0, multi)
                && let Ok(v) = t.trim().replace(',', ".").parse::<f64>()
                && v.is_finite()
            {
                edits.push(Edit::FileTempo(v));
            }
            if ev.musical.is_some() {
                let mut mode = ev.stretch_mode;
                let label = ui.weak(tr("Stretch"));
                egui::ComboBox::from_id_salt("stretch-art")
                    .selected_text(tr_dyn(mode.label()))
                    .show_ui(ui, |ui| {
                        for m in studiocraft_project::StretchMode::ALL {
                            ui.selectable_value(&mut mode, m, tr_dyn(m.label()));
                        }
                    })
                    .response
                    .labelled_by(label.id)
                    .on_hover_text(tr(
                        "Rhythmisch: knackige Anschläge · Flächig: weiche Klänge, mehr Rechenzeit",
                    ));
                if mode != ev.stretch_mode {
                    edits.push(Edit::Stretch(mode));
                }
            }
        }
        for edit in edits {
            apply(w, &ids, id, edit);
        }
    });
}

/// Beschriftetes Textfeld. Gibt den neuen Text zurück, sobald die Eingabe abgeschlossen ist.
/// `multi`: Mehrfachauswahl, die Beschriftung wird gelb.
pub(crate) fn field(
    ui: &mut Ui,
    label: &str,
    value: String,
    width: f32,
    multi: bool,
) -> Option<String> {
    let id = ui.make_persistent_id(("infozeile", label));
    // Beschriftung und Feld bleiben beim Umbrechen beisammen.
    let label_width = egui::WidgetText::from(label)
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        )
        .size()
        .x;
    let margin = ui.spacing().button_padding.x * 2.0;
    keep_together(
        ui,
        label_width + ui.spacing().item_spacing.x + width + margin,
    );
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| value.clone());
    let l = if multi {
        let t = crate::theme::tokens(ui.ctx());
        ui.label(RichText::new(label).color(t.warning))
    } else {
        ui.weak(label)
    };
    let r = ui
        .add(
            TextEdit::singleline(&mut text)
                .desired_width(width)
                .id(id.with("feld")),
        )
        .labelled_by(l.id);
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
        return None;
    }
    ui.data_mut(|d| d.remove_temp::<String>(id));
    let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
    (r.lost_focus() && !cancelled && text != value).then_some(text)
}

/// Beginnt in einer umbrechenden Zeile eine neue Zeile, wenn `width` nicht mehr in die laufende
/// passt. In nicht umbrechenden Layouts ohne Wirkung.
pub(crate) fn keep_together(ui: &mut Ui, width: f32) {
    let at_row_start = ui.cursor().left() <= ui.max_rect().left() + 0.5;
    if !at_row_start && ui.available_size_before_wrap().x < width {
        ui.end_row();
    }
}

/// Wendet `edit` an: auf das Bezugs-Event `reference` absolut, auf alle anderen in `ids`
/// relativ (gleicher Abstand); Name und Stumm für alle gleich. Ein Rückgängig-Schritt.
fn apply(w: &mut ProjectWindow, ids: &[EventId], reference: EventId, edit: Edit) {
    let Some(r) = w.project.event(reference).cloned() else {
        return;
    };
    let before = w.project.clone();
    let delta = |new: u64, old: u64| i128::from(new) - i128::from(old);
    let shift = |v: u64, d: i128| u64::try_from((i128::from(v) + d).max(0)).unwrap_or(u64::MAX);
    for &id in ids {
        let Some(ev) = w.project.event(id).cloned() else {
            continue;
        };
        let own = match &edit {
            Edit::Name(_)
            | Edit::Mute(_)
            | Edit::Musical(_)
            | Edit::FileTempo(_)
            | Edit::Stretch(_) => edit.clone(),
            Edit::Start(t) => Edit::Start(shift(ev.start.0, delta(*t, r.start.0))),
            Edit::End(t) => Edit::End(shift(ev.end(), delta(*t, r.end()))),
            Edit::Length(l) => Edit::Length(shift(ev.length, delta(*l, r.length)).max(1)),
            Edit::FadeIn(f) => Edit::FadeIn(shift(ev.fade_in, delta(*f, r.fade_in))),
            Edit::FadeOut(f) => Edit::FadeOut(shift(ev.fade_out, delta(*f, r.fade_out))),
        };
        apply_one(&mut w.project, id, own);
    }
    w.checkpoint(before);
}

/// Wendet einen absoluten Wert auf ein Event an.
fn apply_one(p: &mut studiocraft_project::Project, id: EventId, edit: Edit) {
    match edit {
        Edit::Name(name) => {
            if let Some(ev) = p.event_mut(id) {
                ev.name = name;
            }
        }
        Edit::Start(start) => {
            if let Some((track, _)) = p.find(id) {
                p.move_event(id, track, start, false);
            }
        }
        Edit::End(end) => {
            p.trim_end(id, end);
        }
        Edit::Length(len) => {
            if let Some(start) = p.event(id).map(|e| e.start.0) {
                p.trim_end(id, start.saturating_add(len));
            }
        }
        Edit::FadeIn(fade) => {
            if let Some(out) = p.event(id).map(|e| e.fade_out) {
                // Ein längerer Fade-In kürzt den Fade-Out, nicht umgekehrt.
                let len = p.event(id).map_or(0, |e| e.length);
                p.set_fades(id, fade.min(len), out.min(len.saturating_sub(fade)));
            }
        }
        Edit::FadeOut(fade) => {
            if let Some(fade_in) = p.event(id).map(|e| e.fade_in) {
                p.set_fades(id, fade_in, fade);
            }
        }
        Edit::Mute(m) => {
            if let Some(ev) = p.event_mut(id) {
                ev.muted = m;
            }
        }
        Edit::Musical(on) => {
            let Some(ev) = p.event(id) else { return };
            if on == ev.musical.is_some() {
                return;
            }
            // Startwert: ein Tempo im Namen der Datei, sonst das Projekttempo am Event (dann
            // ändert sich zunächst nichts hörbar).
            let named = ev
                .clip
                .and_then(|c| p.pool.get(c as usize))
                .and_then(|c| studiocraft_project::bpm_in_name(&c.name))
                .or_else(|| studiocraft_project::bpm_in_name(&ev.name));
            let bpm = named.unwrap_or_else(|| p.tempo.bpm_at(ev.start.0));
            p.set_musical(id, on.then_some(bpm));
        }
        Edit::FileTempo(bpm) => {
            if p.event(id).is_some_and(|e| e.musical.is_some()) {
                p.set_musical(id, Some(bpm));
            }
        }
        Edit::Stretch(mode) => {
            if let Some(ev) = p.event_mut(id).filter(|e| e.musical.is_some()) {
                ev.stretch_mode = mode;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_follows_the_tempo_with_the_tempo_from_its_name() {
        let mut project = studiocraft_project::Project::default();
        project.add_track(studiocraft_project::TrackKind::Audio, "A");
        let c = project.add_clip(studiocraft_project::PoolClip {
            name: "Drums 90bpm".into(),
            path: None,
            sample_rate: 48_000,
            frames: 48_000,
        });
        let id = project.add_audio_event(0, 0, c).unwrap();
        let mut w = ProjectWindow::new(project, None, String::new());
        apply(&mut w, &[id], id, Edit::Musical(true));
        let ev = w.project.event(id).unwrap();
        assert_eq!((ev.musical, ev.length), (Some(90.0), 1440));
        apply(&mut w, &[id], id, Edit::FileTempo(120.0));
        assert_eq!(w.project.event(id).unwrap().length, 1920);
        apply(
            &mut w,
            &[id],
            id,
            Edit::Stretch(studiocraft_project::StretchMode::Pad),
        );
        assert_eq!(
            w.project.event(id).unwrap().stretch_mode,
            studiocraft_project::StretchMode::Pad
        );
        apply(&mut w, &[id], id, Edit::Musical(false));
        assert_eq!(w.project.event(id).unwrap().musical, None);
        // Jede Änderung ist ein Schritt.
        w.undo();
        assert_eq!(w.project.event(id).unwrap().musical, Some(120.0));
        w.undo();
        assert!(w.project.event(id).unwrap().stretch_mode.is_standard());
    }

    fn window() -> (ProjectWindow, EventId) {
        let mut project = studiocraft_project::Project::default();
        project.add_track(studiocraft_project::TrackKind::Instrument, "Synth");
        let id = project.create_event(0, 3840, 3840).expect("event");
        let mut w = ProjectWindow::new(project, None, String::new());
        w.selection = vec![id];
        (w, id)
    }

    #[test]
    fn edits_move_trim_fade_and_mute_with_undo() {
        let (mut w, id) = window();
        apply(&mut w, &[id], id, Edit::Start(7680));
        assert_eq!(
            w.project.event(id).map(|e| (e.start.0, e.length)),
            Some((7680, 3840))
        );
        apply(&mut w, &[id], id, Edit::Length(960));
        assert_eq!(w.project.event(id).map(|e| e.length), Some(960));
        apply(&mut w, &[id], id, Edit::End(7680 + 1920));
        assert_eq!(w.project.event(id).map(|e| e.length), Some(1920));
        apply(&mut w, &[id], id, Edit::FadeOut(480));
        apply(&mut w, &[id], id, Edit::FadeIn(1800));
        let ev = w.project.event(id).cloned().expect("event");
        assert_eq!((ev.fade_in, ev.fade_out), (1800, 120));
        apply(&mut w, &[id], id, Edit::Mute(true));
        apply(&mut w, &[id], id, Edit::Name("Refrain".into()));
        let ev = w.project.event(id).cloned().expect("event");
        assert!(ev.muted);
        assert_eq!(ev.name, "Refrain");
        for _ in 0..7 {
            w.undo();
        }
        let ev = w.project.event(id).cloned().expect("event");
        assert_eq!((ev.start.0, ev.length, ev.muted), (3840, 3840, false));
    }

    #[test]
    fn several_events_change_relative_to_the_first() {
        let (mut w, a) = window();
        let b = w.project.create_event(0, 9600, 1920).expect("b");
        w.selection = vec![b, a];
        let (ids, first) = selected(&w).expect("selection");
        assert_eq!(first, a, "the earliest event is the reference");
        let span = |w: &ProjectWindow, id| w.project.event(id).map(|e| (e.start.0, e.length));
        // Start of the first one bar later: both move one bar.
        apply(&mut w, &ids, first, Edit::Start(3840 + 3840));
        assert_eq!(span(&w, a), Some((7680, 3840)));
        assert_eq!(span(&w, b), Some((13440, 1920)));
        // Length a beat shorter: both a beat shorter.
        apply(&mut w, &ids, first, Edit::Length(3840 - 960));
        assert_eq!(span(&w, a), Some((7680, 2880)));
        assert_eq!(span(&w, b), Some((13440, 960)));
        // Much shorter still keeps every event at least one tick long.
        apply(&mut w, &ids, first, Edit::Length(1));
        assert_eq!(span(&w, b), Some((13440, 1)));
        apply(&mut w, &ids, first, Edit::Mute(true));
        apply(&mut w, &ids, first, Edit::Name("Chor".into()));
        for id in [a, b] {
            let ev = w.project.event(id).expect("event");
            assert!(ev.muted);
            assert_eq!(ev.name, "Chor");
        }
        // Each change was one undo step for all events.
        for _ in 0..5 {
            w.undo();
        }
        assert_eq!(span(&w, a), Some((3840, 3840)));
        assert_eq!(span(&w, b), Some((9600, 1920)));
    }

    #[test]
    fn moving_earlier_than_zero_stops_at_the_start() {
        let (mut w, a) = window();
        let b = w.project.create_event(0, 960, 960).expect("b");
        w.selection = vec![a, b];
        let (ids, first) = selected(&w).expect("selection");
        assert_eq!(first, b);
        // b to 0 is 960 earlier; a follows by the same amount.
        apply(&mut w, &ids, first, Edit::Start(0));
        assert_eq!(w.project.event(a).map(|e| e.start.0), Some(2880));
        assert_eq!(w.project.event(b).map(|e| e.start.0), Some(0));
    }

    #[test]
    fn end_before_start_keeps_a_minimal_event() {
        let (mut w, id) = window();
        apply(&mut w, &[id], id, Edit::End(0));
        assert_eq!(w.project.event(id).map(|e| e.length), Some(1));
    }
}
