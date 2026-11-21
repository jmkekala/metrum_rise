// SPDX-License-Identifier: GPL-3.0-or-later

//! The agents' shadow-only twins against the recorder: only the agents within reach cast, each
//! twin with its bucket's mesh, material and margin; an unchanged feed and view send nothing;
//! the kind toggles hide and show.

use super::super::super::backend::Scenario;
use super::super::super::backend::recorder::Recorder;
use super::*;
use crate::nodes::sim::render::agent_frame::CAR_STRIDE;

fn car(x: f32, z: f32) -> [f32; CAR_STRIDE] {
    [1.0, 0.0, 0.0, x, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, z]
}

fn walker(x: f32, z: f32, phase: f32) -> [f32; WALKER_STRIDE] {
    let mut row = [0.0; WALKER_STRIDE];
    row[..CAR_STRIDE].copy_from_slice(&car(x, z));
    row[12] = phase;
    row
}

// A camera 10 m up at the origin looking down +X.
const VIEW: CasterView = CasterView {
    eye: [0.0, 10.0, 0.0],
    forward: [1.0, 0.0, 0.0],
    range: 420.0,
};

fn feed() -> CasterFeed {
    let mut feed = CasterFeed::default();
    // Cars at 100 m ahead (kept), 600 m ahead (beyond the depth reach), 100 m behind (kept: no,
    // depth -100 < -30), 20 m behind (kept: within the margin).
    feed.candidates.cars = vec![Vec::new(); 11];
    for x in [100.0, 600.0, -100.0, -20.0] {
        feed.candidates.cars[10].extend(car(x, 5.0));
        feed.candidates.cars[10].extend([0.0; 4]);
    }
    feed.candidates.walkers = vec![walker(50.0, 0.0, 0.25).to_vec()];
    feed.serial = 1;
    feed
}

fn layer() -> AgentCasterLayer {
    let mut layer = AgentCasterLayer::new(SLOT_LIMIT, SPARE_LIMIT);
    layer.register(AgentKind::Cars, 10, 3, None, 12.0);
    layer.register(AgentKind::Walkers, 0, 4, Some(7), 12.0);
    layer
}

// A visible instance: mesh, shadow, material, margin, its rows.
type Twin = (u32, Shadow, Option<u32>, f32, Vec<f32>);

// Every visible instance.
fn drawn(backend: &Recorder) -> Vec<Twin> {
    let mut out: Vec<_> = backend
        .visible(Scenario::Main)
        .map(|(_, instance)| {
            let multimesh = &backend.multimeshes[&instance.base];
            // Cars and walkers alike: a transform, then custom data (`RENDER-65`).
            let stride = WALKER_STRIDE;
            let count = multimesh.visible_count.unwrap() as usize;
            (
                multimesh.mesh.unwrap(),
                instance.shadow,
                instance.material,
                instance.extra_margin,
                multimesh.rows[..count * stride].to_vec(),
            )
        })
        .collect();
    out.sort_by_key(|twin| twin.0);
    out
}

#[test]
fn twins_cast_only_the_agents_within_reach_as_their_buckets_did() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    let feed = feed();
    assert!(layer.update(&mut backend, &feed, VIEW));
    let kept_cars: Vec<f32> = [&car(100.0, 5.0)[..], &[0.0; 4], &car(-20.0, 5.0), &[0.0; 4]].concat();
    assert_eq!(
        drawn(&backend),
        vec![
            (3, Shadow::ShadowsOnly, None, 12.0, kept_cars),
            (
                4,
                Shadow::ShadowsOnly,
                Some(7),
                12.0,
                walker(50.0, 0.0, 0.25).to_vec()
            ),
        ]
    );

    // The same feed and view: nothing tested, nothing sent.
    let sent = backend.commands.len();
    assert!(!layer.update(&mut backend, &feed, VIEW));
    assert_eq!(backend.commands.len(), sent);

    // The camera turns round: the cars behind it now lie ahead, the rest behind.
    let turned = CasterView {
        forward: [-1.0, 0.0, 0.0],
        ..VIEW
    };
    assert!(layer.update(&mut backend, &feed, turned));
    let cars: Vec<_> = layer
        .drawn()
        .filter(|(kind, ..)| *kind == AgentKind::Cars)
        .map(|(_, _, rows)| rows.to_vec())
        .collect();
    assert_eq!(cars, vec![[&car(-100.0, 5.0)[..], &[0.0; 4], &car(-20.0, 5.0), &[0.0; 4]].concat()]);
    // The walker 50 m behind the camera leaves.
    assert!(layer.drawn().all(|(kind, ..)| kind == AgentKind::Cars));
}

#[test]
fn kind_toggles_and_the_twins_switch_hide_and_show() {
    let mut backend = Recorder::new();
    let mut layer = layer();
    layer.update(&mut backend, &feed(), VIEW);
    assert_eq!(drawn(&backend).len(), 2);
    layer.set_cast(&mut backend, AgentKind::Cars, false);
    assert_eq!(drawn(&backend).len(), 1);
    layer.set_enabled(&mut backend, false);
    assert!(drawn(&backend).is_empty());
    assert!(!layer.is_active());
    layer.set_enabled(&mut backend, true);
    layer.set_cast(&mut backend, AgentKind::Cars, true);
    assert_eq!(drawn(&backend).len(), 2);
    // No range (the sun casts nothing): every twin empties and hides.
    layer.update(&mut backend, &feed(), CasterView { range: 0.0, ..VIEW });
    assert!(drawn(&backend).is_empty());
    layer.reset(&mut backend);
    assert_eq!(layer.pool().live(), 0);
}
