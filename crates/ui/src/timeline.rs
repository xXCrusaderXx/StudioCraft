//! Spurliste, Lineal und Event-Anzeige: selbst gezeichnet, damit auch
//! hunderte Spuren flüssig bleiben (es wird nur gezeichnet, was sichtbar ist).

use egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use studiocraft_project::{
    EVENT_MAX_DB, Event, EventId, FadeCurve, MIN_DB, TimeBase, Track, TrackKind,
};

use crate::i18n::tr;
use crate::icons::Icon;
use crate::{ProjectWindow, TPQ, Tool};

mod automation;
mod tempo;
pub(crate) use automation::range as param_range;

const HEADER_W: f32 = 290.0;
/// Anzahl der Schalter im Spurkopf.
const HEADER_BUTTONS: usize = 8;
/// Breite der Farbleiste links im Spurkopf; sie trägt die Spurnummer.
const STRIP_W: f32 = 20.0;
/// Linker Rand der Schalter im Spurkopf (hinter Farbleiste und Automationspfeil).
const BUTTONS_X: f32 = 38.0;
/// Breite des Lautstärkereglers oben rechts im Spurkopf.
const VOLUME_W: f32 = 64.0;
const RULER_H: f32 = 30.0;

#[derive(Debug, Clone, Copy)]
pub(crate) struct View {
    /// Pixel pro Viertelnote (horizontaler Zoom).
    pub px_per_quarter: f32,
    /// Linker Rand der Event-Anzeige in Ticks.
    pub scroll_ticks: f64,
    /// Vertikaler Versatz in Pixel.
    pub scroll_y: f32,
    pub row_h: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            px_per_quarter: 24.0,
            scroll_ticks: 0.0,
            scroll_y: 0.0,
            row_h: 52.0,
        }
    }
}

impl View {
    /// Zoomt horizontal; `anchor_x` (Pixel ab linkem Rand der Event-Anzeige) bleibt stehen.
    pub fn zoom(&mut self, factor: f32, anchor_x: Option<f32>) {
        let x = anchor_x.unwrap_or(0.0);
        let anchor_ticks = self.scroll_ticks + f64::from(x / self.px_per_quarter) * TPQ;
        self.px_per_quarter = (self.px_per_quarter * factor).clamp(1.0, 2000.0);
        self.scroll_ticks = (anchor_ticks - f64::from(x / self.px_per_quarter) * TPQ).max(0.0);
    }
}

/// Zustand des Wiedergabe-Werkzeugs, solange die Maustaste gedrückt ist.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Audition {
    /// Cursor vor dem Drücken; ohne laufende Wiedergabe kehrt er beim Loslassen dorthin zurück.
    cursor_before: u64,
    /// Lief die Wiedergabe schon? Dann läuft sie nach dem Loslassen weiter.
    was_playing: bool,
    /// Mausposition des letzten Sprungs; erst nach 2 px Bewegung wird neu positioniert.
    last_x: f32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Drag {
    /// Punkt `idx` der Automationsspur `lane` (Index in `Track::automation`) ziehen.
    AutoPoint {
        row: usize,
        lane: usize,
        idx: usize,
    },
    /// Comping: Abschnitt eines Takes ab `start` aufziehen.
    Comp {
        row: usize,
        id: EventId,
        start: u64,
    },
    /// Marker `index` (in `Project::markers`) ziehen; `grab` = Abstand Maus zu Marker in Ticks.
    Marker {
        index: usize,
        grab: f64,
    },
    /// Punkt der Tempospur ziehen (`index` `None`: Anfangstempo); `range` = BPM-Bereich der
    /// Anzeige beim Greifen (bleibt beim Ziehen fest).
    TempoPoint {
        row: usize,
        index: Option<usize>,
        range: (f64, f64),
    },
    /// Automation frei zeichnen; `last` = zuletzt gezeichneter Tick.
    /// Bereichsauswahl auf einer Automationsspur ziehen: Werte in `from..=to` skalieren;
    /// `y0` = Mausposition beim Greifen (Ausgangszustand: `drag_before`).
    AutoScale {
        row: usize,
        lane: usize,
        y0: f32,
        from: u64,
        to: u64,
    },
    AutoDraw {
        row: usize,
        lane: usize,
        last: u64,
    },
    /// Zeichnen mit einer Form (Linie, Parabel, Sinus, Rechteck) vom Anfangspunkt bis zum
    /// Zeiger; die Kurve davor steht in `drag_before`.
    AutoShape {
        row: usize,
        lane: usize,
        tick: u64,
        value: f64,
    },
    /// Auswahl verschieben; `grab` = Abstand Mauszeiger zu Event-Start in Ticks. Strg beim
    /// Anfassen kopiert, Strg+Umschalt macht geteilte Kopien; Strg erst während des Ziehens
    /// sperrt die Richtung.
    Move {
        id: EventId,
        grab: f64,
        row: usize,
        copy: bool,
        shared: bool,
    },
    /// Alt+Ziehen am rechten Rand: die Auswahl so oft wiederholen, wie gezogen wird.
    Repeat {
        start: u64,
        end: u64,
    },
    /// Neues Event aufziehen.
    Draw {
        row: usize,
        start: u64,
    },
    /// Spur am Spurkopf ziehen: umsortieren, in einen Ordner hinein oder heraus.
    TrackMove {
        row: usize,
    },
    /// Ordnerzeile ziehen: alle Events der enthaltenen Spuren zeitlich verschieben;
    /// `start` ist die Greifposition, `applied` die schon angewandte Verschiebung.
    FolderShift {
        row: usize,
        start: u64,
        applied: i64,
    },
    /// Lautstärkeregler im Spurkopf ziehen.
    Volume {
        row: usize,
    },
    /// Cycle-Bereich im Lineal aufziehen (obere Hälfte).
    Ruler {
        start: u64,
    },
    /// Cursor im Lineal ziehen (untere Hälfte).
    Scrub,
    /// Löschen bzw. Stummschalten über Events streichen; `mute_to` ist der Zustand, den das
    /// erste getroffene Event bekommen hat (alle weiteren folgen ihm); `last` die letzte
    /// Mausposition, damit schnelle Bewegungen keine Events überspringen.
    Stroke {
        erase: bool,
        mute_to: Option<bool>,
        last: Pos2,
    },
    /// Zoom-Werkzeug: Rechteck aufziehen, beim Loslassen auf diesen Zeitbereich zoomen.
    ZoomRect {
        start: Pos2,
    },
    /// Bereichsauswahl aufziehen (Werkzeug 2).
    Range {
        row: usize,
        start: u64,
    },
    /// Länge am linken bzw. rechten Rand ändern (untere Hälfte des Rands).
    TrimStart {
        id: EventId,
    },
    TrimEnd {
        id: EventId,
    },
    /// Fade-Griffe (obere Hälfte des Rands bzw. am Griff).
    FadeIn {
        id: EventId,
    },
    FadeOut {
        id: EventId,
    },
    /// Lautstärkegriff oben in der Mitte eines Audio-Events: senkrecht ziehen.
    EventGain {
        id: EventId,
        /// Mauszeiger-Höhe und Lautstärke beim Greifen.
        y0: f32,
        db0: f32,
    },
}

/// dB je Pixel beim Ziehen am Lautstärkegriff (mit Umschalt ein Viertel davon).
const GAIN_DB_PER_PX: f32 = 0.25;

/// Fangbereich um Ränder und Fade-Griffe in Pixel.
const EDGE: f32 = 7.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    Body,
    TrimStart,
    TrimEnd,
    FadeIn,
    FadeOut,
    /// Lautstärkegriff oben in der Mitte (Audio-Events).
    Gain,
}

/// Smart-Bereiche eines Events unter dem Mauszeiger (Spec „Werkzeuge“):
/// obere Hälfte am Rand bzw. am Griff = Fade, untere Hälfte am Rand = Länge.
fn smart_zone(w: &ProjectWindow, g: &Geo, p: Pos2) -> Option<(EventId, usize, Zone)> {
    if !g.lanes.contains(p) {
        return None;
    }
    let row = g.row_at(p.y)?;
    let px = |dx: f32| f64::from(dx / g.view.px_per_quarter) * TPQ;
    // Etwas Toleranz, damit man den Rand auch knapp außerhalb trifft.
    let t = g.ticks(p.x);
    let id = hit_event(w, g, row, p.y, t.max(0.0) as u64)
        .or_else(|| hit_event(w, g, row, p.y, (t - px(EDGE)).max(0.0) as u64))
        .or_else(|| hit_event(w, g, row, p.y, (t + px(EDGE)).max(0.0) as u64))?;
    let ev = w.project.event(id)?;
    let lane_top = g.row_top(row) + lane_of(&w.project.tracks[row], id) as f32 * g.view.row_h;
    let x0 = g.x(ev.start.0 as f64);
    let x1 = g.x(ev.end() as f64);
    let upper = p.y < lane_top + g.view.row_h * 0.5;
    let fin = g.x((ev.start.0 + ev.fade_in) as f64);
    let fout = g.x((ev.end() - ev.fade_out) as f64);
    let mid = (x0 + x1) / 2.0;
    let gain_handle = (ev.clip.is_some() || ev.is_audio_part())
        && p.y < lane_top + EDGE * 2.0
        && (p.x - mid).abs() <= EDGE
        && x1 - x0 > EDGE * 6.0;
    let zone = if gain_handle {
        Zone::Gain
    } else if upper && (p.x - fin).abs() <= EDGE && (p.x - fin).abs() <= (p.x - fout).abs() {
        Zone::FadeIn
    } else if upper && (p.x - fout).abs() <= EDGE {
        Zone::FadeOut
    } else if !upper && (p.x - x0).abs() <= EDGE {
        Zone::TrimStart
    } else if !upper && (x1 - p.x).abs() <= EDGE {
        Zone::TrimEnd
    } else if p.x >= x0 && p.x <= x1 {
        Zone::Body
    } else {
        return None;
    };
    Some((id, row, zone))
}

struct Geo {
    header: Rect,
    ruler: Rect,
    lanes: Rect,
    view: View,
    /// Anzahl der angezeigten Zeilen (Spuren in zugeklappten Ordnern fehlen).
    rows: usize,
    /// Oberkante jeder Anzeigezeile ab Inhaltsanfang, dazu die Gesamthöhe (Länge `rows + 1`).
    /// Spuren mit aufgeklappten Lanes sind so hoch wie alle ihre Lanes.
    tops: Vec<f32>,
    /// Spur je Anzeigezeile, in Baumreihenfolge (Ordner, darunter ihre Spuren).
    order: Vec<usize>,
    /// Anzeigezeile je Spur; `None` in zugeklappten Ordnern.
    pos: Vec<Option<usize>>,
    /// Einrückung je Spur (Ordnertiefe).
    depth: Vec<usize>,
}

/// Anzeigereihenfolge, Zeilen und Einrückung der Spuren (siehe [`Geo`]).
fn layout(w: &ProjectWindow) -> (Vec<usize>, Vec<Option<usize>>, Vec<usize>, Vec<f32>) {
    let n = w.project.tracks.len();
    let tree = w.project.display_order();
    let mut pos = vec![None; n];
    let mut depth = vec![0; n];
    let mut order = Vec::with_capacity(tree.len());
    let mut tops = Vec::with_capacity(tree.len() + 1);
    let mut y = 0.0;
    tops.push(y);
    for (i, (t, d)) in tree.into_iter().enumerate() {
        order.push(t);
        pos[t] = Some(i);
        depth[t] = d;
        let lanes = automation::rows_of(&w.project.tracks[t]);
        y += w.view.row_h * lanes as f32;
        tops.push(y);
    }
    (order, pos, depth, tops)
}

/// Einrückung des Spurkopfs nach Ordnertiefe.
fn indent(g: &Geo, row: usize) -> f32 {
    g.depth.get(row).copied().unwrap_or(0) as f32 * 14.0
}

/// Ordnerzeile: die Events aller enthaltenen Spuren als schmale Balken (Überblick, auch
/// zugeklappt).
fn paint_folder_summary(w: &ProjectWindow, painter: &Painter, g: &Geo, row: usize) {
    let top = g.row_top(row);
    let band = top + g.view.row_h * 0.3..=top + g.view.row_h * 0.7;
    let t0 = g.ticks(g.lanes.left()).max(0.0) as u64;
    let t1 = g.ticks(g.lanes.right()).max(0.0) as u64;
    for (i, t) in w.project.tracks.iter().enumerate() {
        if !w.project.is_inside(i, row) {
            continue;
        }
        let color = track_color(t.color).gamma_multiply(0.7);
        for e in t.events.iter().filter(|e| e.end() >= t0 && e.start.0 <= t1) {
            let r = Rect::from_x_y_ranges(
                g.x(e.start.0 as f64)..=g.x(e.end() as f64).max(g.x(e.start.0 as f64) + 1.0),
                band.clone(),
            );
            painter.rect_filled(r, 2.0, color);
        }
    }
}

/// Marker unter `p` in der Marker-Spur: Positionsmarker bis 6 px neben der Linie oder auf
/// ihrem Fähnchen, Cycle-Marker in ihrem Bereich. Positionsmarker haben Vorrang.
fn marker_at(w: &ProjectWindow, g: &Geo, p: Pos2) -> Option<usize> {
    let ms = &w.project.markers;
    ms.iter()
        .position(|m| {
            let x = g.x(m.start as f64);
            !m.is_cycle() && (p.x - x).abs() <= 6.0
                || (!m.is_cycle() && p.x >= x && p.x <= x + 16.0)
        })
        .or_else(|| {
            ms.iter().position(|m| {
                m.end
                    .is_some_and(|e| p.x >= g.x(m.start as f64) && p.x <= g.x(e as f64))
            })
        })
}

/// Marker-Spur: Cycle-Marker als Balken, Positionsmarker als Linie mit Fähnchen und Name.
/// Abstände der Beschriftung einer Lineal-Spur in Sekunden (von fein bis grob).
const RULER_STEPS: [f64; 17] = [
    0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0,
    600.0,
];
/// Mindestabstand zweier Beschriftungen einer Lineal-Spur in Pixeln.
const RULER_LABEL_PX: f32 = 90.0;

/// Lineal-Spur (Issue #44): Striche und Beschriftung im Format der Spur – Takte, Sekunden,
/// Timecode oder Samples. Zeitformate folgen der Tempo-Map (Sekunden sind gleichmäßig, die
/// Striche liegen also bei Tempowechseln ungleich weit auseinander).
fn paint_ruler_track(w: &ProjectWindow, painter: &Painter, g: &Geo, row: usize) {
    use studiocraft_project::RulerFormat;
    let t = crate::theme::tokens(painter.ctx());
    let top = g.row_top(row);
    let h = g.row_height(row);
    let format = w.project.tracks[row].ruler;
    let tempo = &w.project.tempo;
    let font = egui::FontId::proportional(10.0);
    let t0 = g.ticks(g.lanes.left()).max(0.0);
    let t1 = g.ticks(g.lanes.right()).max(t0 + 1.0);
    let mark = |ticks: f64, label: String| {
        let x = g.x(ticks);
        if !g.lanes.x_range().contains(x) {
            return;
        }
        painter.vline(x, top + h * 0.5..=top + h, Stroke::new(1.0, t.text_dim));
        painter.text(
            pos2(x + 3.0, top + h * 0.3),
            egui::Align2::LEFT_CENTER,
            label,
            font.clone(),
            t.text,
        );
    };
    if format == RulerFormat::Bars {
        let bars = tempo.bar_lines(t0 as u64, t1 as u64);
        // Jeden n-ten Takt beschriften, damit die Zahlen sich nicht überlappen.
        let bar_px = bars
            .get(1)
            .zip(bars.first())
            .map_or(RULER_LABEL_PX, |(b, a)| g.x(b.0 as f64) - g.x(a.0 as f64));
        let every = (RULER_LABEL_PX / bar_px.max(1.0)).ceil().max(1.0) as u64;
        for (tick, bar, _) in bars {
            if (bar - 1) % every == 0 {
                mark(tick as f64, bar.to_string());
            }
        }
        return;
    }
    let s0 = tempo.seconds(studiocraft_project::Position(t0 as u64));
    let s1 = tempo.seconds(studiocraft_project::Position(t1 as u64));
    let px_per_s = (g.lanes.width() / ((s1 - s0).max(1e-6)) as f32).max(1e-3);
    let frame = format.fps().map_or(0.0, |f| 1.0 / f64::from(f));
    let step = RULER_STEPS
        .iter()
        .copied()
        .filter(|s| *s >= frame)
        .find(|s| *s as f32 * px_per_s >= RULER_LABEL_PX)
        .unwrap_or(600.0);
    let mut k = (s0 / step).floor().max(0.0) as u64;
    let mut n = 0;
    while n < 1000 {
        let s = k as f64 * step;
        if s > s1 {
            break;
        }
        let label = format.format_seconds(s, w.sample_rate);
        mark(tempo.ticks_at_seconds(s), label);
        k += 1;
        n += 1;
    }
}

fn paint_markers(w: &ProjectWindow, painter: &Painter, g: &Geo, row: usize) {
    let t = crate::theme::tokens(painter.ctx());
    let top = g.row_top(row);
    let h = g.row_height(row);
    let color = track_color(w.project.tracks[row].color);
    for m in w.project.markers.iter().filter(|m| m.is_cycle()) {
        let Some(end) = m.end else { continue };
        let r = Rect::from_x_y_ranges(
            g.x(m.start as f64)..=g.x(end as f64),
            top + h * 0.55..=top + h - 4.0,
        );
        painter.rect_filled(r, 2.0, color.gamma_multiply(0.45));
        painter.rect_stroke(r, 2.0, Stroke::new(1.0, color), StrokeKind::Inside);
        painter.with_clip_rect(r).text(
            r.left_center() + vec2(4.0, 0.0),
            Align2::LEFT_CENTER,
            format!("C{} {}", m.id, m.name),
            FontId::proportional(11.0),
            t.text,
        );
    }
    for m in w.project.markers.iter().filter(|m| !m.is_cycle()) {
        let x = g.x(m.start as f64);
        painter.vline(x, top..=top + h, Stroke::new(1.5, color));
        let flag = Rect::from_min_size(pos2(x, top + 3.0), vec2(16.0, 14.0));
        painter.rect_filled(flag, 2.0, color);
        painter.text(
            flag.center(),
            Align2::CENTER_CENTER,
            m.id.to_string(),
            FontId::proportional(10.0),
            Color32::BLACK,
        );
        painter.text(
            pos2(flag.right() + 3.0, flag.center().y),
            Align2::LEFT_CENTER,
            &m.name,
            FontId::proportional(11.0),
            t.text,
        );
    }
}

/// Anfang und Ende der Auswahl in Ticks.
fn selection_span(w: &ProjectWindow) -> Option<(u64, u64)> {
    let evs: Vec<_> = w
        .selection
        .iter()
        .filter_map(|&id| w.project.event(id))
        .collect();
    let a = evs.iter().map(|e| e.start.0).min()?;
    let b = evs.iter().map(|e| e.end()).max()?;
    (b > a).then_some((a, b))
}

/// Wie oft Alt+Ziehen am rechten Rand wiederholt: so viele Auswahl-Längen, wie die Maus hinter
/// dem Ende steht (gerundet).
fn repeat_count(g: &Geo, start: u64, end: u64, p: Pos2) -> u32 {
    let span = (end - start).max(1) as f64;
    let n = ((g.ticks(p.x) - end as f64) / span).round();
    if n.is_finite() && n > 0.0 {
        n.min(999.0) as u32
    } else {
        0
    }
}

/// Richtungssperre: nur die größere Bewegung (waagerecht in Pixeln gegen senkrecht) bleibt.
fn lock_direction(g: &Geo, drow: isize, dt: i64) -> (isize, i64) {
    let px_x = (dt as f64 / TPQ).abs() as f32 * g.view.px_per_quarter;
    let px_y = drow.unsigned_abs() as f32 * g.view.row_h;
    if px_x >= px_y { (0, dt) } else { (drow, 0) }
}

/// Lane eines Events in seiner Spur (0 ohne aufgeklappte Lanes).
fn lane_of(track: &Track, id: EventId) -> usize {
    if !track.show_lanes {
        return 0;
    }
    let (lanes, _) = track.lanes();
    track
        .events
        .iter()
        .position(|e| e.id == id)
        .and_then(|i| lanes.get(i).copied())
        .unwrap_or(0)
}

/// Das Event unter (`y`, `ticks`) in Spur `row`; bei aufgeklappten Lanes nur in der Lane
/// unter `y`, sonst das oberste (klingende).
fn hit_event(w: &ProjectWindow, g: &Geo, row: usize, y: f32, ticks: u64) -> Option<EventId> {
    let track = w.project.tracks.get(row)?;
    // Eingefroren: Events gesperrt.
    if track.frozen.is_some() {
        return None;
    }
    let lane = ((y - g.row_top(row)) / g.view.row_h).floor().max(0.0) as usize;
    // Darunter liegen die Automationsspuren.
    if lane >= automation::take_lanes(track) {
        return None;
    }
    if !track.show_lanes {
        return w.project.event_at(row, ticks);
    }
    let (lanes, _) = track.lanes();
    track
        .events
        .iter()
        .zip(lanes)
        .rev()
        .find(|(e, l)| *l == lane && e.contains(ticks))
        .map(|(e, _)| e.id)
}

impl Geo {
    fn x(&self, ticks: f64) -> f32 {
        self.lanes.left()
            + ((ticks - self.view.scroll_ticks) / TPQ) as f32 * self.view.px_per_quarter
    }
    fn ticks(&self, x: f32) -> f64 {
        self.view.scroll_ticks + f64::from((x - self.lanes.left()) / self.view.px_per_quarter) * TPQ
    }
    /// Oberkante der Spur `row` (Spur-Index). Ausgeblendete Spuren liegen weit außerhalb.
    fn row_top(&self, row: usize) -> f32 {
        match self.pos.get(row).copied().flatten() {
            Some(d) => self.lanes.top() + self.tops[d] - self.view.scroll_y,
            None => -1.0e6,
        }
    }
    fn row_height(&self, row: usize) -> f32 {
        match self.pos.get(row).copied().flatten() {
            Some(d) => self.tops[d + 1] - self.tops[d],
            None => 0.0,
        }
    }
    /// Unterkante der letzten Zeile (dort entstehen neue Spuren).
    fn content_bottom(&self) -> f32 {
        self.lanes.top() + self.tops.last().copied().unwrap_or(0.0) - self.view.scroll_y
    }
    /// Anzeigezeile, deren Oberkante als letzte bei oder über `content_y` liegt.
    fn row_index(&self, content_y: f32) -> usize {
        self.tops
            .partition_point(|&t| t <= content_y)
            .saturating_sub(1)
    }
    /// Spur unter `y`.
    fn row_at(&self, y: f32) -> Option<usize> {
        let c = y - self.lanes.top() + self.view.scroll_y;
        let total = self.tops.last().copied().unwrap_or(0.0);
        (c >= 0.0 && c < total).then(|| self.order.get(self.row_index(c)).copied())?
    }
    /// Sichtbare Spuren in Anzeigereihenfolge.
    fn visible_rows(&self) -> Vec<usize> {
        let first = self.row_index(self.view.scroll_y.max(0.0));
        let last = self.row_index(self.view.scroll_y + self.lanes.height()) + 1;
        self.order[first.min(self.rows)..last.min(self.rows)].to_vec()
    }
    /// Anzeigezeile der Spur `row`.
    fn display_pos(&self, row: usize) -> Option<usize> {
        self.pos.get(row).copied().flatten()
    }
    /// Spur `delta` Anzeigezeilen von `row` entfernt (am Rand begrenzt).
    fn offset_row(&self, row: usize, delta: isize) -> usize {
        let d = self.display_pos(row).unwrap_or(0) as isize + delta;
        let d = d.clamp(0, self.rows.saturating_sub(1) as isize) as usize;
        self.order.get(d).copied().unwrap_or(row)
    }
    /// Oberkante der oberen bzw. unteren Zeile im Spurkopf.
    fn header_line(&self, row: usize, line: usize) -> f32 {
        let mid = self.row_top(row) + self.view.row_h / 2.0;
        if line == 0 { mid - 19.0 } else { mid + 1.0 }
    }
    /// Schalter im Spurkopf (Spec „Spurkopf in der Spurliste“), in zwei Zeilen:
    /// oben Mute und Solo vor dem Namen, unten Aufnahme bereit, Monitor,
    /// Automation lesen, Automation schreiben, Zeitbasis, Lanes.
    fn header_buttons(&self, row: usize) -> [Rect; HEADER_BUTTONS] {
        std::array::from_fn(|i| {
            let (line, col) = if i < 2 { (0, i) } else { (1, i - 2) };
            let x = self.header.left() + BUTTONS_X + indent(self, row) + col as f32 * 21.0;
            Rect::from_min_size(pos2(x, self.header_line(row, line)), vec2(19.0, 18.0))
        })
    }
    /// Lautstärkeregler oben rechts im Spurkopf (nur Spuren mit Kanal).
    fn header_volume(&self, row: usize) -> Rect {
        Rect::from_min_size(
            pos2(
                self.header.right() - 6.0 - VOLUME_W,
                self.header_line(row, 0),
            ),
            vec2(VOLUME_W, 18.0),
        )
    }
}

pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    // Thumbnails der Videospuren nachladen (im Hintergrund erzeugt).
    let videos = w.video_paths();
    if !videos.is_empty() {
        w.videos.poll(ui.ctx(), &videos);
    }
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let r = resp.rect;
    let header = Rect::from_min_max(
        pos2(r.left(), r.top() + RULER_H),
        pos2(r.left() + HEADER_W, r.bottom()),
    );
    let ruler = Rect::from_min_max(
        pos2(r.left() + HEADER_W, r.top()),
        pos2(r.right(), r.top() + RULER_H),
    );
    let lanes = Rect::from_min_max(pos2(r.left() + HEADER_W, r.top() + RULER_H), r.max);
    w.lanes_rect = lanes;

    let (order, pos, depth, tops) = layout(w);
    let rows = order.len();
    scroll_and_zoom(w, ui, &resp, lanes, tops.last().copied().unwrap_or(0.0));
    follow_cursor(w, lanes);
    let geo = Geo {
        header,
        ruler,
        lanes,
        view: w.view,
        rows,
        tops,
        order,
        pos,
        depth,
    };

    interact(w, ui, &resp, &geo);
    let drop_target = file_drop(w, ui, &geo);
    let plugin_target = plugin_drop(w, ui, &geo);

    let geo = Geo {
        view: w.view,
        ..geo
    };
    paint_lanes(w, &painter.with_clip_rect(lanes), &geo);
    paint_ruler(w, &painter.with_clip_rect(ruler), &geo);
    paint_headers(w, &painter.with_clip_rect(header), &geo);
    let corner = Rect::from_min_max(r.min, pos2(header.right(), ruler.bottom()));
    painter.rect_filled(corner, 0.0, ui.visuals().faint_bg_color);
    painter.text(
        corner.left_center() + vec2(8.0, 0.0),
        Align2::LEFT_CENTER,
        crate::i18n::trf("{rows} Spuren", &[("rows", &rows)]),
        FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );
    paint_cursor(w, &painter.with_clip_rect(ruler.union(lanes)), &geo);
    if let Some((row, ticks)) = drop_target {
        paint_drop_target(&painter.with_clip_rect(lanes), &geo, row, ticks);
    }
    if let Some(t) = plugin_target {
        paint_plugin_target(&painter.with_clip_rect(header.union(lanes)), &geo, t);
    }
    let content_h = geo.tops.last().copied().unwrap_or(0.0);
    scrollbars(w, ui, &painter, lanes, content_h);
}

/// Dicke der Bildlaufleisten in Punkten.
const BAR: f32 = 10.0;

/// Ausdehnung des Projekts in Ticks für die Leiste: bis hinter das letzte Event bzw. den
/// sichtbaren Teil, mit einer halben Fensterbreite Luft.
fn timeline_extent(w: &ProjectWindow, visible: f64) -> f64 {
    (w.project.length() as f64).max(w.view.scroll_ticks + visible) + visible * 0.5
}

/// Leiste mit Griff: (Griff, Position des Griffs als Anteil, Größe als Anteil).
fn thumb(track: Rect, horizontal: bool, start: f32, size: f32) -> Rect {
    let len = if horizontal {
        track.width()
    } else {
        track.height()
    };
    let size = (size.clamp(0.0, 1.0) * len).clamp(24.0_f32.min(len), len);
    let pos = (start.clamp(0.0, 1.0) * len).min(len - size);
    if horizontal {
        Rect::from_min_size(
            pos2(track.left() + pos, track.top()),
            vec2(size, track.height()),
        )
    } else {
        Rect::from_min_size(
            pos2(track.left(), track.top() + pos),
            vec2(track.width(), size),
        )
    }
}

/// Bildlaufleisten unten (Zeit) und rechts (Spuren) über dem Rand der Event-Anzeige: Griff
/// ziehen verschiebt, Klick daneben blättert um eine Fensterbreite bzw. -höhe.
fn scrollbars(w: &mut ProjectWindow, ui: &Ui, painter: &Painter, lanes: Rect, content_h: f32) {
    let t = crate::theme::tokens(ui.ctx());
    let draw = |track: Rect, grip: Rect, active: bool| {
        painter.rect_filled(track, 0.0, t.panel_bg2.gamma_multiply(0.85));
        let c = if active { t.accent } else { t.button_hi };
        painter.rect_filled(grip.shrink(2.0), 3.0, c);
    };
    // Waagerecht: Zeit.
    let visible = f64::from(lanes.width() / w.view.px_per_quarter.max(0.01)) * TPQ;
    let total = timeline_extent(w, visible).max(1.0);
    let htrack = Rect::from_min_max(
        pos2(lanes.left(), lanes.bottom() - BAR),
        pos2(lanes.right() - BAR, lanes.bottom()),
    );
    let hgrip = thumb(
        htrack,
        true,
        (w.view.scroll_ticks / total) as f32,
        (visible / total) as f32,
    );
    let hresp = ui
        .interact(
            htrack,
            ui.scope_id().with("bildlauf-zeit"),
            Sense::click_and_drag(),
        )
        .on_hover_text(tr("Zeit: ziehen oder daneben klicken"));
    hresp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::ScrollBar, true, tr("Bildlaufleiste Zeit"))
    });
    if hresp.dragged() {
        let per_px = total / f64::from(htrack.width().max(1.0));
        w.view.scroll_ticks += f64::from(hresp.drag_delta().x) * per_px;
    } else if hresp.clicked()
        && let Some(p) = hresp.interact_pointer_pos()
        && !hgrip.contains(p)
    {
        let page = if p.x < hgrip.left() {
            -visible
        } else {
            visible
        };
        w.view.scroll_ticks += page * 0.9;
    }
    w.view.scroll_ticks = w
        .view
        .scroll_ticks
        .clamp(0.0, (total - visible * 0.5).max(0.0));
    draw(htrack, hgrip, hresp.dragged() || hresp.hovered());
    // Senkrecht: Spuren (nur, wenn sie nicht alle hineinpassen).
    let view_h = lanes.height() - BAR;
    let total_h = content_h + w.view.row_h;
    if total_h > view_h {
        let vtrack = Rect::from_min_max(
            pos2(lanes.right() - BAR, lanes.top()),
            pos2(lanes.right(), lanes.bottom() - BAR),
        );
        let vgrip = thumb(vtrack, false, w.view.scroll_y / total_h, view_h / total_h);
        let vresp = ui
            .interact(
                vtrack,
                ui.scope_id().with("bildlauf-spuren"),
                Sense::click_and_drag(),
            )
            .on_hover_text(tr("Spuren: ziehen oder daneben klicken"));
        vresp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ScrollBar, true, tr("Bildlaufleiste Spuren"))
        });
        if vresp.dragged() {
            w.view.scroll_y += vresp.drag_delta().y * total_h / vtrack.height().max(1.0);
        } else if vresp.clicked()
            && let Some(p) = vresp.interact_pointer_pos()
            && !vgrip.contains(p)
        {
            let page = if p.y < vgrip.top() { -view_h } else { view_h };
            w.view.scroll_y += page * 0.9;
        }
        w.view.scroll_y = w.view.scroll_y.clamp(0.0, (total_h - view_h).max(0.0));
        draw(vtrack, vgrip, vresp.dragged() || vresp.hovered());
    }
}

/// Wohin ein gezogenes Plugin fällt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginTarget {
    /// Instrument auf leere Fläche (oder Spur ohne Instrument): neue Instrument-Spur.
    NewTrack,
    /// Instrument auf eine Instrument-Spur: ersetzt deren Instrument.
    Replace(usize),
    /// Effekt auf eine Spur mit Mixer-Kanal: neuer Insert.
    Insert(usize),
}

/// Plugin aus der rechten Zone: Ziel, solange es über Spurliste oder Event-Anzeige schwebt;
/// beim Loslassen wird es angewendet.
fn plugin_drop(w: &mut ProjectWindow, ui: &Ui, g: &Geo) -> Option<PluginTarget> {
    let payload = egui::DragAndDrop::payload::<crate::rightzone::BrowserPlugin>(ui.ctx())?;
    let (pos, released) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.any_released()));
    let p = pos.filter(|p| g.lanes.contains(*p) || g.header.contains(*p))?;
    let info = &payload.0;
    let row = g.row_at(p.y);
    let kind = row.and_then(|r| w.project.tracks.get(r)).map(|t| t.kind);
    let target = match (info.is_instrument(), row, kind) {
        (true, Some(r), Some(TrackKind::Instrument)) => Some(PluginTarget::Replace(r)),
        (true, ..) => Some(PluginTarget::NewTrack),
        (false, Some(r), Some(k)) if crate::mixer::has_channel(k) => Some(PluginTarget::Insert(r)),
        (false, ..) => None,
    };
    if !released {
        return target;
    }
    egui::DragAndDrop::clear_payload(ui.ctx());
    match target {
        Some(PluginTarget::NewTrack) => w.add_instrument_track(Some(info.clone())),
        Some(PluginTarget::Replace(r)) => w.replace_instrument(r, info),
        Some(PluginTarget::Insert(r)) => {
            w.add_insert(r, info);
            w.selected_track = Some(r);
        }
        None => {}
    }
    None
}

fn paint_plugin_target(painter: &Painter, g: &Geo, target: PluginTarget) {
    let accent = crate::theme::tokens(painter.ctx()).accent;
    let full = g.header.union(g.lanes);
    match target {
        PluginTarget::Replace(r) | PluginTarget::Insert(r) => {
            let rect = Rect::from_x_y_ranges(
                full.x_range(),
                g.row_top(r)..=g.row_top(r) + g.row_height(r),
            );
            painter.rect_stroke(rect, 2.0, Stroke::new(2.0, accent), StrokeKind::Inside);
        }
        PluginTarget::NewTrack => {
            let y = g.content_bottom().min(g.lanes.bottom() - 2.0);
            painter.hline(full.x_range(), y, Stroke::new(2.0, accent));
            painter.text(
                pos2(g.header.left() + 12.0, y + 4.0),
                Align2::LEFT_TOP,
                tr("Neue Instrument-Spur"),
                FontId::proportional(12.0),
                accent,
            );
        }
    }
}

/// Audiodateien per Drag & Drop: auf eine Audio-Spur an die Mausposition,
/// sonst auf neue Spuren. Gibt das Ziel zurück, solange Dateien darüber schweben.
fn file_drop(w: &mut ProjectWindow, ui: &Ui, g: &Geo) -> Option<(Option<usize>, u64)> {
    let (hovering, mut dropped, pos, released) = ui.input(|i| {
        let paths: Vec<std::path::PathBuf> = i
            .raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .collect();
        (
            !i.raw.hovered_files.is_empty(),
            paths,
            i.pointer.latest_pos(),
            i.pointer.any_released(),
        )
    });
    // Aus dem Dateibrowser der rechten Zone gezogen.
    let browser = egui::DragAndDrop::payload::<crate::rightzone::BrowserFile>(ui.ctx());
    let hovering = hovering || browser.is_some();
    if let Some(file) = &browser
        && released
    {
        egui::DragAndDrop::clear_payload(ui.ctx());
        if pos.is_some_and(|p| g.lanes.contains(p)) {
            dropped.push(file.0.clone());
        }
    }
    let target = pos.filter(|p| g.lanes.contains(*p)).map(|p| {
        let row = g
            .row_at(p.y)
            .filter(|&r| w.project.tracks[r].kind == TrackKind::Audio);
        (row, w.snap_ticks(g.ticks(p.x)))
    });
    if !dropped.is_empty() {
        let before = w.project.clone();
        let (row, at) = target.unwrap_or((None, 0));
        let mut errors = Vec::new();
        for (i, path) in dropped.iter().enumerate() {
            // Erste Datei auf die Zielspur, weitere auf eigene neue Spuren.
            let t = if i == 0 { row.map(|r| (r, at)) } else { None };
            let t = t.or(Some((usize::MAX, at)));
            if let Err(e) = w.import_audio(path, t) {
                errors.push(e);
            }
        }
        w.message = (!errors.is_empty()).then(|| errors.join("; "));
        w.checkpoint(before);
        return None;
    }
    if hovering { target } else { None }
}

fn paint_drop_target(painter: &Painter, g: &Geo, row: Option<usize>, ticks: u64) {
    let x = g.x(ticks as f64);
    let accent = painter.ctx().global_style().visuals.selection.stroke.color;
    match row {
        Some(r) => {
            let top = g.row_top(r);
            let rect = Rect::from_min_max(pos2(x, top), pos2(g.lanes.right(), top + g.view.row_h));
            painter.rect_stroke(rect, 2.0, Stroke::new(2.0, accent), StrokeKind::Inside);
        }
        None => {
            painter.vline(x, g.lanes.y_range(), Stroke::new(2.0, accent));
            painter.text(
                pos2(x + 6.0, g.lanes.top() + 6.0),
                Align2::LEFT_TOP,
                tr("Neue Audio-Spur"),
                FontId::proportional(12.0),
                accent,
            );
        }
    }
}

fn scroll_and_zoom(
    w: &mut ProjectWindow,
    ui: &Ui,
    resp: &egui::Response,
    lanes: Rect,
    content_h: f32,
) {
    if resp.hovered() {
        let (delta, zoom, mods, hover) = ui.input(|i| {
            (
                i.smooth_scroll_delta,
                i.zoom_delta(),
                i.modifiers,
                i.pointer.hover_pos(),
            )
        });
        let anchor = hover.map(|p| p.x - lanes.left());
        if zoom != 1.0 {
            w.view.zoom(zoom, anchor);
        } else if mods.shift {
            w.view.scroll_ticks -= f64::from((delta.y + delta.x) / w.view.px_per_quarter) * TPQ;
        } else {
            w.view.scroll_y -= delta.y;
            w.view.scroll_ticks -= f64::from(delta.x / w.view.px_per_quarter) * TPQ;
        }
    }
    let max_y = (content_h - lanes.height() + w.view.row_h).max(0.0);
    w.view.scroll_y = w.view.scroll_y.clamp(0.0, max_y);
    w.view.scroll_ticks = w.view.scroll_ticks.max(0.0);
}

/// Während der Wiedergabe blättert die Ansicht mit, wenn der Cursor rechts hinausläuft.
fn follow_cursor(w: &mut ProjectWindow, lanes: Rect) {
    if !w.is_playing() || !w.follow_playback {
        return;
    }
    let visible = f64::from(lanes.width() / w.view.px_per_quarter) * TPQ;
    let c = w.cursor as f64;
    if c > w.view.scroll_ticks + visible * 0.95 || c < w.view.scroll_ticks {
        w.view.scroll_ticks = (c - visible * 0.05).max(0.0);
    }
}

fn interact(w: &mut ProjectWindow, ui: &Ui, resp: &egui::Response, g: &Geo) {
    let (mods, press, now) = ui.input(|i| {
        (
            i.modifiers,
            i.pointer.press_origin(),
            i.pointer.interact_pos(),
        )
    });
    if resp.clicked() || resp.drag_started() {
        w.key_editor.focused = false;
    }
    // Rechtsklick in die Event-Anzeige: Werkzeugpalette am Mauszeiger (Spec „Werkzeuge“).
    if resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && g.lanes.contains(p)
    {
        w.open_tool_palette(p);
        return;
    }
    if audition(w, ui, resp, g) {
        return;
    }
    if time_warp(w, ui, resp, g) {
        return;
    }
    // Doppelklick auf einen Fade-Griff öffnet den Kurveneditor.
    let fade_hit = resp
        .double_clicked()
        .then(|| resp.interact_pointer_pos())
        .flatten()
        .and_then(|p| smart_zone(w, g, p))
        .and_then(|(id, _, z)| match z {
            Zone::FadeIn => Some((id, crate::fades_ui::Side::In)),
            Zone::FadeOut => Some((id, crate::fades_ui::Side::Out)),
            _ => None,
        });
    if let Some((id, side)) = fade_hit {
        w.open_fade_editor(id, side);
        return;
    }
    // Doppelklick auf ein Audio-Event öffnet den Sample-Editor.
    if resp.double_clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && g.lanes.contains(p)
        && let Some(row) = g.row_at(p.y)
        && w.project.tracks[row].kind == TrackKind::Audio
        && let Some(id) = hit_event(w, g, row, p.y, g.ticks(p.x).max(0.0) as u64)
    {
        // Kein `return`: der Klick wählt das Event wie gewohnt aus. Audio-Parts öffnen den
        // Part-Editor.
        if w.project.event(id).is_some_and(|e| e.is_audio_part()) {
            w.open_audio_part_editor(id);
        } else {
            w.open_sample_editor(id);
        }
    }
    // Doppelklick auf einen Part öffnet den Key-Editor.
    if resp.double_clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && g.lanes.contains(p)
        && let Some(row) = g.row_at(p.y)
        && matches!(
            w.project.tracks[row].kind,
            TrackKind::Instrument | TrackKind::Midi
        )
        && let Some(id) = hit_event(w, g, row, p.y, g.ticks(p.x).max(0.0) as u64)
    {
        w.open_key_editor_from_selection(id);
        return;
    }

    if resp.drag_started()
        && let Some(p) = press
    {
        w.drag_before = Some(w.project.clone());
        w.drag = start_drag(w, g, p, mods);
    }
    if resp.dragged()
        && let (Some(d), Some(p)) = (w.drag, now)
    {
        live_drag(w, g, d, p, mods);
    }
    if resp.drag_stopped() {
        if let (Some(d), Some(p)) = (w.drag.take(), now) {
            finish_drag(w, g, d, p, mods);
        }
        if let Some(before) = w.drag_before.take() {
            w.checkpoint(before);
        }
        return;
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let before = w.project.clone();
        click(w, g, p, mods);
        w.checkpoint(before);
    }

    // Mauszeiger zeigt den Smart-Bereich an.
    if w.tool == Tool::Select
        && let Some(p) = resp.hover_pos()
    {
        let zone = match w.drag {
            Some(Drag::TrimStart { .. } | Drag::TrimEnd { .. }) => Some(Zone::TrimStart),
            Some(Drag::FadeIn { .. } | Drag::FadeOut { .. }) => Some(Zone::FadeIn),
            Some(Drag::EventGain { .. }) => Some(Zone::Gain),
            Some(_) => None,
            None => smart_zone(w, g, p).map(|(_, _, z)| z),
        };
        match zone {
            Some(Zone::TrimStart | Zone::TrimEnd) => {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal)
            }
            Some(Zone::FadeIn | Zone::FadeOut) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
            Some(Zone::Gain) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical),
            _ => {}
        }
    }
}

/// Wiedergabe-Werkzeug (Spec „Werkzeuge“, Taste 9): Drücken spielt ab der Mausposition,
/// Ziehen scrubbt (der Cursor folgt der Maus), Loslassen hält an und setzt den Cursor
/// zurück. Lief die Wiedergabe schon, springt sie nur und läuft weiter. Gibt `true` zurück,
/// solange das Werkzeug die Maus für sich beansprucht.
fn audition(w: &mut ProjectWindow, ui: &Ui, resp: &egui::Response, g: &Geo) -> bool {
    if w.tool != Tool::Play && w.audition.is_none() {
        return false;
    }
    let (pressed, down, pos) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.interact_pos(),
        )
    });
    match w.audition {
        None => {
            let Some(p) = pos.filter(|p| pressed && resp.hovered() && g.lanes.contains(*p)) else {
                return false;
            };
            let was_playing = w.is_playing();
            w.audition = Some(Audition {
                cursor_before: w.cursor,
                was_playing,
                last_x: p.x,
            });
            w.locate(g.ticks(p.x).max(0.0) as u64);
            if !was_playing {
                w.play();
            }
            true
        }
        Some(a) if down => {
            if let Some(p) = pos
                && (p.x - a.last_x).abs() >= 2.0
            {
                w.locate(g.ticks(p.x).max(0.0) as u64);
                w.audition = Some(Audition { last_x: p.x, ..a });
            }
            true
        }
        Some(a) => {
            w.audition = None;
            if !a.was_playing {
                w.stop_transport();
                w.locate(a.cursor_before);
            }
            true
        }
    }
}

/// Laufende Zeitkorrektur: Projekt vor dem Ziehen und die Stelle (in Sekunden), die gepackt
/// wurde.
#[derive(Debug, Clone)]
pub(crate) struct TimeWarp {
    before: studiocraft_project::Project,
    seconds: f64,
    /// Zählzeit, auf der die gepackte Stelle gerade liegt.
    pub target: Option<u64>,
}

/// Nächste Zählzeit zu `ticks` (Taktart beachtet).
fn nearest_beat(tempo: &studiocraft_project::TempoMap, ticks: u64) -> u64 {
    let (_, start, sig) = tempo.bar_at(ticks);
    let beat = sig.beat_ticks().max(1);
    let k = ((ticks - start) as f64 / beat as f64).round() as u64;
    start + k * beat
}

/// Zeitkorrektur (Spec „Werkzeuge“, Taste 0; Issue #42): Drücken packt die Stelle unter der
/// Maus – z. B. einen Schlag in einer Audioaufnahme –, Ziehen legt sie auf die Zählzeit unter
/// der Maus (mit Snap, Alt: frei). Die Tempo-Map ändert sich dabei nur seit dem letzten
/// Tempowechsel davor; Audio auf linearen Spuren behält seine Zeit, das Raster passt sich an.
/// Loslassen ist ein Rückgängig-Schritt. Gibt `true` zurück, solange das Werkzeug die Maus hat.
fn time_warp(w: &mut ProjectWindow, ui: &Ui, resp: &egui::Response, g: &Geo) -> bool {
    if w.tool != Tool::TimeWarp && w.time_warp.is_none() {
        return false;
    }
    let (pressed, down, pos, alt) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.interact_pos(),
            i.modifiers.alt,
        )
    });
    match w.time_warp.take() {
        None => {
            let Some(p) = pos.filter(|p| pressed && resp.hovered() && g.lanes.contains(*p)) else {
                return false;
            };
            let at = g.ticks(p.x).max(0.0) as u64;
            let seconds = w.project.tempo.seconds(studiocraft_project::Position(at));
            w.time_warp = Some(TimeWarp {
                before: w.project.clone(),
                seconds,
                target: None,
            });
            true
        }
        Some(mut tw) if down => {
            if let Some(p) = pos {
                // Ziel in der Tick-Anzeige; sie bleibt beim Ziehen stehen, nur das Audio wandert.
                let at = g.ticks(p.x).max(0.0) as u64;
                let target = if w.snap && !alt {
                    nearest_beat(&tw.before.tempo, at)
                } else {
                    at
                };
                if tw.target != Some(target) {
                    tw.target = Some(target);
                    let mut project = tw.before.clone();
                    if let Some(map) = tw.before.tempo.warped(target, tw.seconds) {
                        project.set_tempo_map(map);
                    }
                    w.project = project;
                }
            }
            w.time_warp = Some(tw);
            true
        }
        Some(tw) => {
            w.checkpoint(tw.before);
            true
        }
    }
}

/// Ziel der laufenden Zeitkorrektur als senkrechte Linie.
fn paint_time_warp(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let Some(target) = w.time_warp.as_ref().and_then(|t| t.target) else {
        return;
    };
    let x = g.x(target as f64);
    if g.lanes.x_range().contains(x) {
        let t = crate::theme::tokens(painter.ctx());
        painter.vline(x, g.lanes.y_range(), Stroke::new(2.0, t.accent));
    }
}

/// Ziehen am linken (`start`) bzw. rechten Rand im eingestellten Modus der Objektauswahl.
fn resize_edge(w: &mut ProjectWindow, id: EventId, at: u64, start: bool) {
    use crate::tools::ResizeMode;
    match w.resize_mode {
        ResizeMode::Normal => {
            if start {
                w.project.trim_start(id, at);
            } else {
                w.project.trim_end(id, at);
            }
        }
        ResizeMode::Slide => {
            if start {
                w.project.slide_start(id, at);
            } else {
                w.project.slide_end(id, at);
            }
        }
        ResizeMode::Stretch => {
            let Some((a, b)) = w.project.event(id).map(|e| (e.start.0, e.end())) else {
                return;
            };
            let (na, nb) = if start { (at.min(b - 1), b) } else { (a, at) };
            if !w.project.stretch_to(id, na, nb) {
                w.message = Some(tr("Inhalt strecken geht bisher nur bei MIDI-Parts").into());
                if start {
                    w.project.trim_start(id, at);
                } else {
                    w.project.trim_end(id, at);
                }
            }
        }
    }
}

/// Änderungen, die schon während des Ziehens sichtbar und hörbar sind.
fn live_drag(w: &mut ProjectWindow, g: &Geo, d: Drag, p: Pos2, mods: egui::Modifiers) {
    let raw = g.ticks(p.x).max(0.0);
    let snapped = if mods.alt {
        raw.round() as u64
    } else {
        w.snap_ticks(raw)
    };
    match d {
        Drag::Volume { row } => {
            set_header_volume(w, g, row, p.x);
            return;
        }
        Drag::Ruler { start } => {
            let t = w.snap_ticks(raw);
            w.left_locator = start.min(t);
            w.right_locator = start.max(t);
            return;
        }
        Drag::Scrub => {
            w.locate(snapped);
            return;
        }
        Drag::TrackMove { .. } => return,
        Drag::FolderShift {
            row,
            start,
            applied,
        } => {
            let want = snapped as i64 - start as i64 - applied;
            let inside = w.project.tracks_inside(row);
            let got = w.project.shift_tracks(&inside, want);
            w.drag = Some(Drag::FolderShift {
                row,
                start,
                applied: applied + got,
            });
            return;
        }
        Drag::Stroke { .. } => {
            w.drag = Some(stroke(w, g, d, p));
            return;
        }
        Drag::ZoomRect { .. } => return,
        Drag::TrimStart { id } => resize_edge(w, id, snapped, true),
        Drag::TrimEnd { id } => resize_edge(w, id, snapped, false),
        Drag::FadeIn { id } => {
            if let Some(ev) = w.project.event(id) {
                let len = (raw as u64).saturating_sub(ev.start.0);
                let out = ev.fade_out;
                w.project.set_fades(id, len, out);
            }
        }
        Drag::FadeOut { id } => {
            if let Some(ev) = w.project.event(id) {
                let len = ev.end().saturating_sub(raw as u64);
                let fin = ev.fade_in;
                w.project.set_fades(id, fin, len.min(ev.length - fin));
            }
        }
        Drag::EventGain { id, y0, db0 } => {
            let per_px = if mods.shift {
                GAIN_DB_PER_PX / 4.0
            } else {
                GAIN_DB_PER_PX
            };
            let db = (db0 + (y0 - p.y) * per_px).clamp(MIN_DB, EVENT_MAX_DB);
            // Ein Zehntel dB genügt; ganz unten stumm.
            let db = (db * 10.0).round() / 10.0;
            if let Some(ev) = w.project.event_mut(id) {
                ev.gain_db = db;
            }
        }
        Drag::Range { row, start } => {
            let r = g
                .row_at(p.y.clamp(g.lanes.top(), g.lanes.bottom() - 1.0))
                .unwrap_or(row);
            w.range = Some((
                row.min(r),
                row.max(r),
                start.min(snapped),
                start.max(snapped),
            ));
            return;
        }
        Drag::TempoPoint { row, index, range } => {
            let t = if mods.alt {
                raw.round() as u64
            } else {
                snapped
            };
            tempo::live_drag(w, g, (row, index, range), p, t);
            return;
        }
        Drag::Marker { index, grab } => {
            let t = if mods.alt {
                (raw - grab).max(0.0).round() as u64
            } else {
                w.snap_ticks(raw - grab)
            };
            if let Some(i) = w.project.move_marker(index, t) {
                w.drag = Some(Drag::Marker { index: i, grab });
            }
            return;
        }
        Drag::AutoPoint { .. }
        | Drag::AutoDraw { .. }
        | Drag::AutoShape { .. }
        | Drag::AutoScale { .. } => {
            w.drag = Some(automation::live_drag(w, g, d, p, mods.alt));
            return;
        }
        Drag::Move { .. } | Drag::Draw { .. } | Drag::Comp { .. } | Drag::Repeat { .. } => return,
    }
    w.dirty = true;
}

fn start_drag(w: &mut ProjectWindow, g: &Geo, p: Pos2, mods: egui::Modifiers) -> Option<Drag> {
    let alt = mods.alt;
    if g.ruler.contains(p) {
        // Obere Hälfte: neuer Cycle-Bereich; untere Hälfte: Cursor ziehen (Spec).
        return Some(if p.y < g.ruler.center().y {
            Drag::Ruler {
                start: w.snap_ticks(g.ticks(p.x)),
            }
        } else {
            Drag::Scrub
        });
    }
    if g.header.contains(p) {
        let row = g.row_at(p.y)?;
        let kind = w.project.tracks.get(row)?.kind;
        if crate::mixer::has_channel(kind) && g.header_volume(row).contains(p) {
            return Some(Drag::Volume { row });
        }
        // Spur verschieben: im Kopf der Spur, aber nicht auf Farbleiste, Pfeil oder
        // Schaltern (deren Klicks bleiben, wie sie sind).
        let in_head = p.y < g.row_top(row) + g.view.row_h;
        let on_control = p.x < g.header.left() + STRIP_W
            || automation::arrow_rect(g, row).expand(3.0).contains(p)
            || g.header_buttons(row).iter().any(|b| b.contains(p));
        return (in_head && !on_control).then_some(Drag::TrackMove { row });
    }
    if !g.lanes.contains(p) {
        return None;
    }
    if let Some(row) = g.row_at(p.y)
        && w.project.tracks[row].kind == TrackKind::Tempo
    {
        return tempo::start_drag(w, g, row, p);
    }
    if let Some(row) = g.row_at(p.y)
        && w.project.tracks[row].kind == TrackKind::Marker
    {
        if w.tool != Tool::Select {
            return None;
        }
        let i = marker_at(w, g, p)?;
        let grab = g.ticks(p.x) - w.project.markers[i].start as f64;
        return Some(Drag::Marker { index: i, grab });
    }
    if let Some(d) = automation::start_drag(w, g, p, alt) {
        return d;
    }
    let row = g.row_at(p.y)?;
    let ticks = g.ticks(p.x);
    // Ordnerzeile mit der Objektauswahl: den ganzen Ordnerbereich verschieben.
    if w.tool == Tool::Select && w.project.tracks[row].kind == TrackKind::Folder {
        return Some(Drag::FolderShift {
            row,
            start: if alt {
                ticks.max(0.0) as u64
            } else {
                w.snap_ticks(ticks)
            },
            applied: 0,
        });
    }
    match w.tool {
        Tool::Select => {
            let (id, row, zone) = smart_zone(w, g, p)?;
            if !w.selection.contains(&id) {
                w.selection = vec![id];
            }
            w.selected_track = Some(row);
            let start = w.project.event(id)?.start.0 as f64;
            Some(match zone {
                Zone::Body => Drag::Move {
                    id,
                    grab: ticks - start,
                    row,
                    copy: mods.command,
                    shared: mods.command && mods.shift,
                },
                Zone::TrimStart => Drag::TrimStart { id },
                Zone::TrimEnd if alt => {
                    let (a, b) = selection_span(w)?;
                    Drag::Repeat { start: a, end: b }
                }
                Zone::TrimEnd => Drag::TrimEnd { id },
                Zone::FadeIn => Drag::FadeIn { id },
                Zone::FadeOut => Drag::FadeOut { id },
                Zone::Gain => Drag::EventGain {
                    id,
                    y0: p.y,
                    db0: w.project.event(id).map_or(0.0, |e| e.gain_db),
                },
            })
        }
        Tool::Range => {
            let t = w.snap_ticks(ticks);
            // Umschalt: die bestehende Auswahl vom gegenüberliegenden Ende aus erweitern.
            if mods.shift
                && let Some((r0, r1, a, b)) = w.range
            {
                let (anchor_row, anchor) = range_anchor((r0, r1, a, b), row, t);
                w.range = Some((
                    anchor_row.min(row),
                    anchor_row.max(row),
                    anchor.min(t),
                    anchor.max(t),
                ));
                return Some(Drag::Range {
                    row: anchor_row,
                    start: anchor,
                });
            }
            w.range = Some((row, row, t, t));
            Some(Drag::Range { row, start: t })
        }
        Tool::Erase | Tool::Mute => {
            let d = Drag::Stroke {
                erase: w.tool == Tool::Erase,
                mute_to: None,
                last: p,
            };
            Some(stroke(w, g, d, p))
        }
        Tool::Zoom => Some(Drag::ZoomRect { start: p }),
        Tool::Draw if w.project.tracks[row].kind.holds_events() => Some(Drag::Draw {
            row,
            start: w.snap_ticks(ticks),
        }),
        Tool::Comp => {
            let id = hit_event(w, g, row, p.y, ticks.max(0.0) as u64)?;
            w.selected_track = Some(row);
            Some(Drag::Comp {
                row,
                id,
                start: w.snap_ticks(ticks),
            })
        }
        _ => None,
    }
}

/// Comping: Abschnitt `a..b` (Klick: das ganze Take) hörbar machen und auswählen.
fn comp(w: &mut ProjectWindow, id: EventId, a: u64, b: u64) {
    let (a, b) = if a == b {
        match w.project.event(id) {
            Some(e) => (e.start.0, e.end()),
            None => return,
        }
    } else {
        (a.min(b), a.max(b))
    };
    if let Some(mid) = w.project.comp(id, a, b) {
        w.selection = vec![mid];
    }
}

/// Ziel einer Verschiebung: (Zeilenversatz, Tick-Versatz) gegenüber dem gegriffenen Event.
fn move_delta(
    w: &ProjectWindow,
    g: &Geo,
    id: EventId,
    grab: f64,
    row: usize,
    p: Pos2,
    no_snap: bool,
) -> Option<(isize, i64)> {
    let ev = w.project.event(id)?;
    let raw = g.ticks(p.x) - grab;
    let new_start = if no_snap {
        raw.max(0.0).round() as u64
    } else {
        w.snap_move(raw, ev.length, ev.start.0)
    };
    let target_row = g
        .row_at(p.y.clamp(g.lanes.top(), g.lanes.bottom() - 1.0))
        .unwrap_or(row);
    // Versatz in Anzeigezeilen (Ordner zeigen ihre Spuren in Baumreihenfolge).
    let from = g.display_pos(row).unwrap_or(0) as isize;
    let to = g.display_pos(target_row).map_or(from, |d| d as isize);
    Some((to - from, new_start as i64 - ev.start.0 as i64))
}

/// Setzt die Lautstärke der Spur aus der Mausposition im Regler des Spurkopfs
/// (gleiche Kennlinie wie der Fader im Mixer).
fn set_header_volume(w: &mut ProjectWindow, g: &Geo, row: usize, x: f32) {
    let r = g.header_volume(row);
    let pos = ((x - r.left()) / r.width()).clamp(0.0, 1.0);
    if let Some(track) = w.project.tracks.get_mut(row) {
        track.volume_db = crate::mixer::pos_to_db(pos);
        w.dirty = true;
    }
}

/// Gegenüberliegende Ecke der Bereichsauswahl `r` zur Mausposition (`row`, `t`): von dort
/// aus wird beim Erweitern mit Umschalt aufgezogen.
fn range_anchor(r: (usize, usize, u64, u64), row: usize, t: u64) -> (usize, u64) {
    let (r0, r1, a, b) = r;
    let anchor = if t.saturating_mul(2) < a.saturating_add(b) {
        b
    } else {
        a
    };
    let anchor_row = if row.saturating_mul(2) < r0 + r1 {
        r1
    } else {
        r0
    };
    (anchor_row, anchor)
}

/// Löschen bzw. Stummschalten entlang der Strecke von der letzten Mausposition bis `p`
/// (alle 3 px ein Treffertest). Gibt den aktualisierten Zustand zurück.
fn stroke(w: &mut ProjectWindow, g: &Geo, d: Drag, p: Pos2) -> Drag {
    let Drag::Stroke {
        erase,
        mut mute_to,
        last,
    } = d
    else {
        return d;
    };
    let steps = ((p - last).length() / 3.0).ceil().clamp(1.0, 2000.0) as usize;
    for k in 0..=steps {
        let q = last + (p - last) * (k as f32 / steps as f32);
        if !g.lanes.contains(q) {
            continue;
        }
        let Some(row) = g.row_at(q.y) else { continue };
        let Some(id) = hit_event(w, g, row, q.y, g.ticks(q.x).max(0.0) as u64) else {
            continue;
        };
        if erase {
            w.project.delete(id);
            w.selection.retain(|&s| s != id);
        } else if let Some(ev) = w.project.event_mut(id) {
            let to = *mute_to.get_or_insert(!ev.muted);
            ev.muted = to;
        }
    }
    Drag::Stroke {
        erase,
        mute_to,
        last: p,
    }
}

/// Zoom-Rechteck von `a` nach `b`: der Zeitbereich füllt danach die Breite der
/// Event-Anzeige, die oberste Spur des Rechtecks steht oben. Unter 6 px Breite wird wie bei
/// einem Klick eingezoomt (Alt: ausgezoomt).
fn zoom_to_rect(w: &mut ProjectWindow, g: &Geo, a: Pos2, b: Pos2) {
    let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
    if x1 - x0 < 6.0 {
        return;
    }
    let t0 = g.ticks(x0).max(0.0);
    let t1 = g.ticks(x1).max(t0 + 1.0);
    let quarters = ((t1 - t0) / TPQ) as f32;
    w.view.px_per_quarter = (g.lanes.width() / quarters.max(1e-3)).clamp(1.0, 2000.0);
    w.view.scroll_ticks = t0;
    let top = a.y.min(b.y);
    if let Some(row) = g.row_at(top.max(g.lanes.top())) {
        w.view.scroll_y = g.row_top(row) - g.lanes.top() + g.view.scroll_y;
    }
}

/// Ziel beim Ziehen einer Spur: (vor welcher Spur, in welchen Ordner). Die mittlere Hälfte
/// eines Ordnerkopfs legt die Spur in diesen Ordner (ans Ende); die obere Hälfte einer Zeile
/// setzt davor, die untere dahinter, jeweils in den Ordner der Nachbarspur. Unter der
/// letzten Spur: ans Ende der obersten Ebene. `None`, wenn sich nichts ändern würde.
fn track_drop(
    w: &ProjectWindow,
    g: &Geo,
    row: usize,
    p: Pos2,
) -> Option<(Option<usize>, Option<usize>)> {
    let y = p.y.max(g.lanes.top());
    let Some(target) = g.row_at(y) else {
        return Some((None, None));
    };
    if target == row {
        return None;
    }
    let frac = (y - g.row_top(target)) / g.view.row_h;
    let project = &w.project;
    if project.tracks[target].kind == TrackKind::Folder && (0.25..0.75).contains(&frac) {
        return Some((None, Some(target)));
    }
    if frac < 0.5 {
        return Some((Some(target), project.parent_of(target)));
    }
    // Dahinter: vor die nächste angezeigte Spur (bei offenem Ordner: sein erstes Kind).
    let next = g
        .display_pos(target)
        .and_then(|d| g.order.get(d + 1..))
        .and_then(|rest| rest.iter().copied().find(|&t| t != row));
    match next {
        Some(n) => Some((Some(n), project.parent_of(n))),
        None => Some((None, project.parent_of(target))),
    }
}

fn finish_drag(w: &mut ProjectWindow, g: &Geo, d: Drag, p: Pos2, mods: egui::Modifiers) {
    match d {
        Drag::Ruler { .. } => w.send_locators(),
        Drag::Scrub => {}
        Drag::FolderShift { .. } => {}
        Drag::TrackMove { row } => {
            if let Some((before, folder)) = track_drop(w, g, row, p) {
                w.project.move_track(row, before, folder);
            }
        }
        Drag::Volume { .. } => {}
        Drag::Stroke { .. } => {}
        Drag::ZoomRect { start } => zoom_to_rect(w, g, start, p),
        // Schon während des Ziehens angewendet.
        Drag::Range { .. } => {
            if w.range.is_some_and(|(_, _, a, b)| b <= a) {
                w.range = None;
            }
        }
        Drag::TrimStart { .. }
        | Drag::TrimEnd { .. }
        | Drag::FadeIn { .. }
        | Drag::FadeOut { .. }
        | Drag::EventGain { .. }
        | Drag::AutoPoint { .. }
        | Drag::AutoDraw { .. }
        | Drag::AutoShape { .. }
        | Drag::AutoScale { .. }
        | Drag::Marker { .. }
        | Drag::TempoPoint { .. } => {}
        Drag::Comp { id, start, .. } => {
            let end = w.snap_ticks(g.ticks(p.x));
            comp(w, id, start, end);
        }
        Drag::Draw { row, start } => {
            let end = w.snap_ticks(g.ticks(p.x));
            let min = w.grid_ticks();
            let (a, b) = (start.min(end), start.max(end));
            let len = (b - a).max(min);
            if let Some(id) = w.project.create_event(row, a, len) {
                w.selection = vec![id];
            }
        }
        Drag::Repeat { start, end } => {
            let n = repeat_count(g, start, end, p);
            if n > 0 {
                let sel = w.selection.clone();
                let new = w.project.repeat(&sel, n, false);
                if !new.is_empty() {
                    w.selection = new;
                }
            }
        }
        Drag::Move {
            id,
            grab,
            row,
            copy,
            shared,
        } => {
            let Some((drow, dt)) = move_delta(w, g, id, grab, row, p, mods.alt) else {
                return;
            };
            // Strg erst während des Ziehens gedrückt: nur waagerecht oder senkrecht.
            let (drow, dt) = if mods.command && !copy {
                lock_direction(g, drow, dt)
            } else {
                (drow, dt)
            };
            if drow == 0 && dt == 0 {
                return;
            }
            // Snap-Modus „Schieben“: ein Event innerhalb seiner Spur, die Nachbarn rücken nach.
            if w.snap
                && w.snap_mode == crate::tools::SnapMode::Shuffle
                && !mods.alt
                && !copy
                && drow == 0
                && let [only] = w.selection[..]
                && let Some(ev) = w.project.event(only)
            {
                let start = (ev.start.0 as i64 + dt).max(0) as u64;
                w.project.shuffle_event(only, start);
                return;
            }
            let mut new_sel = Vec::new();
            for sel in w.selection.clone() {
                let Some((t, _)) = w.project.find(sel) else {
                    continue;
                };
                let Some(ev) = w.project.event(sel) else {
                    continue;
                };
                let target = g.offset_row(t, drow);
                let start = (ev.start.0 as i64 + dt).max(0) as u64;
                match w.project.move_event(sel, target, start, copy) {
                    Some(nid) => {
                        if copy {
                            if shared {
                                w.project.link_shared(sel, nid);
                            } else {
                                w.project.unlink_shared(nid);
                            }
                        }
                        new_sel.push(nid);
                    }
                    None => new_sel.push(sel),
                }
            }
            w.selection = new_sel;
        }
    }
}

fn click(w: &mut ProjectWindow, g: &Geo, p: Pos2, mods: egui::Modifiers) {
    if g.ruler.contains(p) {
        let t = w.snap_ticks(g.ticks(p.x));
        if mods.command {
            w.left_locator = t;
        } else if mods.alt {
            w.right_locator = t;
        } else {
            w.locate(t);
            return;
        }
        if w.left_locator > w.right_locator {
            std::mem::swap(&mut w.left_locator, &mut w.right_locator);
        }
        w.send_locators();
        return;
    }
    if g.header.contains(p) {
        if automation::header_click(w, g, p) {
            return;
        }
        // Farbleiste ganz links: Palette der Spurfarbe.
        if let Some(row) = g.row_at(p.y)
            && p.x < g.header.left() + STRIP_W
        {
            w.open_color_popup(crate::colors::Target::Track(row), p);
            return;
        }
        if let Some(row) = g.row_at(p.y)
            && automation::arrow_rect(g, row).expand(3.0).contains(p)
        {
            let track = &mut w.project.tracks[row];
            if track.kind == TrackKind::Folder {
                track.folded = !track.folded;
            } else {
                automation::toggle(track);
            }
            return;
        }
        if let Some(row) = g.row_at(p.y) {
            let [m, s, r, mon, read, write, base, lanes] = g.header_buttons(row);
            let kind = w.project.tracks[row].kind;
            if crate::mixer::has_channel(kind) && g.header_volume(row).contains(p) {
                if mods.command {
                    w.project.tracks[row].volume_db = 0.0;
                    w.dirty = true;
                } else {
                    set_header_volume(w, g, row, p.x);
                }
                return;
            }
            // Lineal-Spuren haben kein Mute/Solo (ein Solo schaltete sonst alles stumm).
            if kind == TrackKind::Ruler && (m.contains(p) || s.contains(p)) {
                return;
            }
            let track = &mut w.project.tracks[row];
            if m.contains(p) {
                track.mute = !track.mute;
            } else if s.contains(p) {
                // Strg: Solo-Defeat; Alt: exklusives Solo (Spec „Kanalzug-Elemente“).
                if mods.command {
                    track.solo_defeat = !track.solo_defeat;
                } else if mods.alt {
                    w.project.solo_exclusive(row);
                } else {
                    track.solo = !track.solo;
                }
            } else if r.contains(p) && kind.holds_events() {
                track.record = !track.record;
            } else if mon.contains(p) && kind == TrackKind::Audio {
                let on = !track.monitor;
                w.set_monitor(row, on);
            } else if read.contains(p) && crate::mixer::has_channel(kind) {
                track.read = !track.read;
            } else if write.contains(p) && crate::mixer::has_channel(kind) {
                track.write = !track.write;
            } else if base.contains(p) && kind.holds_events() {
                track.time_base = Some(match track.time_base() {
                    TimeBase::Musical => TimeBase::Linear,
                    TimeBase::Linear => TimeBase::Musical,
                });
            } else if lanes.contains(p) && kind.holds_events() {
                track.show_lanes = !track.show_lanes;
            } else {
                w.selected_track = Some(row);
            }
        }
        return;
    }
    if !g.lanes.contains(p) {
        return;
    }
    if let Some(row) = g.row_at(p.y)
        && w.project.tracks[row].kind == TrackKind::Tempo
    {
        tempo::click(w, g, row, p);
        return;
    }
    if let Some(row) = g.row_at(p.y)
        && w.project.tracks[row].kind == TrackKind::Marker
    {
        w.selected_track = Some(row);
        let hit = marker_at(w, g, p);
        match (w.tool, hit) {
            (Tool::Erase, Some(i)) => {
                w.project.remove_marker(i);
            }
            (Tool::Draw, None) => {
                let t = w.snap_ticks(g.ticks(p.x));
                w.project.add_marker(t);
            }
            (_, Some(i)) => w.select_marker(i),
            _ => {}
        }
        return;
    }
    if automation::click(w, g, p, mods.alt) {
        return;
    }
    let Some(row) = g.row_at(p.y) else { return };
    let ticks = g.ticks(p.x).max(0.0);
    let hit = hit_event(w, g, row, p.y, ticks as u64);
    w.selected_track = Some(row);
    match (w.tool, hit) {
        (Tool::Select, Some(id)) => {
            if mods.shift {
                if let Some(i) = w.selection.iter().position(|&s| s == id) {
                    w.selection.remove(i);
                } else {
                    w.selection.push(id);
                }
            } else {
                w.selection = vec![id];
            }
        }
        (Tool::Select, None) => {
            w.selection.clear();
            w.range = None;
        }
        (Tool::Range, _) => {
            let t = w.snap_ticks(ticks);
            w.range = match w.range {
                // Umschalt+Klick: Auswahl bis hierher erweitern.
                Some(r) if mods.shift => {
                    let (anchor_row, anchor) = range_anchor(r, row, t);
                    Some((
                        anchor_row.min(row),
                        anchor_row.max(row),
                        anchor.min(t),
                        anchor.max(t),
                    ))
                }
                _ => None,
            };
        }
        (Tool::Comp, Some(id)) => comp(w, id, 0, 0),
        (Tool::Glue, Some(id)) => {
            // Alt: alle folgenden Events dieser Spur ankleben.
            let mut result = w.project.glue(id);
            while mods.alt && result.is_ok() {
                result = w.project.glue(id);
            }
            match result {
                Ok(_) => w.selection = vec![id],
                Err(e) if !mods.alt => w.message = Some(e.to_owned()),
                Err(_) => {}
            }
        }
        (Tool::Split, Some(id)) => {
            let at = w.snap_ticks(ticks);
            if mods.alt {
                // Alt: das ganze Event in Stücke dieser Länge teilen (Spec „Werkzeuge“).
                let ids = w.project.split_equal(id, at);
                if ids.len() as u64 == studiocraft_project::Project::MAX_EQUAL_PIECES {
                    w.message = Some(crate::i18n::trf(
                        "Höchstens {max} Stücke; der Rest bleibt ein Event",
                        &[("max", &studiocraft_project::Project::MAX_EQUAL_PIECES)],
                    ));
                }
            } else {
                w.project.split(id, at);
            }
        }
        (Tool::Erase, Some(id)) => {
            w.project.delete(id);
            w.selection.retain(|&s| s != id);
        }
        (Tool::Mute, Some(id)) => {
            if let Some(ev) = w.project.event_mut(id) {
                ev.muted = !ev.muted;
            }
        }
        (Tool::Draw, None) => {
            let start = w.snap_ticks(ticks);
            let len = w.bar_ticks();
            if let Some(id) = w.project.create_event(row, start, len) {
                w.selection = vec![id];
            }
        }
        (Tool::Zoom, _) => {
            let f = if mods.alt { 1.0 / 1.5 } else { 1.5 };
            w.view.zoom(f, Some(p.x - g.lanes.left()));
        }
        // Wiedergabe-Werkzeug: läuft über Drücken und Loslassen, siehe `audition`.
        (Tool::Play, _) => {}
        _ => {}
    }
}

fn track_color(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Abstand der Taktlinien, damit Beschriftungen nicht überlappen.
fn bar_step(w: &ProjectWindow) -> u64 {
    let bar_px = w.bar_ticks() as f32 / TPQ as f32 * w.view.px_per_quarter;
    let mut step = 1;
    while bar_px * step as f32 * 1.0 < 40.0 {
        step *= 2;
    }
    step
}

fn paint_grid(w: &ProjectWindow, painter: &Painter, g: &Geo, rect: Rect, labels: bool) {
    let visuals = painter.ctx().global_style().visuals.clone();
    let bar = w.bar_ticks();
    let beat = w.beat_ticks();
    let beat_px = beat as f32 / TPQ as f32 * w.view.px_per_quarter;
    let t0 = g.ticks(rect.left()).max(0.0) as u64;
    let t1 = g.ticks(rect.right()).max(0.0) as u64 + bar;
    let step = bar_step(w);
    let strong = Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color);
    let weak = Stroke::new(
        1.0,
        visuals
            .widgets
            .noninteractive
            .bg_stroke
            .color
            .gamma_multiply(0.45),
    );
    // Takt- und Zählzeitlinien entlang der Taktwechsel.
    for (t, bar_no) in w.meter_lines(t0, t1, beat_px >= 10.0 && !labels) {
        let x = g.x(t as f64);
        match bar_no {
            Some(n) if (n - 1) % step == 0 => {
                painter.vline(x, rect.y_range(), strong);
                if labels {
                    painter.text(
                        pos2(x + 3.0, rect.bottom() - 4.0),
                        Align2::LEFT_BOTTOM,
                        n.to_string(),
                        FontId::proportional(11.0),
                        visuals.text_color(),
                    );
                }
            }
            Some(_) => {}
            None => {
                painter.vline(x, rect.y_range(), weak);
            }
        }
    }
    // Feines Snap-Raster (z. B. 1/16, Frames, „An Zoom anpassen“) zwischen den Zählzeiten.
    if !labels {
        let fine = Stroke::new(1.0, weak.color.gamma_multiply(0.75));
        let lines = w.fine_grid_lines(t0 as f64, t1 as f64, w.view.px_per_quarter, beat_px >= 10.0);
        for t in lines {
            painter.vline(g.x(t), rect.y_range(), fine);
        }
    }
}

fn paint_lanes(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let visuals = painter.ctx().global_style().visuals.clone();
    painter.rect_filled(g.lanes, 0.0, visuals.extreme_bg_color);
    for row in g.visible_rows() {
        let top = g.row_top(row);
        let rect = Rect::from_min_size(
            pos2(g.lanes.left(), top),
            vec2(g.lanes.width(), g.row_height(row)),
        );
        if row % 2 == 1 {
            painter.rect_filled(rect, 0.0, visuals.faint_bg_color.gamma_multiply(0.5));
        }
        // Aufgeklappte Lanes: dünne Linien zwischen den Lanes.
        let lanes = (g.row_height(row) / g.view.row_h).round() as usize;
        for k in 1..lanes {
            painter.hline(
                rect.x_range(),
                top + k as f32 * g.view.row_h,
                Stroke::new(1.0, visuals.faint_bg_color.gamma_multiply(0.6)),
            );
        }
        painter.hline(
            rect.x_range(),
            rect.bottom(),
            Stroke::new(1.0, visuals.faint_bg_color),
        );
    }
    paint_grid(w, painter, g, g.lanes, false);

    if w.cycle {
        let r = Rect::from_x_y_ranges(
            g.x(w.left_locator as f64)..=g.x(w.right_locator as f64),
            g.lanes.y_range(),
        );
        painter.rect_filled(
            r,
            0.0,
            crate::theme::tokens(painter.ctx())
                .cycle_fill
                .gamma_multiply(0.5),
        );
    }

    let t0 = g.ticks(g.lanes.left()).max(0.0) as u64;
    let t1 = g.ticks(g.lanes.right()).max(0.0) as u64;
    for row in g.visible_rows() {
        let track = &w.project.tracks[row];
        let h = g.view.row_h - 4.0;
        let lanes = track.show_lanes.then(|| track.lanes().0);
        // Aufgeklappt: nicht hörbare Abschnitte abdunkeln (Comping).
        let audible = track.show_lanes.then(|| track.audible_parts());
        // Zugeklappt: das klingende Event (höchster Comp-Rang) oben zeichnen.
        let mut order: Vec<usize> = (0..track.events.len()).collect();
        if track.events.iter().any(|e| e.comp > 0) {
            order.sort_by_key(|&i| (track.events[i].comp, i));
        }
        for i in order {
            let ev = &track.events[i];
            if ev.end() < t0 || ev.start.0 > t1 {
                continue;
            }
            let lane = lanes.as_ref().and_then(|l| l.get(i).copied()).unwrap_or(0);
            let top = g.row_top(row) + lane as f32 * g.view.row_h + 2.0;
            let rect = Rect::from_min_max(
                pos2(g.x(ev.start.0 as f64), top),
                pos2(g.x(ev.end() as f64), top + h),
            );
            paint_event(w, painter, g, track, ev, rect);
            if let Some(parts) = audible.as_ref().and_then(|a| a.get(i)) {
                let mut from = ev.start.0;
                let shade = Color32::from_black_alpha(120);
                for &(a, b) in parts.iter().chain(std::iter::once(&(ev.end(), ev.end()))) {
                    if a > from {
                        let r =
                            Rect::from_x_y_ranges(g.x(from as f64)..=g.x(a as f64), rect.y_range());
                        painter.rect_filled(r.intersect(rect), 3.0, shade);
                    }
                    from = from.max(b);
                }
            }
        }
    }
    for row in g.visible_rows() {
        if w.project.tracks[row].frozen.is_some() {
            paint_frozen(painter, g, row);
        }
        if w.project.tracks[row].kind == TrackKind::Folder {
            paint_folder_summary(w, painter, g, row);
        }
        if w.project.tracks[row].kind == TrackKind::Marker {
            paint_markers(w, painter, g, row);
        }
        if w.project.tracks[row].kind == TrackKind::Tempo {
            tempo::paint(w, painter, g, row);
        }
        if w.project.tracks[row].kind == TrackKind::Ruler {
            paint_ruler_track(w, painter, g, row);
        }
    }
    automation::paint(w, painter, g);
    paint_recording(w, painter, g);
    paint_time_warp(w, painter, g);
    if let Some((r0, r1, a, b)) = w.range {
        let rect = Rect::from_min_max(
            pos2(g.x(a as f64), g.row_top(r0)),
            pos2(
                g.x(b as f64).max(g.x(a as f64) + 1.0),
                g.row_top(r1) + g.row_height(r1),
            ),
        );
        painter.rect_filled(rect, 0.0, crate::theme::tokens(painter.ctx()).selection);
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, crate::theme::tokens(painter.ctx()).accent),
            StrokeKind::Inside,
        );
    }
    if let Some(Drag::ZoomRect { start }) = w.drag
        && let Some(now) = painter.ctx().pointer_interact_pos()
    {
        let t = crate::theme::tokens(painter.ctx());
        let rect = Rect::from_two_pos(start, now);
        painter.rect_filled(rect, 0.0, t.selection);
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
    }
    paint_drag_preview(w, painter, g);
}

/// Während der Aufnahme wächst auf jeder aufnahmebereiten Spur ein roter Bereich mit.
fn paint_recording(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let Some(t) = w.transport.as_ref().filter(|_| w.recording) else {
        return;
    };
    let start = t.position(t.status.rec_start()).0;
    let from = if w.cursor >= start {
        start
    } else {
        w.left_locator
    };
    for row in g.visible_rows() {
        let tr = &w.project.tracks[row];
        if !(tr.record && tr.kind == TrackKind::Audio) {
            continue;
        }
        let top = g.row_top(row) + 2.0;
        let rect = Rect::from_min_max(
            pos2(g.x(from as f64), top),
            pos2(
                g.x(w.cursor as f64).max(g.x(from as f64) + 1.0),
                top + g.view.row_h - 4.0,
            ),
        );
        painter.rect_filled(
            rect,
            3.0,
            crate::theme::tokens(painter.ctx()).rec.gamma_multiply(0.55),
        );
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(1.0, crate::theme::tokens(painter.ctx()).rec),
            StrokeKind::Inside,
        );
    }
}

/// Ein Event: farbige Namensleiste oben, darunter der Inhalt (bei Audio die Wellenform).
/// Eingefrorene Spur: Events abgedunkelt und gesperrt, mit Hinweis am linken Rand.
fn paint_frozen(painter: &Painter, g: &Geo, row: usize) {
    let tk = crate::theme::tokens(painter.ctx());
    let top = g.row_top(row);
    let r = Rect::from_x_y_ranges(g.lanes.x_range(), top..=top + g.view.row_h);
    // Halbdurchsichtig in Hintergrundfarbe: Events bleiben zu sehen, wirken aber gesperrt.
    painter.rect_filled(r.intersect(g.lanes), 0.0, tk.window_bg.gamma_multiply(0.6));
    painter.text(
        pos2(g.lanes.left() + 6.0, top + g.view.row_h - 4.0),
        egui::Align2::LEFT_BOTTOM,
        tr("Eingefroren"),
        egui::FontId::proportional(12.0),
        tk.text,
    );
}

fn paint_event(
    w: &ProjectWindow,
    painter: &Painter,
    g: &Geo,
    track: &Track,
    ev: &Event,
    rect: Rect,
) {
    const HEAD: f32 = 13.0;
    let color = if ev.muted {
        crate::theme::tokens(painter.ctx()).event_muted
    } else {
        track_color(ev.color_on(track))
    };
    let body_fill = if ev.muted {
        crate::theme::tokens(painter.ctx())
            .event_muted
            .lerp_to_gamma(crate::theme::tokens(painter.ctx()).event_shade, 0.6)
    } else {
        color.lerp_to_gamma(crate::theme::tokens(painter.ctx()).event_shade, 0.62)
    };
    painter.rect_filled(rect, 3.0, body_fill);
    let head = Rect::from_min_max(
        rect.min,
        pos2(rect.right(), (rect.top() + HEAD).min(rect.bottom())),
    );
    painter.rect_filled(
        head,
        egui::CornerRadius {
            nw: 3,
            ne: 3,
            sw: 0,
            se: 0,
        },
        color.gamma_multiply(if ev.muted { 0.6 } else { 1.0 }),
    );

    let visible = rect.intersect(g.lanes);
    let body = Rect::from_min_max(pos2(rect.left(), head.bottom()), rect.max);
    if !ev.notes.is_empty() {
        paint_notes(
            painter,
            g,
            ev,
            body.intersect(g.lanes),
            color.lerp_to_gamma(Color32::WHITE, 0.45),
        );
    }
    if track.kind == TrackKind::Video {
        crate::video::paint(w, painter, track, ev, rect, body.intersect(g.lanes));
    } else if let Some(clip) = ev.clip.and_then(|c| w.clip(c)) {
        let wave = if ev.muted {
            crate::theme::tokens(painter.ctx())
                .event_text_muted
                .gamma_multiply(0.7)
        } else {
            color.lerp_to_gamma(Color32::WHITE, 0.35)
        };
        paint_waveform(w, painter, g, ev, clip, body.intersect(g.lanes), wave);
    }
    // Audio-Part: die Wellenformen seiner Events, auf den Part zugeschnitten (Issue #184).
    if ev.is_audio_part() {
        let wave = if ev.muted {
            crate::theme::tokens(painter.ctx())
                .event_text_muted
                .gamma_multiply(0.7)
        } else {
            color.lerp_to_gamma(Color32::WHITE, 0.35)
        };
        let area = body.intersect(g.lanes);
        for c in w.project.part_children(ev) {
            let Some(clip) = c.clip.and_then(|id| w.clip(id)) else {
                continue;
            };
            let x0 = g.x(c.start.0 as f64).max(area.left());
            let x1 = g.x(c.end() as f64).min(area.right());
            if x1 <= x0 {
                continue;
            }
            let r = Rect::from_x_y_ranges(x0..=x1, area.y_range());
            paint_waveform(w, painter, g, &c, clip, r, wave);
            painter.vline(x0, r.y_range(), Stroke::new(1.0, wave.gamma_multiply(0.5)));
        }
    }
    let selected = w.selection.contains(&ev.id);
    paint_fades(painter, g, ev, body, selected);

    let stroke = if selected {
        Stroke::new(2.0, crate::theme::tokens(painter.ctx()).event_selected)
    } else {
        Stroke::new(1.0, color.gamma_multiply(0.8))
    };
    painter.rect_stroke(rect, 3.0, stroke, StrokeKind::Inside);

    if visible.width() > 24.0 {
        // Name am sichtbaren linken Rand festhalten, auch wenn das Event links hinausragt.
        painter
            .with_clip_rect(head.intersect(visible).shrink2(egui::vec2(3.0, 0.0)))
            .text(
                pos2(visible.left() + 5.0, head.center().y),
                Align2::LEFT_CENTER,
                // Geteilte Kopien: „=“ vor dem Namen.
                if ev.shared.is_some() {
                    format!("= {}", ev.name)
                } else {
                    ev.name.clone()
                },
                FontId::proportional(10.5),
                if ev.muted {
                    crate::theme::tokens(painter.ctx()).event_text_muted
                } else {
                    crate::theme::tokens(painter.ctx()).event_text
                },
            );
    }
}

/// Mini-Noten in einem MIDI-Part: Tonhöhenbereich der Noten auf die Höhe verteilt.
fn paint_notes(painter: &Painter, g: &Geo, ev: &Event, area: Rect, color: Color32) {
    if area.height() < 4.0 || area.width() < 1.0 {
        return;
    }
    let lo = ev.notes.iter().map(|n| n.pitch).min().unwrap_or(60).min(84) as f32;
    let hi = ev
        .notes
        .iter()
        .map(|n| n.pitch)
        .max()
        .unwrap_or(60)
        .max(lo as u8 + 12) as f32;
    let step = (area.height() - 2.0) / (hi - lo + 1.0);
    let len = ev.length as i64;
    for n in ev.notes.iter().filter(|n| n.start < len && n.end() > 0) {
        let a = ev.start.0 as f64 + n.start.max(0) as f64;
        let b = ev.start.0 as f64 + n.end().min(len) as f64;
        let y = area.bottom() - 1.0 - (f32::from(n.pitch) - lo + 1.0) * step;
        let r = Rect::from_min_max(
            pos2(g.x(a), y),
            pos2(g.x(b).max(g.x(a) + 2.0), y + step.clamp(1.5, 4.0)),
        );
        // Nur Sichtbares: ein langer Part hat zehntausende Noten.
        if r.intersects(area) {
            painter.rect_filled(r.intersect(area), 0.5, color);
        }
    }
}

/// Faktor aus Fades (mit Kurvenform) und Event-Lautstärke an Tick `k` relativ zum
/// Event-Anfang, wie in der Engine.
fn fade_gain(ev: &Event, k: f64) -> f32 {
    let mut g = studiocraft_project::db_to_gain(ev.gain_db);
    if ev.fade_in > 0 && k < ev.fade_in as f64 {
        g *= ev.fade_in_curve.shape((k / ev.fade_in as f64) as f32);
    }
    let rest = ev.length as f64 - k;
    if ev.fade_out > 0 && rest < ev.fade_out as f64 {
        g *= ev.fade_out_curve.shape((rest / ev.fade_out as f64) as f32);
    }
    g
}

/// Punkte einer Fade-Kurve von `x0` (stumm) nach `x1` (voll) bzw. umgekehrt.
fn curve_points(curve: FadeCurve, x0: f32, x1: f32, top: f32, bottom: f32) -> Vec<Pos2> {
    const STEPS: usize = 16;
    (0..=STEPS)
        .map(|i| {
            let t = i as f32 / STEPS as f32;
            pos2(x0 + (x1 - x0) * t, bottom - curve.shape(t) * (bottom - top))
        })
        .collect()
}

/// Abgedunkelter Bereich über einer Kurve, in konvexen Streifen.
fn shade_above(painter: &Painter, pts: &[Pos2], top: f32, color: Color32) {
    for w in pts.windows(2) {
        let quad = vec![pos2(w[0].x, top), pos2(w[1].x, top), w[1], w[0]];
        painter.add(egui::Shape::convex_polygon(quad, color, Stroke::NONE));
    }
}

/// Fade-Bereiche abdunkeln, Kurve zeichnen, Griffe am ausgewählten Event.
fn paint_fades(painter: &Painter, g: &Geo, ev: &Event, body: Rect, selected: bool) {
    let shade = Color32::from_black_alpha(120);
    let line = Stroke::new(1.2, Color32::from_white_alpha(200));
    let (top, bottom) = (body.top(), body.bottom());
    let fin = g.x((ev.start.0 + ev.fade_in) as f64);
    let fout = g.x((ev.end() - ev.fade_out) as f64);
    if ev.fade_in > 0 {
        let pts = curve_points(ev.fade_in_curve, body.left(), fin, top, bottom);
        shade_above(painter, &pts, top, shade);
        painter.add(egui::Shape::line(pts, line));
    }
    if ev.fade_out > 0 {
        let pts = curve_points(ev.fade_out_curve, body.right(), fout, top, bottom);
        shade_above(painter, &pts, top, shade);
        painter.add(egui::Shape::line(pts, line));
    }
    // Lautstärkegriff oben in der Mitte (Audio-Events), mit Wert, wenn nicht 0 dB.
    if (ev.clip.is_some() || ev.is_audio_part()) && body.width() > EDGE * 6.0 {
        let t = crate::theme::tokens(painter.ctx());
        let c = pos2(body.center().x, top + 3.0);
        painter.circle_filled(c, 3.0, t.handle);
        painter.circle_stroke(c, 3.0, Stroke::new(1.0, t.border));
        if ev.gain_db != 0.0 {
            let text = if ev.gain_db <= MIN_DB {
                tr("stumm").to_owned()
            } else {
                format!("{:+.1} dB", ev.gain_db)
            };
            painter.text(
                c + egui::vec2(6.0, 0.0),
                egui::Align2::LEFT_CENTER,
                text,
                egui::FontId::proportional(10.0),
                t.text,
            );
        }
    }
    if selected {
        for x in [fin, fout] {
            let handle = Rect::from_center_size(pos2(x, top + 1.0), egui::vec2(7.0, 7.0));
            painter.rect_filled(handle, 1.0, crate::theme::tokens(painter.ctx()).handle);
            painter.rect_stroke(
                handle,
                1.0,
                Stroke::new(1.0, crate::theme::tokens(painter.ctx()).border),
                StrokeKind::Inside,
            );
        }
    }
}

fn paint_waveform(
    w: &ProjectWindow,
    painter: &Painter,
    g: &Geo,
    ev: &Event,
    clip: &crate::LoadedClip,
    area: Rect,
    color: Color32,
) {
    if area.width() < 1.0 || area.height() < 4.0 {
        return;
    }
    let tempo = &w.project.tempo;
    let rate = f64::from(w.sample_rate);
    // Clip-Frame zu einer Tick-Position (über Sekunden, damit Tempowechsel stimmen).
    let start_s = tempo.seconds(studiocraft_project::Position(ev.start.0));
    // Folgt das Event dem Tempo, zählt der Inhalt nach seiner Abbildung (Tempo der Datei,
    // Warp-Marker; Issue #55).
    let map = ev
        .content_map(w.sample_rate)
        .map(|m| (m.tick_at(ev.offset as f64) - ev.start.0 as f64, m));
    let frame_at = |t: f64| match &map {
        Some((origin, m)) => m.frame_at(origin + t),
        None => {
            let s = tempo.seconds(studiocraft_project::Position(t.max(0.0) as u64));
            ev.offset as f64 + (s - start_s) * rate
        }
    };
    let mid = area.center().y;
    let half = area.height() * 0.5 - 1.0;
    // In echten Bildschirmpixeln zeichnen, als lückenloses Mesh aus Spalten: Linien an
    // halben Punkten würden bei nicht ganzzahliger Skalierung (z. B. 1,25) die Pixel
    // schneiden und durch die Kantenglättung senkrechte Streifen bilden.
    let ppp = painter.ctx().pixels_per_point().max(0.1);
    let step = 1.0 / ppp;
    let snap = |v: f32| (v * ppp).round() / ppp;
    let mut mesh = egui::Mesh::default();
    let mut x = snap(area.left());
    while x < area.right() {
        let x1 = (x + step).min(area.right());
        let f0 = frame_at(g.ticks(x));
        let f1 = frame_at(g.ticks(x1));
        let (lo, hi) = clip.peaks.range(f0, f1.max(f0 + 1.0));
        let fade = fade_gain(ev, g.ticks((x + x1) * 0.5) - ev.start.0 as f64);
        let (lo, hi) = (lo * fade, hi * fade);
        let y0 = snap(mid - hi.clamp(-1.0, 1.0) * half);
        let y1 = snap(mid - lo.clamp(-1.0, 1.0) * half).max(y0 + step);
        mesh.add_colored_rect(Rect::from_min_max(pos2(x, y0), pos2(x1, y1)), color);
        x += step;
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_drag_preview(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let Some(p) = painter.ctx().input(|i| i.pointer.interact_pos()) else {
        return;
    };
    let (alt, ctrl) = painter
        .ctx()
        .input(|i| (i.modifiers.alt, i.modifiers.command));
    let ghost = Stroke::new(1.5, Color32::from_white_alpha(200));
    match w.drag {
        Some(Drag::Repeat { start, end }) => {
            let n = repeat_count(g, start, end, p);
            let span = end - start;
            for k in 1..=u64::from(n) {
                for &sel in &w.selection {
                    let (Some((t, _)), Some(ev)) = (w.project.find(sel), w.project.event(sel))
                    else {
                        continue;
                    };
                    let s = (ev.start.0 + span * k) as f64;
                    let top = g.row_top(t) + 2.0;
                    let rect = Rect::from_min_max(
                        pos2(g.x(s), top),
                        pos2(g.x(s + ev.length as f64), top + g.view.row_h - 4.0),
                    );
                    painter.rect_stroke(rect, 3.0, ghost, StrokeKind::Inside);
                }
            }
        }
        Some(Drag::Move {
            id,
            grab,
            row,
            copy,
            ..
        }) => {
            let Some((drow, dt)) = move_delta(w, g, id, grab, row, p, alt) else {
                return;
            };
            let (drow, dt) = if ctrl && !copy {
                lock_direction(g, drow, dt)
            } else {
                (drow, dt)
            };
            for &sel in &w.selection {
                let (Some((t, _)), Some(ev)) = (w.project.find(sel), w.project.event(sel)) else {
                    continue;
                };
                let target = g.offset_row(t, drow);
                let start = (ev.start.0 as i64 + dt).max(0) as f64;
                let top = g.row_top(target) + 2.0;
                let rect = Rect::from_min_max(
                    pos2(g.x(start), top),
                    pos2(g.x(start + ev.length as f64), top + g.view.row_h - 4.0),
                );
                painter.rect_stroke(rect, 3.0, ghost, StrokeKind::Inside);
            }
        }
        Some(Drag::Comp { row, id, start }) => {
            let end = w.snap_ticks(g.ticks(p.x));
            let lane = w.project.tracks.get(row).map_or(0, |t| lane_of(t, id));
            let top = g.row_top(row) + lane as f32 * g.view.row_h + 2.0;
            let rect = Rect::from_x_y_ranges(
                g.x(start.min(end) as f64)..=g.x(start.max(end) as f64),
                top..=top + g.view.row_h - 4.0,
            );
            let accent = crate::theme::tokens(painter.ctx()).accent;
            painter.rect_stroke(rect, 3.0, Stroke::new(2.0, accent), StrokeKind::Inside);
        }
        Some(Drag::Draw { row, start }) => {
            let end = w.snap_ticks(g.ticks(p.x));
            let top = g.row_top(row) + 2.0;
            let rect = Rect::from_x_y_ranges(
                g.x(start.min(end) as f64)..=g.x(start.max(end) as f64),
                top..=top + g.view.row_h - 4.0,
            );
            painter.rect_stroke(rect, 3.0, ghost, StrokeKind::Inside);
        }
        _ => {}
    }
}

fn paint_ruler(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let visuals = painter.ctx().global_style().visuals.clone();
    painter.rect_filled(g.ruler, 0.0, visuals.faint_bg_color);
    let band = Rect::from_x_y_ranges(
        g.x(w.left_locator as f64)..=g.x(w.right_locator as f64),
        g.ruler.top() + 2.0..=g.ruler.top() + 10.0,
    );
    let col = if w.cycle {
        crate::theme::tokens(painter.ctx()).accent
    } else {
        crate::theme::tokens(painter.ctx()).ruler_tick
    };
    painter.rect_filled(band, 2.0, col);
    paint_grid(w, painter, g, g.ruler, true);
}

fn paint_headers(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let visuals = painter.ctx().global_style().visuals.clone();
    painter.rect_filled(g.header, 0.0, visuals.panel_fill);
    for row in g.visible_rows() {
        let track = &w.project.tracks[row];
        let top = g.row_top(row);
        let rect = Rect::from_min_size(
            pos2(g.header.left(), top),
            vec2(g.header.width(), g.row_height(row)),
        );
        let t = crate::theme::tokens(painter.ctx());
        if w.selected_track == Some(row) {
            // Gewählte Spur: heller hinterlegt, nicht vollflächig in Akzentfarbe (ui-parity.md).
            painter.rect_filled(rect, 0.0, t.row_selected);
        }
        // Farbleiste über die ganze Höhe, oben die Spurnummer.
        let color = track_color(track.color);
        let strip = Rect::from_min_size(rect.min, vec2(STRIP_W, rect.height()));
        painter.rect_filled(strip, 0.0, color);
        painter.with_clip_rect(strip.intersect(g.header)).text(
            pos2(strip.center().x, g.header_line(row, 0) + 9.0),
            Align2::CENTER_CENTER,
            (g.display_pos(row).unwrap_or(row) + 1).to_string(),
            FontId::proportional(10.0),
            contrast_text(color),
        );
        let [_, solo, ..] = g.header_buttons(row);
        let volume = g.header_volume(row);
        let channel = crate::mixer::has_channel(track.kind);
        // Name hinter Mute/Solo, nur bis vor den Lautstärkeregler.
        let name_right = if channel {
            volume.left() - 6.0
        } else {
            rect.right() - 6.0
        };
        let name_clip =
            Rect::from_x_y_ranges(solo.right() + 6.0..=name_right, rect.top()..=rect.bottom());
        painter.with_clip_rect(name_clip.intersect(g.header)).text(
            pos2(name_clip.left(), solo.center().y),
            Align2::LEFT_CENTER,
            &track.name,
            FontId::proportional(13.0),
            t.text,
        );
        // Spurtyp unten rechts, klein und gedimmt.
        painter.text(
            pos2(rect.right() - 6.0, g.header_line(row, 1) + 9.0),
            Align2::RIGHT_CENTER,
            tr(track.kind.label()),
            FontId::proportional(10.0),
            visuals.weak_text_color(),
        );
        if channel {
            paint_header_volume(painter, volume, track.volume_db, color, &visuals);
        }
        let flags = [
            // Lineal-Spuren klingen nicht: kein Mute/Solo.
            (
                Flag::Text("M"),
                track.mute,
                t.mute,
                track.kind != TrackKind::Ruler,
            ),
            (
                Flag::Text("S"),
                track.solo,
                t.solo,
                track.kind != TrackKind::Ruler,
            ),
            (
                Flag::Icon(Icon::Record),
                track.record,
                t.rec,
                track.kind.holds_events(),
            ),
            (
                Flag::Icon(Icon::Play),
                track.monitor,
                t.monitor,
                track.kind == TrackKind::Audio,
            ),
            (Flag::Text("R"), track.read, t.monitor, channel),
            (Flag::Text("W"), track.write, t.rec, channel),
            (
                Flag::Icon(match track.time_base() {
                    TimeBase::Musical => Icon::TimeMusical,
                    TimeBase::Linear => Icon::TimeLinear,
                }),
                false,
                t.accent,
                track.kind.holds_events(),
            ),
            (
                Flag::Icon(Icon::Lanes),
                track.show_lanes,
                t.accent,
                track.kind.holds_events(),
            ),
        ];
        for (b, (flag, on, color, shown)) in g.header_buttons(row).into_iter().zip(flags) {
            if !shown {
                continue;
            }
            let fill = if on {
                color
            } else {
                visuals.widgets.inactive.bg_fill
            };
            painter.rect_filled(b, 3.0, fill);
            match flag {
                Flag::Icon(icon) => {
                    // Aufnahme bereit (Punkt) und Monitor (Lautsprecher) als Symbol.
                    let fg = if on {
                        Color32::WHITE
                    } else if icon == Icon::Record {
                        color
                    } else {
                        visuals.text_color()
                    };
                    crate::icons::paint(painter, b.shrink(4.0), icon, fg);
                }
                Flag::Text(label) => {
                    let text = if on {
                        Color32::BLACK
                    } else {
                        visuals.text_color()
                    };
                    painter.text(
                        b.center(),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(11.0),
                        text,
                    );
                }
            }
        }
        // Solo-Defeat: gelber Rahmen um S.
        if track.solo_defeat {
            let s = g.header_buttons(row)[1];
            painter.rect_stroke(s, 3.0, Stroke::new(1.5, t.solo), StrokeKind::Inside);
        }
        automation::paint_headers(w, painter, g, row);
        painter.hline(
            rect.x_range(),
            rect.bottom(),
            Stroke::new(1.0, visuals.faint_bg_color),
        );
    }
    paint_track_drop(w, painter, g);
}

/// Während eine Spur gezogen wird: Einfügelinie bzw. Rahmen um den Zielordner.
fn paint_track_drop(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let Some(Drag::TrackMove { row }) = w.drag else {
        return;
    };
    let Some(p) = painter.ctx().pointer_interact_pos() else {
        return;
    };
    let Some((before, folder)) = track_drop(w, g, row, p) else {
        return;
    };
    let accent = crate::theme::tokens(painter.ctx()).accent;
    let x = g.header.x_range();
    match (before, folder) {
        (None, Some(f)) => {
            let r = Rect::from_x_y_ranges(x, g.row_top(f)..=g.row_top(f) + g.view.row_h);
            painter.rect_stroke(
                r.shrink(1.0),
                2.0,
                Stroke::new(2.0, accent),
                StrokeKind::Inside,
            );
        }
        (Some(b), _) => {
            painter.hline(x, g.row_top(b), Stroke::new(2.0, accent));
        }
        (None, None) => {
            painter.hline(x, g.content_bottom(), Stroke::new(2.0, accent));
        }
    }
}

/// Lautstärkeregler im Spurkopf: Schiene, gefüllter Teil in Spurfarbe, Griff.
fn paint_header_volume(
    painter: &Painter,
    r: Rect,
    db: f32,
    color: Color32,
    visuals: &egui::Visuals,
) {
    let y = r.center().y;
    let pos = crate::mixer::db_to_pos(db).clamp(0.0, 1.0);
    let x = r.left() + pos * r.width();
    let rail = Rect::from_x_y_ranges(r.x_range(), y - 1.5..=y + 1.5);
    painter.rect_filled(rail, 1.5, visuals.widgets.inactive.bg_fill);
    painter.rect_filled(
        Rect::from_x_y_ranges(r.left()..=x, rail.y_range()),
        1.5,
        color.gamma_multiply(0.8),
    );
    painter.rect_filled(
        Rect::from_center_size(pos2(x, y), vec2(6.0, 12.0)),
        2.0,
        visuals.widgets.active.fg_stroke.color,
    );
}

/// Lesbare Schriftfarbe auf einer Spurfarbe.
fn contrast_text(bg: Color32) -> Color32 {
    let l = 0.299 * f32::from(bg.r()) + 0.587 * f32::from(bg.g()) + 0.114 * f32::from(bg.b());
    if l > 140.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// Beschriftung eines Spurkopf-Schalters.
enum Flag {
    Text(&'static str),
    Icon(Icon),
}

fn paint_cursor(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let x = g.x(w.cursor as f64);
    let y = g.ruler.top()..=g.lanes.bottom();
    painter.vline(
        x,
        y,
        Stroke::new(1.5, crate::theme::tokens(painter.ctx()).playhead),
    );
}
