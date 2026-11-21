//! Drum-Editor (Spec „Editoren“, Issue #38): hat die Spur eine Drum-Map, zeigt der Editor
//! statt der Klaviatur die Instrument-Zeilen der Map und die Noten als Rauten am Notenbeginn
//! (die Länge spielt keine Rolle).
//!
//! - Links je Zeile: Name, M und S (wirken auf die Wiedergabe der Spur) und das eigene Raster
//!   der Zeile („Raster“: das der Werkzeugzeile).
//! - Zeichnen: Klick setzt eine Note mit der Länge der Zeile, Klick auf eine Raute entfernt sie.
//! - Radiergummi: Klicken oder Ziehen löscht Rauten.
//! - Objektauswahl: Klick wählt (Umschalt erweitert), Ziehen verschiebt die Auswahl in der
//!   Zeit und auf andere Zeilen. Alt hebt das Raster auf. Entf löscht wie im Key-Editor.
//!
//! Tonhöhen im Part ohne Zeile in der Map erscheinen unten mit ihrer Nummer.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, UiBuilder, pos2, vec2};
use studiocraft_project::{DrumMap, EventId, Note, QuantizeValue};

use super::{KGeo, NoteDrag, RULER_H, paint_ruler};
use crate::i18n::tr;
use crate::{ProjectWindow, TPQ, Tool};

/// Breite der Zeilenköpfe links.
pub(super) const HEAD_W: f32 = 210.0;
/// Höhe einer Instrument-Zeile.
pub(crate) const ROW_H: f32 = 20.0;
/// Halbe Breite einer Raute.
const DIAMOND: f32 = 6.0;

/// Eine Zeile auf dem Bildschirm: Tonhöhe und Platz in der Map (`None`: nicht in der Map).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    pitch: u8,
    map: Option<usize>,
}

/// Zeilen der Map, darunter Tonhöhen des Parts ohne Zeile (aufsteigend).
fn rows(map: &DrumMap, notes: &[Note]) -> Vec<Row> {
    let mut out: Vec<Row> = map
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| Row {
            pitch: r.pitch,
            map: Some(i),
        })
        .collect();
    let mut extra: Vec<u8> = notes
        .iter()
        .map(|n| n.pitch)
        .filter(|p| map.row(*p).is_none())
        .collect();
    extra.sort_unstable();
    extra.dedup();
    out.extend(extra.into_iter().map(|pitch| Row { pitch, map: None }));
    out
}

/// Geometrie der Zeilen (die Zeitachse liefert [`KGeo`]).
struct Rows {
    grid: Rect,
    scroll_y: f32,
    rows: Vec<Row>,
}

impl Rows {
    fn top(&self, i: usize) -> f32 {
        self.grid.top() + i as f32 * ROW_H - self.scroll_y
    }
    fn center(&self, i: usize) -> f32 {
        self.top(i) + ROW_H * 0.5
    }
    fn index_at(&self, y: f32) -> Option<usize> {
        let f = ((y - self.grid.top() + self.scroll_y) / ROW_H).floor();
        (f >= 0.0)
            .then_some(f as usize)
            .filter(|&i| i < self.rows.len())
    }
    fn index_of(&self, pitch: u8) -> Option<usize> {
        self.rows.iter().position(|r| r.pitch == pitch)
    }
}

/// Raute unter `p`: Index der Note.
fn hit(g: &KGeo, rows: &Rows, notes: &[Note], p: Pos2) -> Option<usize> {
    let row = rows.index_at(p.y)?;
    let pitch = rows.rows[row].pitch;
    notes
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, n)| n.pitch == pitch)
        .find(|(_, n)| (g.x(g.part_start as f64 + n.start as f64) - p.x).abs() <= DIAMOND + 2.0)
        .map(|(i, _)| i)
}

impl ProjectWindow {
    /// Ändert die Drum-Map einer Spur (ein Rückgängig-Schritt; die Wiedergabe folgt).
    fn edit_drum_map(&mut self, track: usize, f: impl FnOnce(&mut DrumMap)) {
        let before = self.project.clone();
        if let Some(m) = self
            .project
            .tracks
            .get_mut(track)
            .and_then(|t| t.drum_map.as_mut())
        {
            f(m);
        }
        self.checkpoint(before);
    }

    /// Zeitposition (Ticks ab Projektanfang) auf das Raster einer Zeile: ihr eigenes oder das
    /// der Werkzeugzeile; mit `free` ohne Raster.
    fn drum_snap(&self, track: usize, pitch: u8, t: f64, free: bool) -> f64 {
        let q = self
            .project
            .tracks
            .get(track)
            .and_then(|t| t.drum_map.as_ref())
            .and_then(|m| m.row(pitch))
            .map_or(0, |r| r.quantize);
        if free || q == 0 {
            super::snap(self, t, free)
        } else {
            ((t / q as f64).round() * q as f64).max(0.0)
        }
    }

    /// Drum-Map der Spur im Inspector: keine (Key-Editor), General MIDI oder die Map einer
    /// anderen Spur (als Kopie); dazu „Bearbeiten …“ (Issue #161).
    pub(crate) fn drum_map_ui(&mut self, ui: &mut Ui, track: usize) {
        let current = self
            .project
            .tracks
            .get(track)
            .and_then(|t| t.drum_map.clone());
        // Maps anderer Spuren zum Übernehmen.
        let others: Vec<(String, DrumMap)> = self
            .project
            .tracks
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != track)
            .filter_map(|(_, t)| t.drum_map.clone().map(|m| (t.name.clone(), m)))
            .collect();
        let mut choice: Option<Option<DrumMap>> = None;
        ui.horizontal(|ui| {
            let l = ui.label(tr("Drum-Map"));
            let shown = match &current {
                None => tr("Keine").to_owned(),
                Some(m) if m.name == "GM" => tr("GM-Schlagzeug").to_owned(),
                Some(m) => m.name.clone(),
            };
            egui::ComboBox::from_id_salt(("drum-map", track))
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(current.is_none(), tr("Keine"))
                        .clicked()
                    {
                        choice = Some(None);
                    }
                    if ui.button(tr("GM-Schlagzeug")).clicked() {
                        choice = Some(Some(DrumMap::general_midi()));
                    }
                    for (name, map) in &others {
                        let label = crate::i18n::trf(
                            "Wie Spur „{spur}“ ({map})",
                            &[("spur", name), ("map", &map.name)],
                        );
                        if ui.button(label).clicked() {
                            choice = Some(Some(map.clone()));
                        }
                    }
                })
                .response
                .labelled_by(l.id);
            if current.is_some() && ui.button(tr("Bearbeiten …")).clicked() {
                self.open_drum_map_editor(track);
            }
        });
        if let Some(map) = choice.filter(|m| *m != current) {
            self.set_drum_map(track, map);
        }
    }
}

/// Zeichnet den Drum-Editor für `part` auf Spur `track` mit der Map `map`.
pub(super) fn show(w: &mut ProjectWindow, ui: &mut Ui, part: EventId, track: usize, map: DrumMap) {
    let Some(ev) = w.project.event(part).cloned() else {
        return;
    };
    let color = Color32::from_rgb(
        w.project.tracks[track].color[0],
        w.project.tracks[track].color[1],
        w.project.tracks[track].color[2],
    );
    ui.horizontal(|ui| {
        ui.strong(crate::i18n::trf(
            "Drum-Editor – {map}",
            &[("map", &map.name)],
        ));
        ui.toggle_value(&mut w.key_editor.feedback, tr("Akustisches Feedback"))
            .on_hover_text(tr("Angeklickte und verschobene Noten erklingen"));
        ui.toggle_value(&mut w.follow_playback, tr("Folgen (F)"))
            .on_hover_text(tr("Die Ansicht folgt dem Cursor während der Wiedergabe"));
        if !w.key_editor.selection.is_empty() {
            ui.separator();
            w.note_info(ui, part);
        }
    });
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let r = resp.rect;
    let head = Rect::from_min_max(
        pos2(r.left(), r.top() + RULER_H),
        pos2(r.left() + HEAD_W, r.bottom()),
    );
    let ruler = Rect::from_min_max(
        pos2(r.left() + HEAD_W, r.top()),
        pos2(r.right(), r.top() + RULER_H),
    );
    let grid = Rect::from_min_max(pos2(r.left() + HEAD_W, r.top() + RULER_H), r.max);

    // Folgen und Scrollen wie im Key-Editor; senkrecht in Zeilen.
    if w.follow_playback && w.is_playing() {
        let v = &mut w.key_editor.view;
        let visible = f64::from(grid.width() / v.px_per_quarter) * TPQ;
        let c = w.cursor as f64;
        if c > v.scroll_ticks + visible * 0.95 || c < v.scroll_ticks {
            v.scroll_ticks = (c - visible * 0.05).max(0.0);
        }
    }
    let all_rows = rows(&map, &ev.notes);
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
            w.key_editor.drum_scroll_y -= delta.y;
            v.scroll_ticks =
                (v.scroll_ticks - f64::from(delta.x / v.px_per_quarter) * TPQ).max(0.0);
        }
    }
    let max_scroll = (all_rows.len() as f32 * ROW_H - grid.height()).max(0.0);
    w.key_editor.drum_scroll_y = w.key_editor.drum_scroll_y.clamp(0.0, max_scroll);
    let g = KGeo {
        keys: head,
        ruler,
        grid,
        vel: Rect::NOTHING,
        view: w.key_editor.view,
        part_start: ev.start.0,
    };
    let rws = Rows {
        grid,
        scroll_y: w.key_editor.drum_scroll_y,
        rows: all_rows,
    };
    w.key_editor.grid_rect = grid;
    w.key_editor.vel_rect = Rect::NOTHING;

    interact(w, ui, &resp, &g, &rws, part, track, &ev.notes);

    // Zeichnen: Zeilen, Part-Bereich, Raster, Rauten.
    let notes: &[Note] = w.project.event(part).map_or(&[], |e| &e.notes);
    let mut selection = w.key_editor.selection.clone();
    selection.sort_unstable();
    let p = painter.with_clip_rect(grid);
    let v = ui.visuals().clone();
    p.rect_filled(grid, 0.0, v.extreme_bg_color);
    for (i, row) in rws.rows.iter().enumerate() {
        let top = rws.top(i);
        if top > grid.bottom() || top + ROW_H < grid.top() {
            continue;
        }
        let rect = Rect::from_min_size(pos2(grid.left(), top), vec2(grid.width(), ROW_H));
        if i % 2 == 1 {
            p.rect_filled(rect, 0.0, crate::theme::tokens(p.ctx()).key_black_row);
        }
        if !map.audible(row.pitch) {
            p.rect_filled(rect, 0.0, crate::theme::tokens(p.ctx()).outside_part);
        }
        p.hline(
            rect.x_range(),
            rect.bottom(),
            Stroke::new(1.0, v.faint_bg_color),
        );
    }
    let (a, b) = (g.x(ev.start.0 as f64), g.x(ev.end() as f64));
    for out in [
        Rect::from_x_y_ranges(grid.left()..=a, grid.y_range()),
        Rect::from_x_y_ranges(b..=grid.right(), grid.y_range()),
    ] {
        p.rect_filled(out, 0.0, crate::theme::tokens(p.ctx()).outside_part);
    }
    let t0 = g.tick(grid.left()).max(0.0);
    let t1 = g.tick(grid.right()).max(0.0);
    let ppq = g.view.px_per_quarter;
    let beats = w.beat_ticks().max(1) as f32 / TPQ as f32 * ppq >= 4.0;
    for t in w.fine_grid_lines(t0, t1, ppq, beats) {
        p.vline(g.x(t), grid.y_range(), Stroke::new(1.0, v.faint_bg_color));
    }
    for (t, bar_no) in w.meter_lines(t0 as u64, t1 as u64, beats) {
        let c = v.widgets.noninteractive.bg_stroke.color;
        let c = if bar_no.is_some() {
            c
        } else {
            c.gamma_multiply(0.5)
        };
        p.vline(g.x(t as f64), grid.y_range(), Stroke::new(1.0, c));
    }
    let selected_stroke = crate::theme::tokens(ui.ctx()).note_selected;
    for (i, n) in notes.iter().enumerate() {
        let Some(row) = rws.index_of(n.pitch) else {
            continue;
        };
        let c = pos2(g.x(ev.start.0 as f64 + n.start as f64), rws.center(row));
        let s = DIAMOND;
        // Nur sichtbare Rauten: ein Part kann zehntausende Noten haben.
        if !Rect::from_center_size(c, vec2(2.0 * s, 2.0 * s)).intersects(grid) {
            continue;
        }
        let pts = vec![
            pos2(c.x, c.y - s),
            pos2(c.x + s, c.y),
            pos2(c.x, c.y + s),
            pos2(c.x - s, c.y),
        ];
        let fill = color.lerp_to_gamma(Color32::WHITE, 0.1 + 0.4 * f32::from(n.velocity) / 127.0);
        let inside = n.start >= 0 && n.start < ev.length as i64;
        let fill = if inside && map.audible(n.pitch) {
            fill
        } else {
            fill.gamma_multiply(0.35)
        };
        let stroke = if selection.binary_search(&i).is_ok() {
            Stroke::new(2.0, selected_stroke)
        } else {
            Stroke::new(1.0, Color32::from_black_alpha(160))
        };
        p.add(egui::Shape::convex_polygon(pts, fill, stroke));
    }
    paint_ruler(w, &painter.with_clip_rect(ruler), &g);
    let cx = g.x(w.cursor as f64);
    painter.with_clip_rect(ruler.union(grid)).vline(
        cx,
        ruler.top()..=grid.bottom(),
        Stroke::new(1.5, crate::theme::tokens(ui.ctx()).playhead),
    );
    heads(w, ui, &painter, head, &rws, track, &map);
}

/// Änderung an einer Zeile der Map (aus den Zeilenköpfen).
type RowEdit = Box<dyn FnOnce(&mut studiocraft_project::DrumRow)>;

/// Zeilenköpfe: Name, M, S und Raster der Zeile.
fn heads(
    w: &mut ProjectWindow,
    ui: &mut Ui,
    painter: &egui::Painter,
    head: Rect,
    rws: &Rows,
    track: usize,
    map: &DrumMap,
) {
    let t = crate::theme::tokens(ui.ctx());
    painter.rect_filled(head, 0.0, ui.visuals().faint_bg_color);
    let mut change: Option<(usize, RowEdit)> = None;
    for (i, row) in rws.rows.iter().enumerate() {
        let top = rws.top(i);
        if top + ROW_H < head.top() || top > head.bottom() {
            continue;
        }
        let rect = Rect::from_min_size(pos2(head.left() + 4.0, top), vec2(HEAD_W - 8.0, ROW_H));
        if top < head.top() {
            continue; // halb unter dem Lineal: nur ganze Zeilen bedienen
        }
        let name = map.name_of(row.pitch);
        let Some(k) = row.map else {
            painter.with_clip_rect(head).text(
                pos2(rect.left(), rect.center().y),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(11.0),
                ui.visuals().weak_text_color(),
            );
            continue;
        };
        let r = &map.rows[k];
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let (mut m, mut s) = (r.mute, r.solo);
                let label = crate::i18n::trf("{name} stumm", &[("name", &r.name)]);
                let resp =
                    crate::icons::state_toggle(ui, &mut m, tr("M"), t.mute).on_hover_text(&label);
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::Button, true, m, label.clone())
                });
                if resp.clicked() {
                    change = Some((k, Box::new(move |r| r.mute = m)));
                }
                let label = crate::i18n::trf("{name} solo", &[("name", &r.name)]);
                let resp =
                    crate::icons::state_toggle(ui, &mut s, tr("S"), t.solo).on_hover_text(&label);
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::Button, true, s, label.clone())
                });
                if resp.clicked() {
                    change = Some((k, Box::new(move |r| r.solo = s)));
                }
                let q = r.quantize;
                let text = if q == 0 {
                    tr("Raster").to_owned()
                } else {
                    QuantizeValue::all()
                        .find(|v| v.ticks() == q)
                        .map_or_else(|| format!("{q}"), |v| v.short_label())
                };
                egui::ComboBox::from_id_salt(("drum-q", track, r.pitch))
                    .width(52.0)
                    .selected_text(egui::RichText::new(text).size(10.0))
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(q == 0, tr("Raster")).clicked() {
                            change = Some((k, Box::new(|r| r.quantize = 0)));
                        }
                        for v in QuantizeValue::all() {
                            let ticks = v.ticks();
                            if ui.selectable_label(q == ticks, v.short_label()).clicked() {
                                change = Some((k, Box::new(move |r| r.quantize = ticks)));
                            }
                        }
                    })
                    .response
                    .on_hover_text(crate::i18n::trf("Raster für {name}", &[("name", &r.name)]))
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::ComboBox,
                            true,
                            crate::i18n::trf("Raster für {name}", &[("name", &r.name)]),
                        )
                    });
                ui.add(egui::Label::new(egui::RichText::new(&r.name).size(11.0)).truncate());
            });
        });
    }
    if let Some((k, f)) = change {
        w.edit_drum_map(track, |m| {
            if let Some(r) = m.rows.get_mut(k) {
                f(r);
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn interact(
    w: &mut ProjectWindow,
    ui: &Ui,
    resp: &egui::Response,
    g: &KGeo,
    rws: &Rows,
    part: EventId,
    track: usize,
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
    let press = press.or_else(|| resp.interact_pointer_pos());
    let in_grid = |p: &Pos2| g.grid.contains(*p);
    if resp.clicked() || resp.drag_started() {
        w.key_editor.focused = true;
        w.selected_track = Some(track);
    }
    if released && w.key_editor.preview.is_some() && w.key_editor.drag.is_none() {
        w.preview(None);
    }
    let free = mods.alt;
    let part_start = g.part_start as f64;
    let length = |w: &ProjectWindow, pitch: u8| {
        w.project.tracks[track]
            .drum_map
            .as_ref()
            .and_then(|m| m.row(pitch))
            .map_or(studiocraft_project::drummap::DEFAULT_DRUM_LENGTH, |r| {
                r.length
            })
    };

    // Klick.
    if resp.clicked()
        && let Some(p) = press.filter(in_grid)
    {
        let Some(row) = rws.index_at(p.y) else {
            return;
        };
        let pitch = rws.rows[row].pitch;
        match (w.tool, hit(g, rws, notes, p)) {
            (Tool::Draw | Tool::Erase, Some(i)) => {
                w.edit_notes(part, |n| {
                    if i < n.len() {
                        n.remove(i);
                    }
                });
                w.key_editor.selection.clear();
            }
            (Tool::Draw, None) => {
                let t = w.drum_snap(track, pitch, g.tick(p.x), free) - part_start;
                if t >= 0.0 {
                    let len = length(w, pitch);
                    w.edit_notes(part, |n| {
                        n.push(Note {
                            pitch,
                            velocity: 100,
                            start: t as i64,
                            length: len,
                            spelling: 0,
                        });
                    });
                    w.key_editor.selection.clear();
                    w.note_feedback(pitch);
                }
            }
            (Tool::Select, Some(i)) => {
                let sel = &mut w.key_editor.selection;
                if mods.shift {
                    if let Some(k) = sel.iter().position(|&s| s == i) {
                        sel.remove(k);
                    } else {
                        sel.push(i);
                    }
                } else {
                    *sel = vec![i];
                }
                w.note_feedback(pitch);
            }
            (Tool::Select, None) => w.key_editor.selection.clear(),
            _ => {}
        }
        return;
    }

    // Ziehen: Auswahl verschieben (Objektauswahl) oder löschen (Radiergummi).
    if resp.drag_started()
        && let Some(p) = press.filter(in_grid)
    {
        w.drag_before = Some(w.project.clone());
        if w.tool == Tool::Select
            && let Some(i) = hit(g, rws, notes, p)
            && let Some(row) = rws.index_at(p.y)
        {
            if !w.key_editor.selection.contains(&i) {
                w.key_editor.selection = vec![i];
            }
            let tick = g.tick(p.x);
            w.key_editor.drag = Some(NoteDrag::Move {
                original: notes.to_vec(),
                anchor_tick: tick,
                anchor_pitch: row as i32,
                grab_offset: tick - (part_start + notes[i].start as f64),
            });
        }
    }
    if resp.dragged()
        && let Some(p) = now
    {
        match (&w.key_editor.drag, w.tool) {
            (
                Some(NoteDrag::Move {
                    original,
                    anchor_pitch,
                    grab_offset,
                    ..
                }),
                _,
            ) => {
                let (original, anchor_row, grab) = (original.clone(), *anchor_pitch, *grab_offset);
                let sel = w.key_editor.selection.clone();
                let lead = sel.first().and_then(|&i| original.get(i)).copied();
                let row_now = rws.index_at(p.y).map_or(anchor_row, |r| r as i32);
                if let Some(lead) = lead {
                    // Erste gewählte Note auf das Raster ihrer Zeile, die anderen gleich weit.
                    let target = g.tick(p.x) - grab;
                    let snapped = w.drum_snap(track, lead.pitch, target, free) - part_start;
                    let dt = snapped as i64 - lead.start;
                    let drow = row_now - anchor_row;
                    let rows_now: Vec<u8> = rws.rows.iter().map(|r| r.pitch).collect();
                    if let Some(ev) = w.project.event_mut(part) {
                        ev.notes = original.clone();
                        for &i in &sel {
                            if let Some(n) = ev.notes.get_mut(i) {
                                n.start = (n.start + dt).max(0);
                                let from = rows_now.iter().position(|&q| q == n.pitch);
                                if let Some(to) = from
                                    .and_then(|f| usize::try_from(f as i32 + drow).ok())
                                    .and_then(|t| rows_now.get(t))
                                {
                                    n.pitch = *to;
                                }
                            }
                        }
                    }
                    w.dirty = true;
                }
            }
            (None, Tool::Erase) => {
                if let Some(i) = hit(g, rws, notes, p)
                    && let Some(ev) = w.project.event_mut(part)
                    && i < ev.notes.len()
                {
                    ev.notes.remove(i);
                    w.key_editor.selection.clear();
                    w.dirty = true;
                }
            }
            _ => {}
        }
    }
    if resp.drag_stopped() {
        w.key_editor.drag = None;
        if let Some(b) = w.drag_before.take()
            && b != w.project
        {
            w.checkpoint(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_list_the_map_then_unmapped_pitches() {
        let map = DrumMap::general_midi();
        let note = |pitch| Note {
            pitch,
            velocity: 100,
            start: 0,
            length: 10,
            spelling: 0,
        };
        let r = rows(&map, &[note(36), note(10), note(100), note(10)]);
        assert_eq!(r.len(), 49);
        assert_eq!(r[0].pitch, 81);
        assert_eq!(
            r[47..].iter().map(|r| (r.pitch, r.map)).collect::<Vec<_>>(),
            vec![(10, None), (100, None)]
        );
    }
}
