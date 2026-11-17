// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's label chips on the bridge (`ROAD-112` rounds V2 and R2-5): each frame builds
//! the chips (kind, tone, text in `road_tool::wording`, size in the HUD's font) into reused
//! buffers and compares them with the last frame's; `road_tool_chips_update` reads the camera,
//! places the chips when they or the view changed and says what the HUD must fetch, so a frame
//! with nothing new crosses no array and redraws nothing. `road_tool_hud.gd` only draws.

use super::*;
use crate::nodes::sim::road_tool::height::ground_marks::push_height_labels;
use crate::nodes::sim::road_tool::labels::{
    LabelKind, LabelTone, ToolLabel, apply_preview, middle_label, place_chips,
};
use crate::nodes::sim::road_tool::wording::{
    CHIP_EXTRA_CHARACTERS, NoticeFacts, ReasonFacts, label_text,
};
use godot::classes::Camera3D;

/// `road_tool_chips_update`'s flag: the chips' kinds, tones, texts or sizes changed since the
/// HUD last fetched them (`road_tool_chips`).
const CHIPS_CHANGED: i64 = 1;
/// `road_tool_chips_update`'s flag: only the chips' corners changed (`road_tool_chip_corners`).
const CORNERS_CHANGED: i64 = 2;

/// The bridge's chips: the font they are sized in, this frame's, the HUD's copy and the
/// placement.
#[derive(Default)]
pub(super) struct Chips {
    // The HUD's chip font (round V2): character advances and the chip's panel.
    font: ChipFont,
    // This frame's chips, the next frame's being built, and the content the HUD last fetched.
    shown: ChipSet,
    next: ChipSet,
    delivered: ChipSet,
    // Bumped when the content (kinds, tones, texts, sizes) changes, and when anything placement
    // reads changes (the content's sizes, the anchors and sides, the handles).
    revision: u64,
    placement: u64,
    // The revision the HUD last fetched, `None` before the first fetch.
    delivered_revision: Option<u64>,
    // The handles the chips keep clear of (the frame's gizmo handles).
    handles: Vec<Vector3>,
    // What the corners were last placed for: the placement revision and the view.
    placed: Option<(u64, Option<ChipView>)>,
    // Each chip's top-left corner, NaN behind the camera; empty without a camera. `pending`: the
    // HUD has not fetched them since they changed.
    corners: Vec<Vector2>,
    corners_pending: bool,
    // The chips whose panel the HUD must redraw at the last fetch (`deliver`).
    redraw: Vec<i32>,
    scratch: ChipScratch,
    text: String,
}

// One frame's chips. The texts are one string, each chip's ending at `text_ends[i]`.
#[derive(Default)]
struct ChipSet {
    labels: Vec<ToolLabel>,
    kinds: Vec<i32>,
    tones: Vec<i32>,
    text: String,
    text_ends: Vec<usize>,
    sizes: Vec<Vector2>,
}

impl ChipSet {
    fn clear(&mut self) {
        self.labels.clear();
        self.kinds.clear();
        self.tones.clear();
        self.text.clear();
        self.text_ends.clear();
        self.sizes.clear();
    }

    fn text_of(&self, index: usize) -> &str {
        let start = index
            .checked_sub(1)
            .map_or(0, |before| self.text_ends[before]);
        &self.text[start..self.text_ends[index]]
    }

    // What the HUD draws: kinds, tones, texts and sizes.
    fn same_content(&self, other: &Self) -> bool {
        self.kinds == other.kinds
            && self.tones == other.tones
            && self.text_ends == other.text_ends
            && self.text == other.text
            && self.sizes == other.sizes
    }

    // What placement reads beside the sizes: each chip's anchor and side.
    fn same_anchors(&self, other: &Self) -> bool {
        self.labels.len() == other.labels.len()
            && self
                .labels
                .iter()
                .zip(&other.labels)
                .all(|(a, b)| a.anchor == b.anchor && a.side == b.side)
    }

    // Copies the content (not the labels) from `other`, keeping this set's buffers.
    fn copy_content(&mut self, other: &Self) {
        self.kinds.clone_from(&other.kinds);
        self.tones.clone_from(&other.tones);
        self.text.clone_from(&other.text);
        self.text_ends.clone_from(&other.text_ends);
        self.sizes.clone_from(&other.sizes);
    }
}

// The view the chips are placed for: the camera's projection and transform, its near plane and
// the viewport's size. Equal views place equal corners.
#[derive(Clone, Copy, PartialEq)]
struct ChipView {
    projection: Projection,
    transform: Transform3D,
    near: f32,
    viewport: Vector2,
}

impl ChipView {
    fn of(camera: &Gd<Camera3D>) -> Option<Self> {
        let viewport = camera.get_viewport()?.get_visible_rect().size;
        Some(Self {
            projection: camera.get_camera_projection(),
            transform: camera.get_camera_transform(),
            near: camera.get_near(),
            viewport,
        })
    }

    // The projection times the camera transform's inverse, computed as Godot's own
    // `Projection * Projection(Transform3D.affine_inverse())` does (the HUD script's expression
    // before round R2-5), so the corners stay bit for bit what they were.
    fn view_projection(&self) -> Projection {
        let inverse = godot_affine_inverse(&self.transform);
        let rows = &inverse.basis.rows;
        let view = [
            [rows[0].x, rows[1].x, rows[2].x, 0.0],
            [rows[0].y, rows[1].y, rows[2].y, 0.0],
            [rows[0].z, rows[1].z, rows[2].z, 0.0],
            [inverse.origin.x, inverse.origin.y, inverse.origin.z, 1.0],
        ];
        let cols = self.projection.cols.map(|col| [col.x, col.y, col.z, col.w]);
        // Godot's `Projection::operator*`: column j, row i sums k = 0..4 in order from zero.
        let mut out = [Vector4::ZERO; 4];
        for (j, column) in out.iter_mut().enumerate() {
            let mut values = [0.0; 4];
            for (i, value) in values.iter_mut().enumerate() {
                let mut sum = 0.0;
                for (k, col) in cols.iter().enumerate() {
                    sum += col[i] * view[j][k];
                }
                *value = sum;
            }
            *column = Vector4::new(values[0], values[1], values[2], values[3]);
        }
        Projection::from_cols(out[0], out[1], out[2], out[3])
    }
}

// Godot's `Transform3D::affine_inverse` (core/math): the basis inverted by its cofactors and the
// origin carried through it, in the engine's order of operations.
fn godot_affine_inverse(transform: &Transform3D) -> Transform3D {
    let r = transform.basis.rows.map(|row| [row.x, row.y, row.z]);
    let cofac =
        |r1: usize, c1: usize, r2: usize, c2: usize| r[r1][c1] * r[r2][c2] - r[r1][c2] * r[r2][c1];
    let co = [cofac(1, 1, 2, 2), cofac(1, 2, 2, 0), cofac(1, 0, 2, 1)];
    let det = r[0][0] * co[0] + r[0][1] * co[1] + r[0][2] * co[2];
    let s = 1.0 / det;
    let basis = Basis::from_rows(
        Vector3::new(co[0] * s, cofac(0, 2, 2, 1) * s, cofac(0, 1, 1, 2) * s),
        Vector3::new(co[1] * s, cofac(0, 0, 2, 2) * s, cofac(0, 2, 1, 0) * s),
        Vector3::new(co[2] * s, cofac(0, 1, 2, 0) * s, cofac(0, 0, 1, 1) * s),
    );
    let o = -transform.origin;
    let dot = |row: Vector3| row.x * o.x + row.y * o.y + row.z * o.z;
    let origin = Vector3::new(dot(basis.rows[0]), dot(basis.rows[1]), dot(basis.rows[2]));
    Transform3D::new(basis, origin)
}

// The chips' font metrics as `road_tool_hud.gd` measures them (`road_tool_set_chip_font`).
#[derive(Default)]
struct ChipFont {
    // Advances in pixels by character: printable ASCII by code, the rest listed.
    ascii: Vec<f32>,
    other: Vec<(char, f32)>,
    // The panel's width beside the text (padding, icon and gap) and its height.
    extra_px: f32,
    height_px: f32,
}

impl ChipFont {
    // A text's chip size: the text's advances summed and rounded up, plus the panel.
    fn chip_size(&self, text: &str) -> Vector2 {
        let fallback = self.ascii.get(usize::from(b'0')).copied().unwrap_or(0.0);
        let width: f32 = text
            .chars()
            .map(|c| {
                self.ascii.get(c as usize).copied().unwrap_or_else(|| {
                    self.other
                        .iter()
                        .find(|(other, _)| *other == c)
                        .map_or(fallback, |&(_, advance)| advance)
                })
            })
            .sum();
        Vector2::new(width.ceil() + self.extra_px, self.height_px)
    }
}

#[derive(Default)]
struct ChipScratch {
    anchors: Vec<Vector2>,
    sides: Vec<Vector2>,
    sizes: Vec<Vector2>,
    obstacles: Vec<Vector2>,
    placed: Vec<Vector2>,
    corners: Vec<Vector2>,
    // Each chip is in front of the camera.
    shown: Vec<bool>,
}

// Godot's `Camera3D.unproject_position` with the camera's `view_projection` (projection times the
// inverse camera transform) and the viewport's size; `None` behind the near plane `near`.
fn unproject(
    view_projection: &Projection,
    viewport: Vector2,
    near: f32,
    point: Vector3,
) -> Option<Vector2> {
    let [x, y, z, w] = view_projection.cols;
    let clip = x * point.x + y * point.y + z * point.z + w;
    (clip.w >= near).then(|| {
        Vector2::new(
            (clip.x / clip.w * 0.5 + 0.5) * viewport.x,
            (-clip.y / clip.w * 0.5 + 0.5) * viewport.y,
        )
    })
}

fn kind_code(kind: LabelKind) -> i32 {
    match kind {
        LabelKind::PieceLength => 0,
        LabelKind::TotalLength => 1,
        LabelKind::Radius => 2,
        LabelKind::Angle => 3,
        LabelKind::Slope => 4,
        LabelKind::Cost => 5,
        LabelKind::Bearing => 6,
        LabelKind::DesignSpeed => 7,
        LabelKind::Notice => 8,
        LabelKind::Hint(_) => 9,
        LabelKind::Build => 10,
        LabelKind::Cancel => 11,
        LabelKind::EndHeight(_) => 12,
    }
}

fn tone_code(tone: LabelTone) -> i32 {
    match tone {
        LabelTone::Normal => 0,
        LabelTone::Warning => 1,
        LabelTone::Refused => 2,
    }
}

impl Chips {
    // Takes the frame's chips built in `next` (and the frame's handles): the revisions move when
    // the content or the placement's inputs differ from the last frame's. `O(chips + handles)`.
    fn commit_next(&mut self, handles: &[Vector3]) {
        let same_content = self.next.same_content(&self.shown);
        let same_place =
            same_content && self.next.same_anchors(&self.shown) && self.handles == handles;
        if !same_content {
            self.revision += 1;
        }
        if !same_place {
            self.placement += 1;
            self.handles.clear();
            self.handles.extend_from_slice(handles);
        }
        std::mem::swap(&mut self.next, &mut self.shown);
    }

    // Places this frame's chips for `view` unless they and the view are as last placed; returns
    // `road_tool_chips_update`'s flags. `O(chips (chips + handles) steps)` when it places
    // (`labels::place_chips`), `O(1)` otherwise; allocates nothing once the buffers have grown.
    fn update(&mut self, view: Option<ChipView>, obstacle_px: f32, gap: f32) -> i64 {
        if self.placed != Some((self.placement, view)) {
            self.placed = Some((self.placement, view));
            self.place(view, obstacle_px, gap);
            let scratch = &mut self.scratch;
            let same =
                scratch.corners.len() == self.corners.len()
                    && scratch.corners.iter().zip(&self.corners).all(|(a, b)| {
                        a.x.to_bits() == b.x.to_bits() && a.y.to_bits() == b.y.to_bits()
                    });
            if !same {
                std::mem::swap(&mut scratch.corners, &mut self.corners);
                self.corners_pending = true;
            }
        }
        if self.delivered_revision != Some(self.revision) {
            CHIPS_CHANGED
        } else if self.corners_pending {
            CORNERS_CHANGED
        } else {
            0
        }
    }

    // Each chip's anchor and side and the handles projected with `view`, then
    // `labels::place_chips`: the corners into `scratch.corners`, NaN for a chip behind the
    // camera, none without a view.
    fn place(&mut self, view: Option<ChipView>, obstacle_px: f32, gap: f32) {
        let scratch = &mut self.scratch;
        scratch.corners.clear();
        let Some(view) = view else { return };
        let view_projection = view.view_projection();
        let (viewport, near) = (view.viewport, view.near);
        scratch.anchors.clear();
        scratch.sides.clear();
        scratch.sizes.clear();
        scratch.shown.clear();
        for (label, &size) in self.shown.labels.iter().zip(&self.shown.sizes) {
            let at = unproject(&view_projection, viewport, near, label.anchor);
            scratch.shown.push(at.is_some());
            let Some(at) = at else { continue };
            let side = Vector3::new(label.side.x as f32, 0.0, label.side.y as f32);
            let toward = unproject(&view_projection, viewport, near, label.anchor + side * 4.0);
            scratch.anchors.push(at);
            scratch
                .sides
                .push(toward.map_or(Vector2::ZERO, |toward| toward - at));
            scratch.sizes.push(size);
        }
        scratch.obstacles.clear();
        for &handle in &self.handles {
            if let Some(at) = unproject(&view_projection, viewport, near, handle) {
                scratch.obstacles.push(at);
            }
        }
        place_chips(
            &scratch.anchors,
            &scratch.sides,
            &scratch.sizes,
            &scratch.obstacles,
            obstacle_px,
            gap,
            &mut scratch.placed,
        );
        let mut placed = scratch.placed.iter();
        let nan = Vector2::new(f32::NAN, f32::NAN);
        scratch.corners.extend(scratch.shown.iter().map(|&shown| {
            shown
                .then(|| placed.next().copied())
                .flatten()
                .unwrap_or(nan)
        }));
    }

    // The chips whose panel (kind, tone or size) differs from the HUD's copy or that it has not
    // had, into `redraw`; then the HUD's copy becomes this frame's. `O(chips)`.
    fn deliver(&mut self) {
        let (shown, delivered) = (&self.shown, &self.delivered);
        self.redraw.clear();
        self.redraw.extend(
            (0..shown.kinds.len())
                .filter(|&index| {
                    index >= delivered.kinds.len()
                        || shown.kinds[index] != delivered.kinds[index]
                        || shown.tones[index] != delivered.tones[index]
                        || shown.sizes[index] != delivered.sizes[index]
                })
                .map(|index| index as i32),
        );
        self.delivered.copy_content(&self.shown);
        self.delivered_revision = Some(self.revision);
        self.corners_pending = false;
    }
}

#[godot_api(secondary)]
impl SimulationNode {
    // The frame's label chips with what the standing preview says (the class-priced cost, the
    // steepest grade, a refusal or a warning; `pending` while the sent stroke waits): kinds,
    // tones, texts (round V2) and sizes in the HUD's font, built into the chips' reused buffers
    // and compared with the last frame's (`Chips::commit_next`). `O(chips + handles)`.
    pub(super) fn road_tool_set_labels(&mut self, pending: bool) {
        let frame = self.road_tool_session.session.frame();
        let chips = &mut self.road_tool_session.chips;
        let next = &mut chips.next;
        next.clear();
        next.labels.extend_from_slice(&frame.labels);
        let preview = (self.road_tool_session.preview.as_ref())
            .map(|standing| &standing.outcome)
            .filter(|_| frame.stroke.points.len() >= 2);
        let hud = frame.hud;
        let mut notice = NoticeFacts {
            refusal: frame.state.refusal.map_or("", refusal_key),
            draft: ReasonFacts {
                geometry_measured: Some(hud.radius_m.unwrap_or(-1.0)),
                geometry_limit: hud.illegal_radius_m,
                ..ReasonFacts::default()
            },
            ..NoticeFacts::default()
        };
        // Round R2-1: the standing preview's typed outcome, read in place.
        if let Some(preview) = preview {
            let middle = middle_label(&next.labels);
            apply_preview(&mut next.labels, preview.chip_facts(pending), middle);
            notice.preview_reason = &preview.reason;
            notice.preview = preview.reason_facts();
            notice.steep = preview.steep();
            notice.demolish_count = preview.demolish_count;
        }
        // Each marked end's height chip (round H2), after the frame's segmentation named its
        // part; placed after every other chip.
        push_height_labels(
            &frame.ground_marks,
            &frame.stroke.points,
            &frame.draft.runs,
            &mut next.labels,
        );
        // Each chip's code, text and its size in the HUD's font (round V2).
        let text = &mut chips.text;
        for label in &next.labels {
            next.kinds.push(kind_code(label.kind));
            next.tones.push(tone_code(label.tone));
            label_text(label, &notice, text);
            next.text.push_str(text);
            next.text_ends.push(next.text.len());
            next.sizes.push(chips.font.chip_size(text));
        }
        chips.commit_next(&frame.gizmos.handles);
    }

    /// Sets the HUD's chip font (round V2): `advances[i]` pixels for the `i`th character of
    /// `characters`, and the chip's panel: `extra_px` beside the text (padding, icon, gap) and
    /// `height_px`. The chips' sizes follow from these from the next `road_tool_frame`.
    #[func]
    pub fn road_tool_set_chip_font(
        &mut self,
        characters: GString,
        advances: PackedFloat32Array,
        extra_px: f32,
        height_px: f32,
    ) {
        let font = &mut self.road_tool_session.chips.font;
        font.ascii.clear();
        font.ascii.resize(128, 0.0);
        font.other.clear();
        for (c, &advance) in characters.to_string().chars().zip(advances.as_slice()) {
            match font.ascii.get_mut(c as usize) {
                Some(slot) => *slot = advance,
                None => font.other.push((c, advance)),
            }
        }
        font.extra_px = extra_px;
        font.height_px = height_px;
    }

    /// The characters the chips' texts use, for `road_tool_set_chip_font`: printable ASCII and
    /// the few others the wording writes.
    #[func]
    pub fn road_tool_chip_characters() -> GString {
        let mut characters: String = (b' '..=b'~').map(char::from).collect();
        characters.push_str(CHIP_EXTRA_CHARACTERS);
        characters.as_str().into()
    }

    /// Once a frame after `road_tool_frame` (round R2-5): places the last frame's chips for
    /// `camera` (its projection and transform, near plane and viewport; no camera: no corners)
    /// when the chips' anchors, sides, sizes, the handles or the view changed since they were
    /// last placed (`labels::place_chips`, `obstacle_px` squares about the handles, `gap`), and
    /// returns what the HUD must fetch: 1 the chips changed (`road_tool_chips`, corners
    /// included), 2 only their corners did (`road_tool_chip_corners`), 0 nothing (a still
    /// frame: the HUD draws nothing). Allocates nothing; `O(1)` on a frame that places nothing.
    #[func]
    pub fn road_tool_chips_update(
        &mut self,
        camera: Option<Gd<Camera3D>>,
        obstacle_px: f32,
        gap: f32,
    ) -> i64 {
        let view = camera.as_ref().and_then(ChipView::of);
        self.road_tool_session.chips.update(view, obstacle_px, gap)
    }

    /// The last frame's chips for the HUD to draw (round R2-5): `kinds` (0 piece length, 1 total
    /// length, 2 radius, 3 angle, 4 slope, 5 cost, 6 bearing, 7 design speed, 8 notice, 9 hint,
    /// 10 Build, 11 Cancel, 12 an end's height above its ground with its part), `tones` (0
    /// normal, 1 warning, 2 refused), `texts` (each chip's text as shown, from
    /// `road_tool::wording`), `sizes` (each chip's pixels in the HUD's font), `corners` (each
    /// chip's top-left corner, NaN behind the camera; as of the last `road_tool_chips_update`)
    /// and `redraw` (the chips whose kind, tone or size differs from the last fetch's, or that
    /// it did not have). Call it when `road_tool_chips_update` returns 1.
    #[func]
    pub fn road_tool_chips(&mut self) -> VarDictionary {
        let chips = &mut self.road_tool_session.chips;
        chips.deliver();
        let shown = &chips.shown;
        let texts: PackedStringArray = (0..shown.kinds.len())
            .map(|index| GString::from(shown.text_of(index)))
            .collect();
        vdict! {
            "kinds": PackedInt32Array::from(shown.kinds.as_slice()),
            "tones": PackedInt32Array::from(shown.tones.as_slice()),
            "texts": texts,
            "sizes": PackedVector2Array::from(shown.sizes.as_slice()),
            "corners": PackedVector2Array::from(chips.corners.as_slice()),
            "redraw": PackedInt32Array::from(chips.redraw.as_slice()),
        }
    }

    /// The chips' corners as of the last `road_tool_chips_update` (round R2-5), for the HUD when
    /// it returned 2: each chip's top-left corner, NaN behind the camera, empty without one.
    #[func]
    pub fn road_tool_chip_corners(&mut self) -> PackedVector2Array {
        let chips = &mut self.road_tool_session.chips;
        chips.corners_pending = false;
        PackedVector2Array::from(chips.corners.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::guide::DVec2;

    fn label(kind: LabelKind, anchor: Vector3) -> ToolLabel {
        ToolLabel {
            kind,
            value: 0.0,
            anchor,
            side: DVec2::new(1.0, 0.0),
            tone: LabelTone::Normal,
        }
    }

    // A frame's chips: one length chip at `anchor` reading `text`.
    fn frame(chips: &mut Chips, anchor: Vector3, text: &str) {
        let next = &mut chips.next;
        next.clear();
        next.labels.push(label(LabelKind::PieceLength, anchor));
        next.kinds.push(0);
        next.tones.push(0);
        next.text.push_str(text);
        next.text_ends.push(next.text.len());
        next.sizes.push(Vector2::new(60.0, 22.0));
        chips.commit_next(&[]);
    }

    fn view(x: f32) -> Option<ChipView> {
        Some(ChipView {
            projection: Projection::create_perspective(70.0, 16.0 / 9.0, 0.05, 4000.0, false),
            transform: Transform3D::new(Basis::IDENTITY, Vector3::new(x, 0.0, 100.0)),
            near: 0.05,
            viewport: Vector2::new(1920.0, 1080.0),
        })
    }

    // Round R2-5: a still frame (same chips, same view) asks the HUD for nothing; a new text
    // asks for the chips, a camera move or a moved anchor for the corners only.
    #[test]
    fn chips_change_gated() {
        let mut chips = Chips::default();
        let anchor = Vector3::new(0.0, 0.0, 0.0);
        frame(&mut chips, anchor, "12.0 m");
        assert_eq!(chips.update(view(0.0), 18.0, 6.0), CHIPS_CHANGED);
        chips.deliver();
        assert_eq!(chips.redraw, [0]);
        for _ in 0..3 {
            frame(&mut chips, anchor, "12.0 m");
            assert_eq!(
                chips.update(view(0.0), 18.0, 6.0),
                0,
                "a still frame fetches nothing"
            );
        }
        frame(&mut chips, anchor, "12.1 m");
        assert_eq!(chips.update(view(0.0), 18.0, 6.0), CHIPS_CHANGED);
        chips.deliver();
        assert!(chips.redraw.is_empty(), "a new text redraws no panel");
        assert_eq!(chips.update(view(3.0), 18.0, 6.0), CORNERS_CHANGED);
        chips.corners_pending = false;
        frame(&mut chips, Vector3::new(1.0, 0.0, 0.0), "12.1 m");
        assert_eq!(chips.update(view(3.0), 18.0, 6.0), CORNERS_CHANGED);
        chips.corners_pending = false;
        assert_eq!(chips.update(view(3.0), 18.0, 6.0), 0);
        assert_eq!(
            chips.update(None, 18.0, 6.0),
            CORNERS_CHANGED,
            "no camera: no corners"
        );
        assert!(chips.corners.is_empty());
    }

    // The view projection is Godot's expression evaluated in the engine's order: for a rotated,
    // moved camera it unprojects a point as the engine's matrix does to within rounding.
    #[test]
    fn view_projection_unprojects() {
        let basis = Basis::from_euler(EulerOrder::YXZ, Vector3::new(-0.7, 0.4, 0.0));
        let view = ChipView {
            transform: Transform3D::new(basis, Vector3::new(30.0, 120.0, 80.0)),
            ..view(0.0).unwrap()
        };
        let reference = view.projection * Projection::from(view.transform.affine_inverse());
        let point = Vector3::new(20.0, 0.0, 10.0);
        let ours = unproject(&view.view_projection(), view.viewport, view.near, point);
        let theirs = unproject(&reference, view.viewport, view.near, point);
        let (ours, theirs) = (ours.unwrap(), theirs.unwrap());
        assert!((ours - theirs).length() < 1e-2, "{ours:?} {theirs:?}");
    }
}
