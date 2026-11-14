// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's label chips (`ROAD-112` round V): small panels on the geometry, each with one
//! value, as the HUD. Rust decides which chips exist, their values, their text
//! ([`super::wording::label_text`], round V2), their world anchors and the side each is pushed
//! to; `road_tool_hud.gd` measures and draws the text, projects the anchors and places the chips
//! with [`place_chips`], the one placement rule (offset along the side, then step).
//!
//! The chips of a draft ([`push_draft_labels`], from the session's guide path and HUD numbers):
//! the plan length of each straight or Bezier piece at its middle, the radius at each arc's
//! middle, the total plan length at the path's middle when there is more than one piece (or the
//! only piece is an arc), the angle where the stroke meets a road at either end (180 degrees is
//! straight on out of a dead end; a road through the point reads its acute angle) or bends at a
//! freeform joint, and near the end the cost, the bearing and the design speed (amber below the
//! road's speed limit). The bridge adds what the preview knows ([`apply_preview`]): the class
//! priced cost, the steepest grade (from 0.5 %) and one notice for a refusal or a warning.
//! There is no upkeep chip (the user, 2026-11-14). Each end off its ground gets a height chip
//! (round H2, [`super::height::ground_marks::push_height_labels`]).

use super::measure::{EndRoad, HudValues};
use crate::simulation::network::guide::{DVec2, GuidePath, GuideSegment};
use godot::prelude::{Vector2, Vector3};

// A freeform joint bending more than this many degrees gets an angle chip.
const BEND_MIN_DEG: f64 = 0.5;
/// The steepest grade the slope chip shows, as a fraction (0.5 %).
pub const SLOPE_MIN_GRADE: f64 = 0.005;
// Rows a chip may step through before it stays at its last candidate.
const PLACEMENT_STEPS: usize = 64;

/// What a chip says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    /// A straight or Bezier piece's plan length in metres, at its middle.
    PieceLength,
    /// The whole draft's plan length in metres, at its middle.
    TotalLength,
    /// An arc's radius in metres, at its middle.
    Radius,
    /// The angle between the stroke and the road it meets, or at a bend, in whole degrees.
    Angle,
    /// The steepest grade of the previewed road, in percent.
    Slope,
    /// The build cost.
    Cost,
    /// The compass bearing of the stroke's end, in degrees clockwise from north.
    Bearing,
    /// The design speed of the tightest curve, in km/h.
    DesignSpeed,
    /// A warning or a refusal, worded from the draft's refusal or the preview
    /// ([`super::wording::label_text`]).
    Notice,
    /// How to go on with an open freeform draft (round V item 7): its text is [`hint_text`] of
    /// the value (0 placing, 1 editing).
    Hint,
    /// The edit phase's clickable Build chip beside the last node: builds the draft.
    Build,
    /// The edit phase's clickable Cancel chip beside the last node: drops the draft.
    Cancel,
    /// An end's signed height above the ground under it in metres, with the part the end is
    /// on (round H2, [`super::height::ground_marks`]).
    EndHeight(EndPart),
}

/// What an end of a draft is built as, for its height chip (round H2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndPart {
    /// A road on the ground (held or followed, within the clearance of the ground).
    Ground,
    /// A bridge.
    Bridge,
    /// A ground road held below the ground: a cut.
    Cut,
    /// A tunnel.
    Tunnel,
}

/// The hint chip's text for a freeform draft placing nodes (`editing` false) or in its edit
/// phase.
pub fn hint_text(editing: bool) -> &'static str {
    if editing {
        "Drag nodes or handles · Enter or click to build · Esc to cancel"
    } else {
        "Click to add a node · click the last node or Enter to finish"
    }
}

/// Pushes the chips of an open freeform draft whose last node is `last`, reached from `previous`
/// (the node before it, if any): the hint and, in the edit phase, the Build and Cancel chips,
/// pushed out beyond the last node. `O(1)`.
pub fn push_freeform_labels(
    last: Vector3,
    previous: Option<Vector3>,
    editing: bool,
    out: &mut Vec<ToolLabel>,
) {
    let along = previous
        .and_then(|previous| {
            DVec2::new(
                f64::from(last.x - previous.x),
                f64::from(last.z - previous.z),
            )
            .try_normalize()
        })
        .unwrap_or(DVec2::X);
    let label = |kind, value, side| ToolLabel {
        kind,
        value,
        anchor: last,
        side,
        tone: LabelTone::Normal,
    };
    if editing {
        out.push(label(LabelKind::Build, 0.0, along));
        out.push(label(LabelKind::Cancel, 0.0, along));
    }
    out.push(label(
        LabelKind::Hint,
        f64::from(u8::from(editing)),
        along.perp(),
    ));
}

/// How a chip is coloured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LabelTone {
    /// Plain information.
    #[default]
    Normal,
    /// Advice: amber.
    Warning,
    /// A refusal: red.
    Refused,
}

/// One chip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolLabel {
    /// What it says.
    pub kind: LabelKind,
    /// The value (metres, degrees, percent, money, km/h; 0 for a notice).
    pub value: f64,
    /// The world point it is anchored to.
    pub anchor: Vector3,
    /// The unit XZ direction it is pushed to from the anchor.
    pub side: DVec2,
    /// Its colour.
    pub tone: LabelTone,
}

/// What the preview of the draft's stroke says, for [`apply_preview`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PreviewFacts {
    /// The preview refused the stroke.
    pub refused: bool,
    /// The stroke was sent and waits for its commit.
    pub pending: bool,
    /// The preview's class-priced build cost.
    pub build_cost: Option<f64>,
    /// The steepest grade of the planned road, as a fraction.
    pub max_grade: Option<f64>,
    /// The preview warns (a steep grade, demolitions, a plan still being checked).
    pub warning: bool,
}

/// Appends the chips of a draft: `path` is its guide, `stroke` its stroke request's points,
/// `hud` its numbers, `start_road` and `end_road` the roads met at its ends, `refused` whether
/// the session refuses it (a notice chip at the end). `O(pieces)`, no allocation beyond growing
/// `out`.
pub fn push_draft_labels(
    path: &GuidePath,
    stroke: &[Vector3],
    hud: &HudValues,
    start_road: Option<EndRoad>,
    end_road: Option<EndRoad>,
    refused: bool,
    out: &mut Vec<ToolLabel>,
) {
    let (Some(first), Some(last)) = (stroke.first(), stroke.last()) else {
        return;
    };
    let segments = path.segments();
    let total: f64 = segments.iter().map(GuideSegment::length).sum();
    if segments.is_empty() || total <= 0.0 {
        return;
    }
    // Chips float over the ground: heights by plan station between the stroke's ends.
    let at = |station: f64, xz: DVec2| {
        let y = first.y + (last.y - first.y) * (station / total) as f32;
        Vector3::new(xz.x as f32, y, xz.y as f32)
    };
    let label = |kind, value, anchor, side| ToolLabel {
        kind,
        value,
        anchor,
        side,
        tone: LabelTone::Normal,
    };
    let end_xz = DVec2::new(f64::from(last.x), f64::from(last.z));
    let end_tangent = path.end_tangent().unwrap_or(DVec2::X);
    // The end's chips sit to the right of travel, the pieces' to the left.
    let end_side = -end_tangent.perp();
    if refused {
        out.push(ToolLabel {
            tone: LabelTone::Refused,
            ..label(LabelKind::Notice, 0.0, *last, end_side)
        });
    } else {
        out.push(label(LabelKind::Cost, hud.build_cost, *last, end_side));
    }
    out.push(label(LabelKind::Bearing, hud.bearing_deg, *last, end_side));
    if let Some(speed) = hud.design_speed_kmh {
        let tone = if hud.design_speed_warning {
            LabelTone::Warning
        } else {
            LabelTone::Normal
        };
        out.push(ToolLabel {
            tone,
            ..label(LabelKind::DesignSpeed, speed, *last, end_side)
        });
    }
    let mut station = 0.0;
    for (index, segment) in segments.iter().enumerate() {
        let length = segment.length();
        let middle = length * 0.5;
        let anchor = at(station + middle, segment.point_at(middle));
        let left = segment.tangent_at(middle).perp();
        match segment {
            GuideSegment::Arc(arc) => {
                // Outside the curve, away from its centre.
                let outward = (segment.point_at(middle) - arc.centre())
                    .try_normalize()
                    .unwrap_or(left);
                out.push(label(LabelKind::Radius, arc.radius(), anchor, outward));
            }
            _ => out.push(label(LabelKind::PieceLength, length, anchor, left)),
        }
        if index > 0 {
            let incoming = segments[index - 1].end_tangent();
            let deflection = deflection_deg(incoming, segment.start_tangent());
            if deflection > BEND_MIN_DEG {
                let joint = segment.start();
                // On the outside of the bend.
                let outside = (incoming - segment.start_tangent())
                    .try_normalize()
                    .unwrap_or(left);
                out.push(label(
                    LabelKind::Angle,
                    (180.0 - deflection).round(),
                    at(station, joint),
                    outside,
                ));
            }
        }
        station += length;
    }
    if segments.len() > 1 || matches!(segments, [GuideSegment::Arc(_)]) {
        let (segment, s) = segment_at(segments, total * 0.5);
        out.push(label(
            LabelKind::TotalLength,
            total,
            at(total * 0.5, segment.point_at(s)),
            segment.tangent_at(s).perp(),
        ));
    }
    let start_tangent = path.start_tangent().unwrap_or(DVec2::X);
    if let Some(road) = start_road {
        // Behind the start: the draft leaves forward.
        out.push(label(
            LabelKind::Angle,
            road_angle_deg(start_tangent, road),
            *first,
            -start_tangent,
        ));
    }
    if let Some(road) = end_road {
        // Arriving at a dead end straight on runs against its outward direction.
        out.push(label(
            LabelKind::Angle,
            road_angle_deg(-end_tangent, road),
            at(total, end_xz),
            end_tangent,
        ));
    }
}

/// Adds what the preview of the draft's stroke says to its chips: the class-priced cost in
/// place of the session's, the steepest grade at the path's middle from [`SLOPE_MIN_GRADE`],
/// and one notice near the end (red for a refusal, which also drops the cost; amber for a
/// warning or a stroke waiting for its commit). `O(labels)`.
pub fn apply_preview(labels: &mut Vec<ToolLabel>, facts: PreviewFacts, middle: Option<ToolLabel>) {
    let Some(cost) = labels
        .iter()
        .position(|label| label.kind == LabelKind::Cost)
    else {
        return;
    };
    let end = labels[cost];
    if facts.refused {
        labels[cost] = ToolLabel {
            kind: LabelKind::Notice,
            value: 0.0,
            tone: LabelTone::Refused,
            ..end
        };
        return;
    }
    if let Some(build_cost) = facts.build_cost {
        labels[cost].value = build_cost;
    }
    if facts.pending || facts.warning {
        labels.insert(
            cost,
            ToolLabel {
                kind: LabelKind::Notice,
                value: 0.0,
                tone: LabelTone::Warning,
                ..end
            },
        );
    }
    if let (Some(grade), Some(middle)) = (facts.max_grade, middle)
        && grade.abs() >= SLOPE_MIN_GRADE
    {
        labels.push(ToolLabel {
            kind: LabelKind::Slope,
            value: grade * 100.0,
            side: -middle.side,
            ..middle
        });
    }
}

/// The chip the slope chip shares an anchor with: the total length's, else the first piece's.
pub fn middle_label(labels: &[ToolLabel]) -> Option<ToolLabel> {
    labels
        .iter()
        .find(|label| label.kind == LabelKind::TotalLength)
        .or_else(|| {
            labels
                .iter()
                .find(|label| matches!(label.kind, LabelKind::PieceLength | LabelKind::Radius))
        })
        .copied()
}

/// Places chips on the screen without overlap, in order: chip `i` of size `sizes[i]` (pixels)
/// goes `gap` pixels out from its anchor `anchors[i]` along the unit screen direction `sides[i]`
/// (its edge nearest the anchor that far); while it overlaps a chip placed before it or an
/// obstacle (squares of `obstacle_px` about each of `obstacles`: the handles), a `gap` apart, it
/// steps across the screen by its own height plus `gap`, alternately down and up (1, -1, 2, -2,
/// ...), so chips sharing an anchor stack in a column beside it; after [`PLACEMENT_STEPS`] steps
/// it keeps its last candidate. Writes each chip's top-left corner into `out` (cleared first).
/// Deterministic: the same input gives the same output bit for bit. `O(n (n + h) steps)` for `n`
/// chips (a draft has about a dozen) and `h` obstacles.
pub fn place_chips(
    anchors: &[Vector2],
    sides: &[Vector2],
    sizes: &[Vector2],
    obstacles: &[Vector2],
    obstacle_px: f32,
    gap: f32,
    out: &mut Vec<Vector2>,
) {
    out.clear();
    let count = anchors.len().min(sides.len()).min(sizes.len());
    let obstacle = Vector2::splat(obstacle_px);
    for index in 0..count {
        let size = sizes[index];
        let side = sides[index]
            .try_normalized()
            .unwrap_or(Vector2::new(0.0, -1.0));
        // The chip's half extent along its side: its box's support distance.
        let reach = (side.x.abs() * size.x + side.y.abs() * size.y) * 0.5;
        let base = anchors[index] + side * (gap + reach) - size * 0.5;
        let mut corner = base;
        for step in 0..PLACEMENT_STEPS {
            // 0, 1, -1, 2, -2, ... rows of the chip's height.
            let row = step.div_ceil(2) as f32 * if step % 2 == 0 { -1.0 } else { 1.0 };
            corner = base + Vector2::new(0.0, row * (size.y + gap));
            let blocked = overlaps_any(corner, size, &out[..index], &sizes[..index], gap)
                || obstacles.iter().any(|&centre| {
                    overlaps_any(corner, size, &[centre - obstacle * 0.5], &[obstacle], gap)
                });
            if !blocked {
                break;
            }
        }
        out.push(corner);
    }
}

fn overlaps_any(
    corner: Vector2,
    size: Vector2,
    placed: &[Vector2],
    sizes: &[Vector2],
    gap: f32,
) -> bool {
    placed.iter().zip(sizes).any(|(other, other_size)| {
        corner.x < other.x + other_size.x + gap
            && other.x < corner.x + size.x + gap
            && corner.y < other.y + other_size.y + gap
            && other.y < corner.y + size.y + gap
    })
}

// The piece holding plan station `s` and the station within it.
fn segment_at(segments: &[GuideSegment], mut s: f64) -> (&GuideSegment, f64) {
    for segment in segments {
        let length = segment.length();
        if s <= length {
            return (segment, s);
        }
        s -= length;
    }
    let last = &segments[segments.len() - 1];
    (last, last.length())
}

// Degrees between two unit directions, `0..=180`.
fn deflection_deg(a: DVec2, b: DVec2) -> f64 {
    a.perp_dot(b).abs().atan2(a.dot(b)).to_degrees()
}

/// The angle a chip shows where the draft, leaving the joint along `leaving`, meets `road`:
/// out of a dead end the angle between the two roads (180 straight on), against a road through
/// the point the acute angle (`0..=90`), whole degrees.
pub fn road_angle_deg(leaving: DVec2, road: EndRoad) -> f64 {
    match road {
        EndRoad::Continue(direction) => (180.0 - deflection_deg(leaving, direction)).round(),
        EndRoad::Along(direction) => {
            let angle = deflection_deg(leaving, direction);
            angle.min(180.0 - angle).round()
        }
    }
}
