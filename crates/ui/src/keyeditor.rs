//! Key-Editor (Spec-Abschnitt „Editoren“): Noten als Balken über einer
//! Klaviatur, darunter die Velocity-Spur. Läuft in der unteren Zone.
//!
//! Werkzeuge wie im Projektfenster: Auswahl (verschieben, Strg kopiert, am
//! rechten Rand Länge ändern), Zeichnen (Note setzen, Länge aufziehen),
//! Löschen. Alt hebt Snap auf. Klick auf die Klaviatur spielt den Ton an.

use crate::i18n::tr;
use egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use studiocraft_engine::Command;
use studiocraft_project::{EventId, Note};

use crate::{ProjectWindow, TPQ, Tool};

pub(crate) mod drum;
mod lanes;

const KEYS_W: f32 = 52.0;
const RULER_H: f32 = 20.0;
const VEL_H: f32 = 70.0;
/// Fangbereich am rechten Notenrand in Pixel.
const EDGE: f32 = 6.0;

#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyView {
    pub px_per_quarter: f32,
    pub scroll_ticks: f64,
    pub key_h: f32,
    /// Oberkante der Ansicht in Pixel ab Tonhöhe 127.
    pub scroll_y: f32,
}

impl Default for KeyView {
    fn default() -> Self {
        // C3 bis C6 grob in der Mitte.
        Self {
            px_per_quarter: 60.0,
            scroll_ticks: 0.0,
            key_h: 11.0,
            scroll_y: (127.0 - 84.0) * 11.0,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum NoteDrag {
    /// Auswahl verschieben: Startzustand der Noten, Ankerpunkt (Tick, Tonhöhe).
    Move {
        original: Vec<Note>,
        anchor_tick: f64,
        anchor_pitch: i32,
        grab_offset: f64,
    },
    /// Länge der Auswahl am rechten Rand ändern.
    Resize { original: Vec<Note>, note: usize },
    /// Velocity in der unteren Spur ziehen.
    Velocity,
}

/// Zustand des Key-Editors (welcher Part, Auswahl, Ansicht).
#[derive(Debug, Clone)]
pub(crate) struct KeyEditor {
    /// Aktiver Part: nur seine Noten werden bearbeitet.
    pub part: Option<EventId>,
    /// Alle geöffneten Parts in fester Reihenfolge (Reiter), der aktive darunter. Die
    /// anderen erscheinen gedimmt; ein Klick auf eine ihrer Noten macht ihren Part aktiv.
    pub open: Vec<EventId>,
    pub view: KeyView,
    pub selection: Vec<usize>,
    pub drag: Option<NoteDrag>,
    /// Gerade angespielte Taste (wird beim Loslassen beendet).
    pub preview: Option<u8>,
    /// Der Editor hatte den letzten Klick: Entf usw. wirken auf Noten.
    pub focused: bool,
    /// Akustisches Feedback: angeklickte und verschobene Noten erklingen.
    pub feedback: bool,
    /// Controller-Spuren unter der Velocity-Spur (lanes.rs).
    pub lanes: Vec<studiocraft_project::CtrlKind>,
    /// Gewählte Controller-Punkte (Indizes in `Event::controllers` des aktiven Parts).
    pub ctrl_sel: Vec<usize>,
    pub ctrl_drag: Option<lanes::CtrlDrag>,
    /// Lage des Notenrasters und der Velocity-Spur im letzten Frame (für Tests).
    pub grid_rect: Rect,
    pub vel_rect: Rect,
    /// Drum-Editor: Oberkante der Ansicht in Pixel ab der ersten Zeile (drum.rs).
    pub drum_scroll_y: f32,
}

impl Default for KeyEditor {
    fn default() -> Self {
        Self {
            part: None,
            open: Vec::new(),
            view: KeyView::default(),
            selection: Vec::new(),
            drag: None,
            preview: None,
            focused: false,
            feedback: true,
            lanes: Vec::new(),
            ctrl_sel: Vec::new(),
            ctrl_drag: None,
            grid_rect: Rect::NOTHING,
            vel_rect: Rect::NOTHING,
            drum_scroll_y: 0.0,
        }
    }
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "H",
];

fn is_black(pitch: i32) -> bool {
    matches!(pitch.rem_euclid(12), 1 | 3 | 6 | 8 | 10)
}

/// Tonname mit mittlerem C als C3 (C3 = 60).
pub(crate) fn note_name(pitch: u8) -> String {
    format!(
        "{}{}",
        crate::i18n::tr(NOTE_NAMES[usize::from(pitch % 12)]),
        i32::from(pitch) / 12 - 2
    )
}

struct KGeo {
    keys: Rect,
    ruler: Rect,
    grid: Rect,
    vel: Rect,
    view: KeyView,
    part_start: u64,
}

impl KGeo {
    fn x(&self, abs_tick: f64) -> f32 {
        self.grid.left()
            + ((abs_tick - self.view.scroll_ticks) / TPQ) as f32 * self.view.px_per_quarter
    }
    fn tick(&self, x: f32) -> f64 {
        self.view.scroll_ticks + f64::from((x - self.grid.left()) / self.view.px_per_quarter) * TPQ
    }
    fn y(&self, pitch: i32) -> f32 {
        self.grid.top() + (127 - pitch) as f32 * self.view.key_h - self.view.scroll_y
    }
    fn pitch(&self, y: f32) -> i32 {
        (127.0 - ((y - self.grid.top() + self.view.scroll_y) / self.view.key_h).floor()) as i32
    }
    fn note_rect(&self, n: &Note) -> Rect {
        let a = self.part_start as f64 + n.start as f64;
        let b = a + n.length as f64;
        let y = self.y(i32::from(n.pitch));
        Rect::from_min_max(
            pos2(self.x(a), y + 1.0),
            pos2(self.x(b).max(self.x(a) + 3.0), y + self.view.key_h - 1.0),
        )
    }
}

impl ProjectWindow {
    /// Öffnet den Key-Editor für einen Part in der unteren Zone.
    pub(crate) fn open_key_editor(&mut self, part: EventId) {
        let Some(ev) = self.project.event(part) else {
            return;
        };
        let (start, notes) = (ev.start.0, ev.notes.clone());
        self.key_editor.part = Some(part);
        self.key_editor.open = vec![part];
        self.key_editor.ctrl_sel.clear();
        self.key_editor.selection.clear();
        self.key_editor.view.scroll_ticks = start as f64;
        if let Some(mid) = notes.iter().map(|n| i32::from(n.pitch)).max() {
            let lo = notes
                .iter()
                .map(|n| i32::from(n.pitch))
                .min()
                .unwrap_or(mid);
            let center = (mid + lo) / 2;
            self.key_editor.view.scroll_y =
                ((127 - center) as f32 - 8.0) * self.key_editor.view.key_h;
        }
        self.lower = crate::LowerZone::Editor;
        self.show_lower = true;
    }

    /// Akustisches Feedback für eine Note (nur wenn eingeschaltet).
    pub(crate) fn note_feedback(&mut self, pitch: u8) {
        if self.key_editor.feedback {
            self.preview(Some(pitch));
        }
    }

    /// Mehrere Parts öffnen; der erste wird aktiv. Nur MIDI-Parts (ohne Audio-Clip).
    pub(crate) fn open_key_editor_parts(&mut self, ids: &[EventId]) {
        let parts: Vec<EventId> = ids
            .iter()
            .copied()
            .filter(|&id| {
                self.project
                    .event(id)
                    .is_some_and(|e| e.clip.is_none() && !e.is_audio_part())
            })
            .collect();
        let Some(&first) = parts.first() else {
            return;
        };
        self.open_key_editor(first);
        self.key_editor.open = parts;
    }

    /// Doppelklick bzw. Strg+E auf `id`: Ist `id` Teil einer Auswahl aus mehreren Parts,
    /// öffnen alle, `id` aktiv; sonst nur `id`.
    pub(crate) fn open_key_editor_from_selection(&mut self, id: EventId) {
        if self.selection.len() > 1 && self.selection.contains(&id) {
            let mut ids = vec![id];
            ids.extend(self.selection.iter().copied().filter(|&s| s != id));
            self.open_key_editor_parts(&ids);
        } else {
            self.open_key_editor(id);
        }
    }

    /// Macht einen der geöffneten Parts aktiv (Ansicht bleibt, Auswahl leert sich).
    pub(crate) fn activate_part(&mut self, id: EventId) {
        if self.key_editor.open.contains(&id) && self.project.event(id).is_some() {
            self.key_editor.part = Some(id);
            self.key_editor.selection.clear();
            self.key_editor.ctrl_sel.clear();
            self.key_editor.drag = None;
        }
    }

    /// Die geöffneten Parts außer dem aktiven, die es noch gibt.
    fn other_parts(&self) -> Vec<EventId> {
        self.key_editor
            .open
            .iter()
            .copied()
            .filter(|&id| Some(id) != self.key_editor.part && self.project.event(id).is_some())
            .collect()
    }

    fn part_track(&self) -> Option<usize> {
        self.key_editor
            .part
            .and_then(|p| self.project.find(p))
            .map(|(t, _)| t)
    }

    fn preview(&mut self, key: Option<u8>) {
        let Some(track) = self.part_track() else {
            return;
        };
        if let Some(old) = self.key_editor.preview.take() {
            self.send(Command::LiveNote(track, old, 0, false));
        }
        if let Some(k) = key {
            self.send(Command::LiveNote(track, k, 100, true));
            self.key_editor.preview = Some(k);
        }
    }

    /// Notenänderungen an einem Part (mit Undo-Schritt).
    fn edit_notes(&mut self, part: EventId, f: impl FnOnce(&mut Vec<Note>)) {
        let before = self.project.clone();
        if let Some(ev) = self.project.event_mut(part) {
            f(&mut ev.notes);
        }
        self.checkpoint(before);
    }

    pub(crate) fn delete_selected_notes(&mut self) {
        // Gewählte Controller-Punkte haben Vorrang (zuletzt in einer Controller-Spur gewählt).
        if !self.key_editor.ctrl_sel.is_empty() {
            self.delete_selected_ctrls();
            return;
        }
        let Some(part) = self.key_editor.part else {
            return;
        };
        let mut sel = std::mem::take(&mut self.key_editor.selection);
        sel.sort_unstable();
        self.edit_notes(part, |notes| {
            for i in sel.into_iter().rev() {
                if i < notes.len() {
                    notes.remove(i);
                }
            }
        });
    }
}

pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    let Some(part) = w.key_editor.part.filter(|p| w.project.event(*p).is_some()) else {
        ui.centered_and_justified(|ui| {
            ui.weak(tr(
                "Doppelklick auf einen Part öffnet ihn hier im Key-Editor",
            ))
        });
        return;
    };
    // Spur mit Drum-Map: Drum-Editor statt Klaviatur (drum.rs).
    if let Some((track, _)) = w.project.find(part)
        && let Some(map) = w.project.tracks[track].drum_map.clone()
    {
        drum::show(w, ui, part, track, map);
        return;
    }
    let Some(ev) = w.project.event(part).cloned() else {
        return;
    };
    let color = w
        .project
        .find(part)
        .map(|(t, _)| w.project.tracks[t].color)
        .map_or(Color32::LIGHT_BLUE, |c| Color32::from_rgb(c[0], c[1], c[2]));

    ui.horizontal(|ui| {
        ui.toggle_value(&mut w.key_editor.feedback, tr("Akustisches Feedback"))
            .on_hover_text(tr("Angeklickte und verschobene Noten erklingen"));
        ui.toggle_value(&mut w.follow_playback, tr("Folgen (F)"))
            .on_hover_text(tr("Die Ansicht folgt dem Cursor während der Wiedergabe"));
        // Reiter der geöffneten Parts, wenn es mehrere sind.
        let others = w.other_parts();
        if !others.is_empty() {
            ui.separator();
            let open: Vec<EventId> = w
                .key_editor
                .open
                .iter()
                .copied()
                .filter(|&id| w.project.event(id).is_some())
                .collect();
            for id in open {
                let name = w
                    .project
                    .event(id)
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                if ui
                    .selectable_label(Some(id) == w.key_editor.part, name)
                    .clicked()
                {
                    w.activate_part(id);
                }
            }
        }
        ui.menu_button(tr("+ Controller-Spur"), |ui| {
            if let Some(k) = lanes::kind_menu(ui) {
                w.add_ctrl_lane(k);
                ui.close();
            }
        });
        // Info-Zeile der gewählten Noten (noteinfo.rs).
        if !w.key_editor.selection.is_empty() {
            ui.separator();
            w.note_info(ui, part);
        }
    });
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let r = resp.rect;
    // Unten: Velocity-Spur, darunter die Controller-Spuren.
    let lanes_h = w.key_editor.lanes.len() as f32 * lanes::CTRL_H;
    let bottom_h = VEL_H + lanes_h;
    let keys = Rect::from_min_max(
        pos2(r.left(), r.top() + RULER_H),
        pos2(r.left() + KEYS_W, r.bottom() - bottom_h),
    );
    let ruler = Rect::from_min_max(
        pos2(r.left() + KEYS_W, r.top()),
        pos2(r.right(), r.top() + RULER_H),
    );
    let grid = Rect::from_min_max(
        pos2(r.left() + KEYS_W, r.top() + RULER_H),
        pos2(r.right(), r.bottom() - bottom_h),
    );
    let vel = Rect::from_min_max(
        pos2(r.left() + KEYS_W, r.bottom() - bottom_h + 4.0),
        pos2(r.right(), r.bottom() - lanes_h),
    );
    let ctrl_lanes: Vec<lanes::Lane> = w
        .key_editor
        .lanes
        .iter()
        .enumerate()
        .map(|(index, &kind)| {
            let top = r.bottom() - lanes_h + index as f32 * lanes::CTRL_H;
            lanes::Lane {
                index,
                kind,
                rect: Rect::from_min_max(
                    pos2(r.left() + KEYS_W, top + 2.0),
                    pos2(r.right(), top + lanes::CTRL_H),
                ),
            }
        })
        .collect();
    // Taste F: während der Wiedergabe mitblättern, wenn der Cursor hinausläuft.
    if w.follow_playback && w.is_playing() {
        let v = &mut w.key_editor.view;
        let visible = f64::from(grid.width() / v.px_per_quarter) * TPQ;
        let c = w.cursor as f64;
        if c > v.scroll_ticks + visible * 0.95 || c < v.scroll_ticks {
            v.scroll_ticks = (c - visible * 0.05).max(0.0);
        }
    }

    // Scrollen und Zoomen wie im Projektfenster.
    if resp.hovered() {
        let (delta, zoom, mods, hover) = ui.input(|i| {
            (
                i.smooth_scroll_delta,
                i.zoom_delta(),
                i.modifiers,
                i.pointer.hover_pos(),
            )
        });
        let v = &mut w.key_editor.view;
        if zoom != 1.0 {
            let ax = hover.map_or(0.0, |p| p.x - grid.left());
            let anchor = v.scroll_ticks + f64::from(ax / v.px_per_quarter) * TPQ;
            v.px_per_quarter = (v.px_per_quarter * zoom).clamp(4.0, 2000.0);
            v.scroll_ticks = (anchor - f64::from(ax / v.px_per_quarter) * TPQ).max(0.0);
        } else if mods.shift {
            v.scroll_ticks =
                (v.scroll_ticks - f64::from((delta.y + delta.x) / v.px_per_quarter) * TPQ).max(0.0);
        } else {
            v.scroll_y -= delta.y;
            v.scroll_ticks =
                (v.scroll_ticks - f64::from(delta.x / v.px_per_quarter) * TPQ).max(0.0);
        }
        v.scroll_y = v
            .scroll_y
            .clamp(0.0, (128.0 * v.key_h - grid.height()).max(0.0));
    }
    let g = KGeo {
        keys,
        ruler,
        grid,
        vel,
        view: w.key_editor.view,
        part_start: ev.start.0,
    };
    w.key_editor.grid_rect = grid;
    w.key_editor.vel_rect = vel;

    if !lanes::interact(w, ui, &resp, &g, part, &ctrl_lanes) {
        interact(w, ui, &resp, &g, part, &ev.notes);
    }

    // Nur Sichtbares zeichnen: ein Part kann zehntausende Noten haben (AGENTS.md: Budget je
    // Bild). Die Noten sind nicht sortiert, also ein linearer Test je Note ohne Kopie.
    let notes: &[Note] = w.project.event(part).map_or(&[], |e| &e.notes);
    let mut selection = w.key_editor.selection.clone();
    selection.sort_unstable();
    let p = painter.with_clip_rect(grid);
    paint_grid(w, &p, &g, ev.start.0, ev.end());
    // Die anderen geöffneten Parts gedimmt darunter.
    for other in w.other_parts() {
        let Some(o) = w.project.event(other) else {
            continue;
        };
        let c = w
            .project
            .find(other)
            .map(|(t, _)| w.project.tracks[t].color)
            .map_or(Color32::GRAY, |c| Color32::from_rgb(c[0], c[1], c[2]))
            .gamma_multiply(0.35);
        let og = KGeo {
            part_start: o.start.0,
            ..g
        };
        for n in &o.notes {
            let rect = og.note_rect(n);
            if rect.intersects(grid) {
                p.rect_filled(rect, 2.0, c);
            }
        }
    }
    let selected_stroke = Stroke::new(2.0, crate::theme::tokens(p.ctx()).note_selected);
    let normal_stroke = Stroke::new(1.0, Color32::from_black_alpha(140));
    for (i, n) in notes.iter().enumerate() {
        let rect = g.note_rect(n);
        if !rect.intersects(grid) {
            continue;
        }
        let selected = selection.binary_search(&i).is_ok();
        let inside = n.start >= 0 && n.start < ev.length as i64;
        let fill = color.lerp_to_gamma(Color32::WHITE, 0.1 + 0.4 * f32::from(n.velocity) / 127.0);
        let fill = if inside {
            fill
        } else {
            fill.gamma_multiply(0.3)
        };
        p.rect_filled(rect, 2.0, fill);
        let stroke = if selected {
            selected_stroke
        } else {
            normal_stroke
        };
        p.rect_stroke(rect, 2.0, stroke, StrokeKind::Inside);
    }
    paint_ruler(w, &painter.with_clip_rect(ruler), &g);
    paint_keys(&painter.with_clip_rect(keys), &g, w.key_editor.preview);
    paint_velocity(&painter.with_clip_rect(vel), &g, notes, &selection, color);
    let ctrls = w
        .project
        .event(part)
        .map(|e| e.controllers.clone())
        .unwrap_or_default();
    lanes::paint(w, &painter, &g, &ctrl_lanes, &ctrls, color);
    lanes::heads(w, ui, &ctrl_lanes, r.left());
    // Projekt-Cursor
    let cx = g.x(w.cursor as f64);
    painter.with_clip_rect(ruler.union(grid)).vline(
        cx,
        ruler.top()..=grid.bottom(),
        Stroke::new(1.5, crate::theme::tokens(ui.ctx()).playhead),
    );
}

fn snap(w: &ProjectWindow, t: f64, free: bool) -> f64 {
    if free {
        t.round().max(0.0)
    } else {
        w.snap_ticks(t) as f64
    }
}

fn hit_note(g: &KGeo, notes: &[Note], p: Pos2) -> Option<usize> {
    notes
        .iter()
        .enumerate()
        .rev()
        .find(|(_, n)| g.note_rect(n).expand(1.0).contains(p))
        .map(|(i, _)| i)
}

fn interact(
    w: &mut ProjectWindow,
    ui: &Ui,
    resp: &egui::Response,
    g: &KGeo,
    part: EventId,
    notes: &[Note],
) {
    let (mods, press, now, released) = ui.input(|i| {
        (
            i.modifiers,
            i.pointer.press_origin(),
            i.pointer.interact_pos(),
            i.pointer.any_released(),
        )
    });
    // Beim Klick ist die Taste schon wieder oben: dann zählt die Klickposition.
    let press = press.or_else(|| resp.interact_pointer_pos());
    if resp.clicked() || resp.drag_started() {
        w.key_editor.focused = true;
        w.selected_track = w.part_track();
    }
    if released && w.key_editor.preview.is_some() && w.key_editor.drag.is_none() {
        w.preview(None);
    }

    // Klaviatur: anspielen.
    if (resp.clicked() || resp.drag_started())
        && let Some(p) = press.filter(|p| g.keys.contains(*p))
    {
        w.preview(Some(g.pitch(p.y).clamp(0, 127) as u8));
        return;
    }

    // Velocity-Spur.
    if (resp.drag_started() || resp.clicked()) && press.is_some_and(|p| g.vel.contains(p)) {
        w.drag_before = Some(w.project.clone());
        w.key_editor.drag = Some(NoteDrag::Velocity);
    }
    if matches!(w.key_editor.drag, Some(NoteDrag::Velocity))
        && let Some(p) = now.or(press)
    {
        let v = (((g.vel.bottom() - p.y) / g.vel.height()) * 127.0).clamp(1.0, 127.0) as u8;
        let t = g.tick(p.x) - g.part_start as f64;
        let sel = w.key_editor.selection.clone();
        if let Some(ev) = w.project.event_mut(part) {
            for (i, n) in ev.notes.iter_mut().enumerate() {
                let under = (n.start as f64) <= t && t < n.end() as f64;
                if under && (sel.is_empty() || sel.contains(&i)) {
                    n.velocity = v;
                }
            }
        }
        w.dirty = true;
        if resp.drag_stopped() || resp.clicked() {
            w.key_editor.drag = None;
            if let Some(b) = w.drag_before.take() {
                w.checkpoint(b);
            }
        }
        return;
    }

    let in_grid = |p: Pos2| g.grid.contains(p);

    // Klick auf eine gedimmte Note eines anderen geöffneten Parts: dieser wird aktiv.
    if (resp.clicked() || resp.drag_started())
        && let Some(p) = press.filter(|p| in_grid(*p))
        && hit_note(g, notes, p).is_none()
    {
        for other in w.other_parts() {
            let Some(o) = w.project.event(other) else {
                continue;
            };
            let og = KGeo {
                part_start: o.start.0,
                ..*g
            };
            if let Some(i) = hit_note(&og, &o.notes, p) {
                w.activate_part(other);
                w.key_editor.selection = vec![i];
                return;
            }
        }
    }

    // Ziehen beginnen.
    if resp.drag_started()
        && let Some(p) = press.filter(|p| in_grid(*p))
    {
        w.drag_before = Some(w.project.clone());
        let tick = g.tick(p.x);
        match w.tool {
            Tool::Select => {
                if let Some(i) = hit_note(g, notes, p) {
                    if !w.key_editor.selection.contains(&i) {
                        w.key_editor.selection = vec![i];
                    }
                    let n = notes[i];
                    let rect = g.note_rect(&n);
                    w.key_editor.drag = Some(if (rect.right() - p.x).abs() <= EDGE {
                        NoteDrag::Resize {
                            original: notes.to_vec(),
                            note: i,
                        }
                    } else {
                        w.note_feedback(n.pitch);
                        NoteDrag::Move {
                            original: notes.to_vec(),
                            anchor_tick: tick,
                            anchor_pitch: i32::from(n.pitch),
                            grab_offset: tick - (g.part_start as f64 + n.start as f64),
                        }
                    });
                }
            }
            Tool::Draw => {
                let pitch = g.pitch(p.y).clamp(0, 127) as u8;
                let start = snap(w, tick, mods.alt) as i64 - g.part_start as i64;
                let len = w.grid_ticks();
                if let Some(ev) = w.project.event_mut(part) {
                    ev.notes.push(Note {
                        pitch,
                        velocity: 100,
                        start,
                        length: len,
                        spelling: 0,
                    });
                    let i = ev.notes.len() - 1;
                    w.key_editor.selection = vec![i];
                    w.key_editor.drag = Some(NoteDrag::Resize {
                        original: ev.notes.clone(),
                        note: i,
                    });
                }
                w.note_feedback(pitch);
            }
            _ => {}
        }
    }

    // Während des Ziehens.
    if resp.dragged()
        && let (Some(drag), Some(p)) = (w.key_editor.drag.clone(), now)
    {
        let sel = w.key_editor.selection.clone();
        match drag {
            NoteDrag::Move {
                original,
                anchor_tick,
                anchor_pitch,
                grab_offset,
            } => {
                let raw = g.tick(p.x) - grab_offset;
                let anchor_start = anchor_tick - grab_offset;
                let dt = snap(w, raw, mods.alt) - snap(w, anchor_start, mods.alt);
                let dp = g.pitch(p.y) - anchor_pitch;
                // Neue Tonhöhe der gegriffenen Note anspielen.
                let heard = (anchor_pitch + dp).clamp(0, 127) as u8;
                if w.key_editor.preview.is_some_and(|k| k != heard) {
                    w.note_feedback(heard);
                }
                let copy = mods.command;
                if let Some(ev) = w.project.event_mut(part) {
                    ev.notes = original.clone();
                    let moved: Vec<Note> = sel
                        .iter()
                        .filter_map(|&i| original.get(i))
                        .map(|n| Note {
                            start: n.start + dt as i64,
                            pitch: (i32::from(n.pitch) + dp).clamp(0, 127) as u8,
                            ..*n
                        })
                        .collect();
                    if copy {
                        ev.notes.extend(moved);
                    } else {
                        for (k, &i) in sel.iter().enumerate() {
                            if let Some(n) = ev.notes.get_mut(i) {
                                *n = moved[k];
                            }
                        }
                    }
                }
                let pitch = (anchor_pitch + dp).clamp(0, 127) as u8;
                if w.key_editor.preview != Some(pitch) {
                    w.preview(Some(pitch));
                }
            }
            NoteDrag::Resize { original, note } => {
                let end = snap(w, g.tick(p.x), mods.alt);
                let min = if mods.alt { 1.0 } else { w.grid_ticks() as f64 };
                if let (Some(ev), Some(anchor)) = (w.project.event_mut(part), original.get(note)) {
                    let anchor_end = g.part_start as f64 + anchor.end() as f64;
                    let delta = end - anchor_end;
                    ev.notes = original.clone();
                    let targets: Vec<usize> = if sel.contains(&note) {
                        sel.clone()
                    } else {
                        vec![note]
                    };
                    for i in targets {
                        if let (Some(n), Some(o)) = (ev.notes.get_mut(i), original.get(i)) {
                            n.length = (o.length as f64 + delta).max(min).round() as u64;
                        }
                    }
                }
            }
            NoteDrag::Velocity => {}
        }
        w.dirty = true;
    }

    if resp.drag_stopped() {
        w.key_editor.drag = None;
        w.preview(None);
        if let Some(b) = w.drag_before.take() {
            w.checkpoint(b);
        }
        return;
    }

    // Einzelklicks.
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos().filter(|p| in_grid(*p))
    {
        let hit = hit_note(g, notes, p);
        match (w.tool, hit) {
            (Tool::Select, Some(i)) => {
                if mods.shift {
                    if let Some(k) = w.key_editor.selection.iter().position(|&s| s == i) {
                        w.key_editor.selection.remove(k);
                    } else {
                        w.key_editor.selection.push(i);
                    }
                } else {
                    w.key_editor.selection = vec![i];
                }
                w.preview(Some(notes[i].pitch));
            }
            (Tool::Select, None) => w.key_editor.selection.clear(),
            (Tool::Erase, Some(i)) => {
                w.key_editor.selection.clear();
                w.edit_notes(part, |n| {
                    n.remove(i);
                });
            }
            (Tool::Draw, None) => {
                let pitch = g.pitch(p.y).clamp(0, 127) as u8;
                let start = snap(w, g.tick(p.x), mods.alt) as i64 - g.part_start as i64;
                let len = w.grid_ticks();
                w.edit_notes(part, |n| {
                    n.push(Note {
                        pitch,
                        velocity: 100,
                        start,
                        length: len,
                        spelling: 0,
                    })
                });
                if let Some(ev) = w.project.event(part) {
                    w.key_editor.selection = vec![ev.notes.len() - 1];
                }
                w.preview(Some(pitch));
            }
            _ => {}
        }
    }

    // Mauszeiger am rechten Notenrand.
    if w.tool == Tool::Select
        && let Some(p) = resp.hover_pos()
        && let Some(i) = hit_note(g, notes, p)
        && (g.note_rect(&notes[i]).right() - p.x).abs() <= EDGE
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
}

fn paint_grid(w: &ProjectWindow, p: &Painter, g: &KGeo, part_start: u64, part_end: u64) {
    let v = p.ctx().global_style().visuals.clone();
    p.rect_filled(g.grid, 0.0, v.extreme_bg_color);
    // Tastenreihen: schwarze Tasten dunkler, C-Linien kräftiger.
    let top = g.pitch(g.grid.top()).min(127);
    let bottom = g.pitch(g.grid.bottom()).max(0);
    for pitch in bottom..=top {
        let y = g.y(pitch);
        let row = Rect::from_min_max(
            pos2(g.grid.left(), y),
            pos2(g.grid.right(), y + g.view.key_h),
        );
        if is_black(pitch) {
            p.rect_filled(row, 0.0, crate::theme::tokens(p.ctx()).key_black_row);
        }
        let s = if pitch % 12 == 0 {
            Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color)
        } else {
            Stroke::new(1.0, v.faint_bg_color)
        };
        p.hline(row.x_range(), row.bottom(), s);
    }
    // Part-Bereich hervorheben, außerhalb abdunkeln.
    let (a, b) = (g.x(part_start as f64), g.x(part_end as f64));
    p.rect_filled(
        Rect::from_x_y_ranges(g.grid.left()..=a, g.grid.y_range()),
        0.0,
        crate::theme::tokens(p.ctx()).outside_part,
    );
    p.rect_filled(
        Rect::from_x_y_ranges(b..=g.grid.right(), g.grid.y_range()),
        0.0,
        crate::theme::tokens(p.ctx()).outside_part,
    );
    // Feines Snap-Raster, darüber Zählzeit- und Taktlinien.
    let beat = w.beat_ticks().max(1);
    let t0 = g.tick(g.grid.left()).max(0.0);
    let t1 = g.tick(g.grid.right()).max(0.0);
    let ppq = g.view.px_per_quarter;
    let beat_px = beat as f32 / TPQ as f32 * ppq;
    let beats = beat_px >= 4.0;
    for t in w.fine_grid_lines(t0, t1, ppq, beats) {
        p.vline(g.x(t), g.grid.y_range(), Stroke::new(1.0, v.faint_bg_color));
    }
    for (t, bar_no) in w.meter_lines(t0 as u64, t1 as u64, beats) {
        let col = if bar_no.is_some() {
            v.widgets.noninteractive.bg_stroke.color
        } else {
            v.widgets.noninteractive.bg_stroke.color.gamma_multiply(0.5)
        };
        p.vline(g.x(t as f64), g.grid.y_range(), Stroke::new(1.0, col));
    }
}

fn paint_ruler(w: &ProjectWindow, p: &Painter, g: &KGeo) {
    let v = p.ctx().global_style().visuals.clone();
    p.rect_filled(g.ruler, 0.0, v.faint_bg_color);
    let t0 = g.tick(g.ruler.left()).max(0.0) as u64;
    let t1 = g.tick(g.ruler.right()).max(0.0) as u64;
    for (t, bar_no) in w.meter_lines(t0, t1, false) {
        let x = g.x(t as f64);
        p.vline(
            x,
            g.ruler.y_range(),
            Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color),
        );
        if let Some(n) = bar_no {
            p.text(
                pos2(x + 3.0, g.ruler.center().y),
                Align2::LEFT_CENTER,
                n.to_string(),
                FontId::proportional(11.0),
                v.text_color(),
            );
        }
    }
}

fn paint_keys(p: &Painter, g: &KGeo, playing: Option<u8>) {
    // Erst eine weiße Tastenfläche, darauf die schwarzen Tasten – wie beim Klavier.
    p.rect_filled(g.keys, 0.0, crate::theme::tokens(p.ctx()).key_white);
    let top = g.pitch(g.keys.top()).min(127);
    let bottom = g.pitch(g.keys.bottom()).max(0);
    let line = Stroke::new(1.0, crate::theme::tokens(p.ctx()).key_line);
    for pitch in bottom..=top {
        let y = g.y(pitch);
        let row = Rect::from_min_max(
            pos2(g.keys.left(), y),
            pos2(g.keys.right(), y + g.view.key_h),
        );
        let lit = playing == Some(pitch as u8);
        if is_black(pitch) {
            let r = Rect::from_min_max(row.min, pos2(row.left() + KEYS_W * 0.6, row.bottom()));
            p.rect_filled(
                r,
                1.0,
                if lit {
                    crate::theme::tokens(p.ctx()).key_pressed
                } else {
                    crate::theme::tokens(p.ctx()).key_black
                },
            );
        } else {
            if lit {
                p.rect_filled(
                    row,
                    0.0,
                    crate::theme::tokens(p.ctx())
                        .key_pressed
                        .gamma_multiply(0.7),
                );
            }
            // Trennlinie zwischen benachbarten weißen Tasten (E–F, H–C).
            if matches!(pitch.rem_euclid(12), 4 | 11) {
                p.hline(row.x_range(), row.top(), line);
            }
        }
        if pitch % 12 == 0 {
            p.hline(row.x_range(), row.bottom(), line);
            if g.view.key_h >= 8.0 {
                p.text(
                    pos2(g.keys.right() - 3.0, y + g.view.key_h / 2.0),
                    Align2::RIGHT_CENTER,
                    note_name(pitch as u8),
                    FontId::proportional(9.0),
                    Color32::from_gray(40),
                );
            }
        }
    }
    p.vline(g.keys.right(), g.keys.y_range(), line);
}

/// `sel`: gewählte Notenindizes, aufsteigend sortiert.
fn paint_velocity(p: &Painter, g: &KGeo, notes: &[Note], sel: &[usize], color: Color32) {
    let v = p.ctx().global_style().visuals.clone();
    p.rect_filled(g.vel, 0.0, v.extreme_bg_color);
    p.hline(
        g.vel.x_range(),
        g.vel.top(),
        Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color),
    );
    p.text(
        g.vel.left_top() + vec2(4.0, 2.0),
        Align2::LEFT_TOP,
        tr("Velocity"),
        FontId::proportional(10.0),
        v.weak_text_color(),
    );
    let selected = crate::theme::tokens(p.ctx()).note_selected;
    for (i, n) in notes.iter().enumerate() {
        let x = g.x(g.part_start as f64 + n.start as f64);
        // Nur Balken im sichtbaren Bereich (Balken sind 4 px breit).
        if x + 4.0 < g.vel.left() || x > g.vel.right() {
            continue;
        }
        let h = g.vel.height() * f32::from(n.velocity) / 127.0;
        let r = Rect::from_min_max(pos2(x, g.vel.bottom() - h), pos2(x + 4.0, g.vel.bottom()));
        // `sel` ist sortiert.
        let c = if sel.binary_search(&i).is_ok() {
            selected
        } else {
            color
        };
        p.rect_filled(r, 0.0, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_with_middle_c_as_c3() {
        assert_eq!(note_name(60), "C3");
        assert_eq!(note_name(69), "A3");
        assert_eq!(note_name(0), "C-2");
        assert_eq!(note_name(61), "C#3");
    }
}
