//! The light that carries a prompt from the Commander to a pane.
//!
//! A star flies along a curve from the Commander's box to the middle of the
//! pane being prompted. It never draws characters directly: it leaves light in
//! every cell it passes, and that light fades on its own, so the trail is just
//! the light the star left behind, still fading. Where the light is hottest the
//! cell's character is replaced by a stroke that follows the direction of
//! travel; elsewhere the character stays and only its colour is lit. When the
//! star lands it throws sparks and a ripple, and that landing is the moment the
//! prompt is delivered.
//!
//! All motion happens in "visual space", where a cell is 1 unit wide and 2 units
//! tall, because a terminal cell is roughly twice as tall as it is wide and
//! curves only look round in a space with square units. Only drawing converts
//! back to rows and columns.
//!
//! This is pure data and arithmetic. The app advances it on its frame clock and
//! the UI draws it over whatever is already in the frame.

use std::ops::{Add, Mul, Sub};

use ratatui::{buffer::Buffer, layout::Rect, style::Color};

const CELL_ASPECT: f32 = 2.0;
/// Distance between light deposits along the path, in visual units.
const STAMP_STEP: f32 = 0.25;
/// Sparks shed per visual unit travelled.
const SPARKS_PER_UNIT: f32 = 0.35;
/// How far the curve bows out, as a fraction of the straight-line distance.
const BOW: f32 = 0.32;
const RIPPLE_LIFE: f32 = 0.6;
const RIPPLE_RADIUS: f32 = 7.0;
/// Below this much light a cell is drawn as if the trail were not there, and a
/// trail with no cell above it is finished.
const DARK: f32 = 0.01;
const ARC_SAMPLES: usize = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct V2 {
    pub x: f32,
    pub y: f32,
}

impl V2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Visual-space centre of a terminal cell.
    pub fn from_cell(col: u16, row: u16) -> Self {
        Self::new(col as f32 + 0.5, (row as f32 + 0.5) * CELL_ASPECT)
    }

    /// Visual-space centre of a rectangle of cells.
    pub fn center_of(rect: Rect) -> Self {
        Self::new(
            rect.x as f32 + rect.width as f32 / 2.0,
            (rect.y as f32 + rect.height as f32 / 2.0) * CELL_ASPECT,
        )
    }

    fn len(self) -> f32 {
        self.x.hypot(self.y)
    }

    fn norm(self) -> Self {
        let l = self.len();
        if l > 1e-6 {
            self * (1.0 / l)
        } else {
            Self::new(1.0, 0.0)
        }
    }

    /// Rotated 90° counter-clockwise (in screen space, where y points down).
    fn perp(self) -> Self {
        Self::new(self.y, -self.x)
    }

    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

impl Add for V2 {
    type Output = V2;
    fn add(self, o: V2) -> V2 {
        V2::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for V2 {
    type Output = V2;
    fn sub(self, o: V2) -> V2 {
        V2::new(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f32> for V2 {
    type Output = V2;
    fn mul(self, k: f32) -> V2 {
        V2::new(self.x * k, self.y * k)
    }
}

/// A quadratic Bézier curve that can be walked by distance rather than by `t`.
///
/// Bézier `t` does not advance at constant speed: the curve bunches up where it
/// bends. Easing describes how *distance* changes over time, so the curve keeps
/// a table of cumulative length at evenly spaced `t` values and inverts it on
/// demand.
#[derive(Debug, Clone)]
struct Arc {
    p0: V2,
    p1: V2,
    p2: V2,
    lengths: Vec<f32>,
}

impl Arc {
    fn new(p0: V2, p1: V2, p2: V2) -> Self {
        let mut arc = Self {
            p0,
            p1,
            p2,
            lengths: Vec::with_capacity(ARC_SAMPLES + 1),
        };
        let mut total = 0.0;
        let mut prev = p0;
        arc.lengths.push(0.0);
        for i in 1..=ARC_SAMPLES {
            let p = arc.point(i as f32 / ARC_SAMPLES as f32);
            total += (p - prev).len();
            arc.lengths.push(total);
            prev = p;
        }
        arc
    }

    fn len(&self) -> f32 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    fn point(&self, t: f32) -> V2 {
        let u = 1.0 - t;
        self.p0 * (u * u) + self.p1 * (2.0 * u * t) + self.p2 * (t * t)
    }

    fn tangent(&self, t: f32) -> V2 {
        ((self.p1 - self.p0) * (1.0 - t) + (self.p2 - self.p1) * t).norm()
    }

    /// The `t` at which the curve has travelled distance `s`.
    fn t_at(&self, s: f32) -> f32 {
        let s = s.clamp(0.0, self.len());
        let i = self
            .lengths
            .partition_point(|&l| l < s)
            .clamp(1, ARC_SAMPLES);
        let (a, b) = (self.lengths[i - 1], self.lengths[i]);
        let frac = if b > a { (s - a) / (b - a) } else { 0.0 };
        (i as f32 - 1.0 + frac) / ARC_SAMPLES as f32
    }
}

/// Slow start, fast middle, slow arrival.
fn ease_in_out_cubic(u: f32) -> f32 {
    if u < 0.5 {
        4.0 * u * u * u
    } else {
        1.0 - (-2.0 * u + 2.0).powi(3) / 2.0
    }
}

fn ease_out_cubic(u: f32) -> f32 {
    1.0 - (1.0 - u).powi(3)
}

/// A per-cell light buffer that everything bright draws into. Every layer loses
/// light exponentially over time.
#[derive(Debug, Clone)]
struct Layer {
    /// Light per cell, 0.0 to 1.0.
    value: Vec<f32>,
    /// Direction of travel of whatever last brightened the cell.
    dir: Vec<V2>,
    /// Vertical position within the cell (0 = top, 1 = bottom) of that deposit.
    sub_y: Vec<f32>,
    /// Seconds for light to fall to ~37%.
    tau: f32,
}

impl Layer {
    fn new(tau: f32) -> Self {
        Self {
            value: Vec::new(),
            dir: Vec::new(),
            sub_y: Vec::new(),
            tau,
        }
    }

    fn resize(&mut self, n: usize) {
        self.value = vec![0.0; n];
        self.dir = vec![V2::new(1.0, 0.0); n];
        self.sub_y = vec![0.5; n];
    }

    fn decay(&mut self, dt: f32) {
        let k = (-dt / self.tau).exp();
        self.value.iter_mut().for_each(|v| *v *= k);
    }

    fn lit(&self) -> bool {
        self.value.iter().any(|v| *v >= DARK)
    }
}

#[derive(Debug, Clone, Copy)]
enum Target {
    Heat,
    Glow,
}

#[derive(Debug, Clone)]
struct Field {
    w: u16,
    h: u16,
    /// Short-lived, hot light: the star and its immediate trail.
    heat: Layer,
    /// Long-lived, dim light: the ghost of the path.
    glow: Layer,
}

impl Field {
    fn new() -> Self {
        Self {
            w: 0,
            h: 0,
            heat: Layer::new(0.14),
            glow: Layer::new(0.9),
        }
    }

    fn resize(&mut self, w: u16, h: u16) {
        if (w, h) == (self.w, self.h) {
            return;
        }
        self.w = w;
        self.h = h;
        let n = w as usize * h as usize;
        self.heat.resize(n);
        self.glow.resize(n);
    }

    fn index(&self, p: V2) -> Option<usize> {
        let col = p.x.floor();
        let row = (p.y / CELL_ASPECT).floor();
        if col < 0.0 || row < 0.0 || col >= self.w as f32 || row >= self.h as f32 {
            return None;
        }
        Some(row as usize * self.w as usize + col as usize)
    }

    /// Deposit light at `p`. The cell containing `p` receives the full `amp`,
    /// which guarantees a connected core line; nearby cells receive a halo that
    /// falls off with visual distance.
    fn stamp(
        &mut self,
        target: Target,
        p: V2,
        dir: V2,
        amp: f32,
        halo_radius: f32,
        halo_gain: f32,
    ) {
        if self.w == 0 || self.h == 0 {
            return;
        }
        let (w, h) = (self.w as i32, self.h as i32);
        let layer = match target {
            Target::Heat => &mut self.heat,
            Target::Glow => &mut self.glow,
        };
        let core_col = p.x.floor() as i32;
        let core_row = (p.y / CELL_ASPECT).floor() as i32;
        let r_cols = halo_radius.ceil() as i32;
        let r_rows = (halo_radius / CELL_ASPECT).ceil() as i32;
        for row in (core_row - r_rows).max(0)..=(core_row + r_rows).min(h - 1) {
            for col in (core_col - r_cols).max(0)..=(core_col + r_cols).min(w - 1) {
                let weight = if (col, row) == (core_col, core_row) {
                    amp
                } else {
                    let d = (V2::from_cell(col as u16, row as u16) - p).len();
                    let f = (1.0 - d / halo_radius).max(0.0);
                    amp * halo_gain * f * f
                };
                let i = row as usize * w as usize + col as usize;
                if weight > layer.value[i] {
                    layer.value[i] = weight;
                    layer.dir[i] = dir;
                    layer.sub_y[i] = (p.y / CELL_ASPECT).fract();
                }
            }
        }
    }
}

/// A loose spark that drifts away from the star and burns out.
#[derive(Debug, Clone)]
struct Particle {
    pos: V2,
    vel: V2,
    age: f32,
    life: f32,
    /// Starting brightness, 0.0 to 1.0.
    heat: f32,
}

impl Particle {
    fn intensity(&self) -> f32 {
        let left = 1.0 - self.age / self.life;
        self.heat * left * left
    }
}

const DRAG: f32 = 3.0;
/// Visual units per second², pointing down: sparks sag slightly as they cool.
const GRAVITY: f32 = 6.0;

/// Small xorshift generator. Enough randomness for sparks, no dependency.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    /// Uniform in [0, 1).
    fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f()
    }
}

#[derive(Debug, Clone)]
struct Flight {
    arc: Arc,
    elapsed: f32,
    duration: f32,
    /// Distance along the arc already deposited into the field.
    travelled: f32,
}

impl Flight {
    fn progress(&self) -> f32 {
        (self.elapsed / self.duration).min(1.0)
    }

    fn distance(&self) -> f32 {
        ease_in_out_cubic(self.progress()) * self.arc.len()
    }

    fn head(&self) -> V2 {
        self.arc.point(self.arc.t_at(self.distance()))
    }
}

/// An expanding ring of light where the star lands.
#[derive(Debug, Clone)]
struct Ripple {
    center: V2,
    age: f32,
}

/// One trip of the star, and the light it leaves behind until that has faded.
#[derive(Debug, Clone)]
pub struct Trail {
    field: Field,
    particles: Vec<Particle>,
    ripples: Vec<Ripple>,
    rng: Rng,
    flight: Option<Flight>,
    target: V2,
    time: f32,
}

impl Trail {
    /// A star leaving `from` for `to`, inside a frame `bounds` cells across.
    pub fn new(from: V2, to: V2, bounds: Rect, seed: u64) -> Self {
        let mut field = Field::new();
        field.resize(bounds.width, bounds.height);
        let chord = to - from;
        let dist = chord.len().max(1.0);
        // Bow upward, like something thrown. A near-vertical chord has no "up"
        // side, so bow toward the middle of the frame instead.
        let mut side = chord.norm().perp();
        let b = V2::new(bounds.width as f32, bounds.height as f32 * CELL_ASPECT);
        if side.y.abs() < 0.2 {
            let toward_centre = b.x / 2.0 - from.x;
            if side.x * toward_centre < 0.0 {
                side = side * -1.0;
            }
        } else if side.y > 0.0 {
            side = side * -1.0;
        }
        let control = from.lerp(to, 0.5) + side * (dist * BOW);
        // Keeping the control point on screen keeps the whole curve on screen.
        let control = V2::new(
            control.x.clamp(0.5, (b.x - 0.5).max(0.5)),
            control.y.clamp(1.0, (b.y - 1.0).max(1.0)),
        );
        let arc = Arc::new(from, control, to);
        let duration = (0.45 + arc.len() / 110.0).clamp(0.5, 1.4);
        Self {
            field,
            particles: Vec::new(),
            ripples: Vec::new(),
            rng: Rng::new(seed),
            flight: Some(Flight {
                arc,
                elapsed: 0.0,
                duration,
                travelled: 0.0,
            }),
            target: to,
            time: 0.0,
        }
    }

    /// Whether the star is still on its way.
    #[cfg(test)]
    pub fn in_flight(&self) -> bool {
        self.flight.is_some()
    }

    /// Whether anything is left to draw: the star, a spark, a ripple, or light
    /// that has not yet faded.
    pub fn is_visible(&self) -> bool {
        self.flight.is_some()
            || !self.particles.is_empty()
            || !self.ripples.is_empty()
            || self.field.heat.lit()
            || self.field.glow.lit()
    }

    /// Advance by `dt` seconds in a frame `w` by `h` cells. Returns whether the
    /// star landed during this step, which happens exactly once.
    pub fn update(&mut self, dt: f32, w: u16, h: u16) -> bool {
        self.time += dt;
        self.field.resize(w, h);
        self.field.heat.decay(dt);
        self.field.glow.decay(dt);
        self.update_particles(dt);
        let landed = self.update_flight(dt);
        self.update_ripples(dt);
        landed
    }

    fn update_particles(&mut self, dt: f32) {
        let drag = (-DRAG * dt).exp();
        for p in &mut self.particles {
            p.vel = p.vel * drag;
            p.vel.y += GRAVITY * dt;
            p.pos = p.pos + p.vel * dt;
            p.age += dt;
        }
        self.particles.retain(|p| p.age < p.life);
    }

    fn update_flight(&mut self, dt: f32) -> bool {
        let Some(f) = &mut self.flight else {
            return false;
        };
        f.elapsed += dt;
        let goal = f.distance();

        // Walk from where the star stopped last frame to where it is now,
        // depositing light at small fixed steps, so the trail stays continuous
        // however fast the star moves or however slow the frame was.
        let mut s = f.travelled;
        let mut deposits = Vec::new();
        while s < goal {
            s = (s + STAMP_STEP).min(goal);
            let t = f.arc.t_at(s);
            deposits.push((f.arc.point(t), f.arc.tangent(t)));
        }
        f.travelled = goal;
        let done = f.progress() >= 1.0;
        let end = f.arc.point(1.0);

        for (p, tan) in deposits {
            self.field.stamp(Target::Heat, p, tan, 1.0, 2.0, 0.55);
            self.field.stamp(Target::Glow, p, tan, 1.0, 1.0, 0.0);
            if self.rng.f() < STAMP_STEP * SPARKS_PER_UNIT {
                let spread = tan.perp() * self.rng.range(-5.0, 5.0);
                let vel = tan * self.rng.range(-8.0, -2.0) + spread;
                let life = self.rng.range(0.25, 0.7);
                let heat = self.rng.range(0.5, 0.95);
                self.particles.push(Particle {
                    pos: p,
                    vel,
                    age: 0.0,
                    life,
                    heat,
                });
            }
        }

        if done {
            self.flight = None;
            self.land(end);
        }
        done
    }

    fn land(&mut self, at: V2) {
        self.ripples.push(Ripple {
            center: at,
            age: 0.0,
        });
        for _ in 0..22 {
            let a = self.rng.range(0.0, std::f32::consts::TAU);
            let speed = self.rng.range(6.0, 22.0);
            let vel = V2::new(a.cos(), a.sin()) * speed;
            let life = self.rng.range(0.35, 0.9);
            let heat = self.rng.range(0.6, 1.0);
            self.particles.push(Particle {
                pos: at,
                vel,
                age: 0.0,
                life,
                heat,
            });
        }
    }

    fn update_ripples(&mut self, dt: f32) {
        let mut rings = Vec::new();
        for r in &mut self.ripples {
            r.age += dt;
            let u = (r.age / RIPPLE_LIFE).min(1.0);
            rings.push((
                r.center,
                ease_out_cubic(u) * RIPPLE_RADIUS,
                0.7 * (1.0 - u) * (1.0 - u),
            ));
        }
        for (center, radius, amp) in rings {
            let n = ((std::f32::consts::TAU * radius) / 0.6).ceil().max(8.0) as usize;
            for k in 0..n {
                let a = k as f32 / n as f32 * std::f32::consts::TAU;
                let outward = V2::new(a.cos(), a.sin());
                self.field.stamp(
                    Target::Heat,
                    center + outward * radius,
                    outward.perp(),
                    amp,
                    1.0,
                    0.0,
                );
            }
        }
        self.ripples.retain(|r| r.age < RIPPLE_LIFE);
    }

    /// Light up whatever is already in `buf`. The rest of the frame has been
    /// drawn first; this only recolours it, and replaces characters where the
    /// trail is hottest.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let w = self.field.w.min(area.width);
        let h = self.field.h.min(area.height);
        let heat = &self.field.heat;
        let glow = &self.field.glow;

        for row in 0..h {
            for col in 0..w {
                let i = row as usize * self.field.w as usize + col as usize;
                let (hv, gv) = (heat.value[i], glow.value[i]);
                if hv < DARK && gv < DARK {
                    continue;
                }
                let cell = &mut buf[(area.x + col, area.y + row)];
                let blank = cell.symbol() == " ";
                let (hot, cool) = (light(HOT, hv), light(COOL, gv));
                let under_fg = from_color(cell.fg, TEXT);
                let under_bg = from_color(cell.bg, BG);

                // The shape comes from whichever layer holds a line here, and
                // the colour is both layers' light added together, so a cooling
                // trail hands over to the afterglow without a visible seam.
                let stroke = if hv > 0.42 {
                    Some(line_glyph(heat.dir[i], heat.sub_y[i]))
                } else if gv > 0.4 && blank {
                    Some(line_glyph(glow.dir[i], glow.sub_y[i]))
                } else if (hv > 0.06 || gv > 0.08) && blank {
                    Some('.')
                } else {
                    None
                };
                match stroke {
                    Some(ch) => {
                        cell.set_char(ch)
                            .set_fg(rgb(add(add(under_bg, hot, 1.0), cool, 1.0)))
                    }
                    None => cell.set_fg(rgb(add(add(under_fg, hot, 1.0), cool, 0.6))),
                };
            }
        }

        for p in &self.particles {
            let Some(i) = self.field.index(p.pos) else {
                continue;
            };
            let e = p.intensity();
            if e < 0.05 || e < heat.value[i] {
                continue;
            }
            // Bright sparks are drawn; faint ones only warm the text they cross.
            match e {
                e if e > 0.6 => put(area, buf, p.pos, Some('*'), lerp_palette(HOT, e)),
                e if e > 0.35 => put(area, buf, p.pos, Some('+'), lerp_palette(HOT, e)),
                _ => put(area, buf, p.pos, None, light(HOT, e * 1.4)),
            }
        }

        if let Some(f) = &self.flight {
            // The destination brightens as the star approaches it.
            let e = 0.25 + 0.5 * f.progress();
            put(area, buf, self.target, Some('o'), lerp_palette(COOL, e));
            let twinkle = 0.9 + 0.1 * (self.time * 17.0).sin();
            put(area, buf, f.head(), Some('*'), lerp_palette(HOT, twinkle));
        }
    }
}

/// Draw `ch` at `p`, or with `None`, add `color` as light to what is there.
fn put(area: Rect, buf: &mut Buffer, p: V2, ch: Option<char>, color: Rgb) {
    if p.x < 0.0 || p.y < 0.0 {
        return;
    }
    let col = p.x as u16;
    let row = (p.y / CELL_ASPECT) as u16;
    if col >= area.width || row >= area.height {
        return;
    }
    let cell = &mut buf[(area.x + col, area.y + row)];
    match ch {
        Some(ch) => cell.set_char(ch).set_fg(rgb(color)),
        None => {
            let under = from_color(cell.fg, TEXT);
            cell.set_fg(rgb(add(under, color, 1.0)))
        }
    };
}

/// Pick the character whose stroke best matches the direction of travel.
///
/// Angles are measured in visual space. Because a cell is twice as tall as it
/// is wide, `/` and `\` drawn in one cell lean at about 63°, not 45°, so the
/// bin boundaries sit where the glyphs actually look.
fn line_glyph(dir: V2, sub_y: f32) -> char {
    let deg = (-dir.y).atan2(dir.x).to_degrees().rem_euclid(180.0);
    match deg {
        d if !(25.0..155.0).contains(&d) => {
            // Near-horizontal strokes use the cell's top, middle or bottom so
            // shallow slopes step down gradually instead of jumping a full row.
            if sub_y < 0.3 {
                '‾'
            } else if sub_y > 0.7 {
                '_'
            } else {
                '-'
            }
        }
        d if d < 70.0 => '/',
        d if d <= 110.0 => '|',
        _ => '\\',
    }
}

type Rgb = (f32, f32, f32);

const BG: Rgb = (8.0, 9.0, 16.0);
const TEXT: Rgb = (165.0, 170.0, 190.0);

/// White-hot core cooling through gold and ember red into violet, then night.
const HOT: &[(f32, Rgb)] = &[
    (0.0, BG),
    (0.12, (60.0, 22.0, 80.0)),
    (0.3, (180.0, 48.0, 84.0)),
    (0.5, (245.0, 120.0, 45.0)),
    (0.75, (255.0, 205.0, 105.0)),
    (1.0, (255.0, 250.0, 238.0)),
];

/// The cold afterglow the path leaves once the heat is gone.
const COOL: &[(f32, Rgb)] = &[
    (0.0, BG),
    (0.3, (22.0, 38.0, 70.0)),
    (0.7, (58.0, 105.0, 160.0)),
    (1.0, (150.0, 195.0, 235.0)),
];

fn lerp_palette(stops: &[(f32, Rgb)], e: f32) -> Rgb {
    let e = e.clamp(0.0, 1.0);
    let i = stops
        .partition_point(|&(k, _)| k < e)
        .clamp(1, stops.len() - 1);
    let ((k0, a), (k1, b)) = (stops[i - 1], stops[i]);
    let t = ((e - k0) / (k1 - k0)).clamp(0.0, 1.0);
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

/// How much a palette colour adds over the background, as light.
fn light(stops: &[(f32, Rgb)], e: f32) -> Rgb {
    let (c, bg) = (lerp_palette(stops, e), stops[0].1);
    (c.0 - bg.0, c.1 - bg.1, c.2 - bg.2)
}

/// `base` with `k` times `light` added, saturating at white.
fn add(base: Rgb, light: Rgb, k: f32) -> Rgb {
    let mix = |x: f32, l: f32| (x + l * k).min(255.0);
    (
        mix(base.0, light.0),
        mix(base.1, light.1),
        mix(base.2, light.2),
    )
}

/// The colour the UI gave a cell, or `fallback` if it is not a plain RGB colour.
fn from_color(c: Color, fallback: Rgb) -> Rgb {
    match c {
        Color::Rgb(r, g, b) => (r as f32, g as f32, b as f32),
        _ => fallback,
    }
}

fn rgb((r, g, b): Rgb) -> Color {
    Color::Rgb(r as u8, g as u8, b as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Rect {
        Rect::new(0, 0, 80, 24)
    }

    #[test]
    fn the_star_lands_exactly_once_and_then_fades_out() {
        let mut trail = Trail::new(V2::from_cell(40, 22), V2::from_cell(10, 5), frame(), 7);
        let mut landings = 0;
        for _ in 0..400 {
            if trail.update(1.0 / 60.0, 80, 24) {
                landings += 1;
            }
        }
        assert_eq!(landings, 1);
        assert!(!trail.in_flight());
        assert!(!trail.is_visible(), "the light should have faded");
    }

    #[test]
    fn the_trail_draws_over_the_frame_while_it_flies() {
        let mut trail = Trail::new(V2::from_cell(40, 22), V2::from_cell(10, 5), frame(), 7);
        for _ in 0..20 {
            trail.update(1.0 / 60.0, 80, 24);
        }
        let mut buf = Buffer::empty(frame());
        trail.render(frame(), &mut buf);
        let touched = buf
            .content
            .iter()
            .filter(|cell| cell.symbol() != " ")
            .count();
        assert!(touched > 0);
    }

    #[test]
    fn a_shrunken_frame_does_not_panic() {
        let mut trail = Trail::new(V2::from_cell(40, 22), V2::from_cell(10, 5), frame(), 3);
        trail.update(0.05, 80, 24);
        trail.update(0.05, 20, 6);
        let small = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(small);
        trail.render(small, &mut buf);
        trail.update(0.05, 0, 0);
        trail.render(
            Rect::new(0, 0, 0, 0),
            &mut Buffer::empty(Rect::new(0, 0, 0, 0)),
        );
    }
}
